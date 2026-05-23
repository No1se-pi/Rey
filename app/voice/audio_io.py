from __future__ import annotations

import importlib.util
import logging
from pathlib import Path

from app.voice.errors import VoiceDependencyError


logger = logging.getLogger(__name__)


def has_audio_recording_dependencies() -> tuple[bool, list[str]]:
    missing = [
        package
        for package in ("sounddevice", "scipy")
        if importlib.util.find_spec(package) is None
    ]
    return not missing, missing


def record_wav(
    output_path: Path,
    *,
    duration_seconds: float = 5.0,
    sample_rate: int = 16000,
    channels: int = 1,
) -> Path:
    available, missing = has_audio_recording_dependencies()
    if not available:
        raise VoiceDependencyError("Не хватает пакетов для записи с микрофона: " + ", ".join(missing))

    import sounddevice as sd
    from scipy.io.wavfile import write

    logger.info("STT recording started duration=%s sample_rate=%s", duration_seconds, sample_rate)
    audio = sd.rec(
        int(duration_seconds * sample_rate),
        samplerate=sample_rate,
        channels=channels,
        dtype="float32",
    )
    sd.wait()
    write(output_path, sample_rate, audio)
    logger.info("STT recording finished path=%s", output_path)
    return output_path


def play_wav(path: Path) -> None:
    available, missing = has_audio_recording_dependencies()
    if not available:
        raise VoiceDependencyError("Не хватает пакетов для проигрывания wav: " + ", ".join(missing))

    import sounddevice as sd
    from scipy.io.wavfile import read

    sample_rate, audio = read(path)
    sd.play(audio, sample_rate)
    sd.wait()

