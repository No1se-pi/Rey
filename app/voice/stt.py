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
        self._forced_device: str | None = None
        self._forced_compute_type: str | None = None

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
            try:
                text = self._transcribe_file(wav_path)
            except RuntimeError as exc:
                if not self._should_retry_on_cpu(exc):
                    raise
                logger.warning("STT GPU runtime failed; retrying with CPU int8. error=%s", exc)
                self._model = None
                self._forced_device = "cpu"
                self._forced_compute_type = "int8"
                text = self._transcribe_file(wav_path)
            logger.info("STT finished text_length=%s", len(text))
            return text
        finally:
            try:
                wav_path.unlink(missing_ok=True)
            except OSError:
                logger.warning("Failed to remove temporary wav file: %s", wav_path)

    def _transcribe_file(self, wav_path: Path) -> str:
        model = self._get_model()
        segments, _info = model.transcribe(str(wav_path), language="ru", vad_filter=True)
        return " ".join(segment.text.strip() for segment in segments if segment.text.strip()).strip()

    def _get_model(self):
        if self._model is not None:
            return self._model

        from faster_whisper import WhisperModel

        device = self._forced_device or self.settings.stt_device
        compute_type = self._forced_compute_type or self.settings.stt_compute_type
        if compute_type == "auto":
            compute_type = "int8" if device == "cpu" else "default"
        logger.info("Loading STT model size=%s device=%s compute_type=%s", self.settings.stt_model_size, device, compute_type)
        self._model = WhisperModel(
            self.settings.stt_model_size,
            device=device,
            compute_type=compute_type,
        )
        return self._model

    def _should_retry_on_cpu(self, exc: RuntimeError) -> bool:
        if self._forced_device == "cpu":
            return False
        message = str(exc).lower()
        cuda_markers = ("cuda", "cublas", "cudnn", "cufft", "cublas64")
        return any(marker in message for marker in cuda_markers)
