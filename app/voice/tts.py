from __future__ import annotations

import importlib.util
import logging

from app.config import Settings, get_settings
from app.voice.errors import VoiceDependencyError


logger = logging.getLogger(__name__)


class TextToSpeech:
    def __init__(self, settings: Settings | None = None) -> None:
        self.settings = settings or get_settings()
        self._engine = None

    def check_availability(self) -> tuple[bool, str]:
        if self.settings.tts_engine != "pyttsx3":
            return False, f"TTS_ENGINE={self.settings.tts_engine} пока не поддержан в MVP. Доступен pyttsx3."
        if importlib.util.find_spec("pyttsx3") is None:
            return False, "Для озвучки поставь пакет: pyttsx3"
        return True, "TTS доступен."

    def speak(self, text: str) -> None:
        available, message = self.check_availability()
        if not available:
            raise VoiceDependencyError(message)
        if not text.strip():
            return

        logger.info("TTS started text_length=%s", len(text))
        engine = self._get_engine()
        engine.say(text)
        engine.runAndWait()
        logger.info("TTS finished")

    def _get_engine(self):
        if self._engine is not None:
            return self._engine

        import pyttsx3

        engine = pyttsx3.init()
        engine.setProperty("rate", self.settings.tts_rate)
        engine.setProperty("volume", self.settings.tts_volume)
        if self.settings.tts_voice_name:
            self._select_voice(engine, self.settings.tts_voice_name)
        self._engine = engine
        return self._engine

    def _select_voice(self, engine, voice_name: str) -> None:
        desired = voice_name.lower()
        for voice in engine.getProperty("voices"):
            voice_id = str(getattr(voice, "id", "")).lower()
            name = str(getattr(voice, "name", "")).lower()
            if desired in voice_id or desired in name:
                engine.setProperty("voice", voice.id)
                return
        logger.warning("Requested TTS voice not found: %s", voice_name)

