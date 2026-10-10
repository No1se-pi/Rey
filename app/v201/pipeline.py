"""Transport-neutral conversation pipeline shared by CLI and Telegram."""
from __future__ import annotations

import logging
import threading
from dataclasses import dataclass
from pathlib import Path
from time import perf_counter
from uuid import uuid4

from app.chat.chat_loop import ChatService
from app.chat.session import ChatSession
from app.config import get_settings

from .audio import AudioOutputs, OfflineSTT, QualityTTS, encode_audio
from .settings import VoiceSettings

log = logging.getLogger(__name__)


@dataclass(frozen=True)
class ReyReply:
    text: str
    audio: AudioOutputs | None
    recognized_text: str | None = None


class ReyPipeline:
    def __init__(self, settings: VoiceSettings | None = None, *, service_factory=None, tts=None, stt=None):
        self.settings = settings or VoiceSettings.from_env()
        self._sessions: dict[str, ChatService] = {}
        self._factory = service_factory or (lambda session_id: ChatService(session=ChatSession(session_id=session_id)))
        self.tts = tts or QualityTTS(self.settings)
        self.stt = stt or OfflineSTT(self.settings)
        self._inference_lock = threading.Lock()

    def service_for(self, session_key: str) -> ChatService:
        if session_key not in self._sessions:
            self._sessions[session_key] = self._factory(session_key)
        return self._sessions[session_key]

    def transcribe(self, file: Path) -> str:
        with self._inference_lock:
            text = self.stt.transcribe(file)
        if not text:
            raise ValueError("No intelligible speech detected")
        return text

    def reply(
        self,
        text: str,
        session_key: str,
        output_dir: Path,
        *,
        recognized_text: str | None = None,
        synthesize_audio: bool = True,
    ) -> ReyReply:
        text = text.strip()
        if not text or len(text) > 4000:
            raise ValueError("Message must be 1-4000 characters")
        with self._inference_lock:
            service = self.service_for(session_key)
            started = perf_counter()
            result = service.handle_user_text(text, voice_mode=synthesize_audio)
            log.info("LLM/dialogue finished seconds=%.3f", perf_counter() - started)
            answer = result.text
            if not answer.strip():
                raise ValueError("LLM returned an empty response")
            if not synthesize_audio:
                return ReyReply(text=answer, audio=None, recognized_text=recognized_text)

            if self.settings.ollama_unload_before_tts:
                self._unload_ollama()
            spoken = answer[: self.settings.max_reply_chars]
            if len(answer) > self.settings.max_reply_chars:
                log.warning("Speech shortened from %s to %s characters", len(answer), len(spoken))
                spoken = spoken.rsplit(" ", 1)[0] or spoken
            output_dir.mkdir(parents=True, exist_ok=True)
            request_id = uuid4().hex[:12]
            wav = self.tts.render(spoken, output_dir / f"rey_reply_{request_id}.wav")
            audio = encode_audio(
                wav,
                output_dir,
                stem=f"rey_reply_{request_id}",
                timeout=self.settings.ffmpeg_timeout_seconds,
            )
            return ReyReply(text=answer, audio=audio, recognized_text=recognized_text)

    @staticmethod
    def _unload_ollama() -> None:
        import requests

        config = get_settings()
        started = perf_counter()
        try:
            requests.post(
                f"{config.ollama_host.rstrip('/')}/api/generate",
                json={"model": config.ollama_model, "keep_alive": 0},
                timeout=15,
            ).raise_for_status()
            log.info("Ollama unload request finished seconds=%.3f", perf_counter() - started)
        except requests.RequestException as exc:
            log.warning("Could not unload Ollama model before TTS: %s", exc)
