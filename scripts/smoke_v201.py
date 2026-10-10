"""Opt-in local smoke checks for Rey 2.0.1; never contacts Telegram."""
from __future__ import annotations

import argparse
import importlib.util
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from app.config import get_settings
from app.llm.ollama_client import OllamaClient
from app.v201.audio import QualityTTS, encode_audio, probe_audio
from app.v201.settings import VoiceSettings
from app.voice.audio_io import record_wav


def main() -> int:
    parser = argparse.ArgumentParser(description="Safe Rey 2.0.1 local smoke checks")
    parser.add_argument("--microphone", action="store_true", help="record 0.5 seconds from the default input")
    parser.add_argument("--ollama", action="store_true", help="run one short prompt against configured Ollama")
    parser.add_argument("--audio", type=Path, help="transcribe an existing audio file (may download the configured STT model)")
    parser.add_argument("--tts", action="store_true", help="synthesize a short phrase (requires installed/cached Qwen3-TTS)")
    args = parser.parse_args()

    voice = VoiceSettings.from_env()
    print("Python:", sys.version.split()[0])
    print("Model cache:", voice.model_cache_dir)
    for module in ("telethon", "faster_whisper", "sounddevice", "torch", "qwen_tts"):
        print(f"{module}:", "installed" if importlib.util.find_spec(module) else "missing")
    try:
        import ctranslate2

        print("CTranslate2 CUDA devices:", ctranslate2.get_cuda_device_count())
    except ImportError:
        print("CTranslate2: missing")

    with tempfile.TemporaryDirectory(prefix="rey-smoke-", dir=ROOT) as folder:
        temporary = Path(folder)
        if args.microphone:
            recording = record_wav(temporary / "microphone.wav", duration_seconds=0.5)
            print("Microphone:", probe_audio(recording))
        if args.audio:
            from app.v201.audio import OfflineSTT

            print("STT:", OfflineSTT(voice).transcribe(args.audio))
        if args.ollama:
            reply = OllamaClient(settings=get_settings()).chat([{"role": "user", "content": "Ответь одним словом: связь?"}])
            print("Ollama reply chars:", len(reply))
        if args.tts:
            wav = QualityTTS(voice).render("Привет! Это локальная проверка голоса Рей.", temporary / "tts.wav")
            output = encode_audio(wav, temporary, timeout=voice.ffmpeg_timeout_seconds)
            print("TTS WAV:", probe_audio(output.wav))
            print("TTS MP3:", probe_audio(output.mp3))
            print("TTS OGG/Opus:", probe_audio(output.opus))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
