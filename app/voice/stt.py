from __future__ import annotations

import importlib.util
import logging
import tempfile
from pathlib import Path

from app.config import Settings, get_settings
from app.voice.audio_io import has_audio_recording_dependencies, record_wav
from app.voice.errors import VoiceDependencyError


logger = logging.getLogger(__name__)


class SpeechToText:
    def __init__(self, settings: Settings | None = None) -> None:
        self.settings = settings or get_settings()
        self._model = None

    def check_availability(self) -> tuple[bool, str]:
        missing: list[str] = []
        if importlib.util.find_spec("faster_whisper") is None:
            missing.append("faster-whisper")
        audio_available, audio_missing = has_audio_recording_dependencies()
        if not audio_available:
            missing.extend(audio_missing)
        if missing:
            return False, "Для voice mode поставь пакеты: " + " ".join(sorted(set(missing)))
        return True, "STT доступен."

    def transcribe_from_mic(self, *, duration_seconds: float = 5.0) -> str:
        available, message = self.check_availability()
        if not available:
            raise VoiceDependencyError(message)

        logger.info("STT started")
        with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as temp_file:
            wav_path = Path(temp_file.name)

        try:
            record_wav(wav_path, duration_seconds=duration_seconds)
            model = self._get_model()
            segments, _info = model.transcribe(str(wav_path), language="ru", vad_filter=True)
            text = " ".join(segment.text.strip() for segment in segments if segment.text.strip()).strip()
            logger.info("STT finished text_length=%s", len(text))
            return text
        finally:
            try:
                wav_path.unlink(missing_ok=True)
            except OSError:
                logger.warning("Failed to remove temporary wav file: %s", wav_path)

    def _get_model(self):
        if self._model is not None:
            return self._model

        from faster_whisper import WhisperModel

        compute_type = self.settings.stt_compute_type
        if compute_type == "auto":
            compute_type = "default"
        self._model = WhisperModel(
            self.settings.stt_model_size,
            device=self.settings.stt_device,
            compute_type=compute_type,
        )
        return self._model

