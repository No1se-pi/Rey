"""Validated configuration for Rey 2.0.1."""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from app.config import PROJECT_ROOT


def env_bool(key: str, default: bool = False) -> bool:
    return os.getenv(key, str(default)).strip().lower() in {"1", "true", "yes", "on"}


def _positive_int(key: str, default: int) -> int:
    raw = os.getenv(key, str(default)).strip()
    try:
        value = int(raw)
    except ValueError as exc:
        raise ValueError(f"{key} must be an integer") from exc
    if value <= 0:
        raise ValueError(f"{key} must be greater than zero")
    return value


def _path_from_env(key: str, default: Path) -> Path:
    value = Path(os.getenv(key, str(default)).strip()).expanduser()
    return value if value.is_absolute() else PROJECT_ROOT / value


def _numeric_ids(key: str) -> frozenset[int]:
    values = os.getenv(key, "")
    try:
        return frozenset(int(item.strip()) for item in values.split(",") if item.strip())
    except ValueError as exc:
        raise ValueError(f"{key} must contain comma-separated numeric IDs") from exc


@dataclass(frozen=True)
class VoiceSettings:
    output_dir: Path
    model_cache_dir: Path
    legacy_transformers_cache_dir: Path | None
    keep_output_audio: bool
    tts_mode: str
    tts_model: str
    tts_device: str
    tts_instruction: str
    reference_audio: Path | None
    reference_text: str
    tts_unload_after_synthesis: bool
    tts_cpu_fallback: bool
    tts_allow_model_download: bool
    tts_chunk_chars: int
    ollama_unload_before_tts: bool
    stt_model: str
    stt_device: str
    stt_cpu_fallback: bool
    stt_unload_after_transcription: bool
    max_audio_seconds: int
    max_audio_bytes: int
    max_reply_chars: int
    ffmpeg_timeout_seconds: int
    cli_voice_enabled: bool

    @classmethod
    def from_env(cls) -> VoiceSettings:
        from app.config import _load_env

        _load_env()
        mode = os.getenv("REY_TTS_MODE", "design").lower().strip()
        if mode not in {"design", "clone"}:
            raise ValueError("REY_TTS_MODE must be design or clone")
        device = os.getenv("REY_TTS_DEVICE", "auto").lower().strip()
        if device not in {"auto", "cpu", "cuda", "cuda:0"}:
            raise ValueError("REY_TTS_DEVICE must be auto, cpu, cuda, or cuda:0")
        stt_device = os.getenv("REY_STT_DEVICE", "cpu").lower().strip()
        if stt_device not in {"auto", "cpu", "cuda"}:
            raise ValueError("REY_STT_DEVICE must be auto, cpu, or cuda")
        ref = os.getenv("REY_TTS_REFERENCE_AUDIO", "").strip()
        reference_audio = _path_from_env("REY_TTS_REFERENCE_AUDIO", PROJECT_ROOT / ref) if ref else None
        configured_cache = _path_from_env("REY_MODEL_CACHE_DIR", PROJECT_ROOT / "data" / "model_cache")
        # Reuse an existing multi-GB Hugging Face cache when HF_HOME is set.
        # The deprecated TRANSFORMERS_CACHE can split nested Qwen files across
        # two trees, leaving an otherwise complete model impossible to load.
        model_cache = Path(os.environ.get("HF_HOME", str(configured_cache))).expanduser().resolve()
        legacy_cache_value = os.environ.get("TRANSFORMERS_CACHE", "").strip()
        legacy_cache = Path(legacy_cache_value).expanduser().resolve() if legacy_cache_value else None
        os.environ["HF_HOME"] = str(model_cache)
        os.environ["HF_HUB_CACHE"] = str(model_cache / "hub")
        os.environ.pop("TRANSFORMERS_CACHE", None)
        os.environ.pop("PYTORCH_TRANSFORMERS_CACHE", None)
        return cls(
            output_dir=_path_from_env("REY_VOICE_OUTPUT_DIR", PROJECT_ROOT / "data" / "voice_output"),
            model_cache_dir=model_cache,
            legacy_transformers_cache_dir=legacy_cache,
            keep_output_audio=env_bool("REY_KEEP_OUTPUT_AUDIO", True),
            tts_mode=mode,
            tts_model=os.getenv(
                "REY_TTS_MODEL",
                "Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign" if mode == "design" else "Qwen/Qwen3-TTS-12Hz-1.7B-Base",
            ).strip(),
            tts_device=device,
            tts_instruction=os.getenv(
                "REY_TTS_INSTRUCTION",
                "A natural young adult Russian female voice, calm, warm and expressive, with clear articulation. Avoid robotic delivery.",
            ).strip(),
            reference_audio=reference_audio,
            reference_text=os.getenv("REY_TTS_REFERENCE_TEXT", "").strip(),
            tts_unload_after_synthesis=env_bool("REY_TTS_UNLOAD_AFTER_SYNTHESIS", True),
            tts_cpu_fallback=env_bool("REY_TTS_CPU_FALLBACK", False),
            tts_allow_model_download=env_bool("REY_TTS_ALLOW_MODEL_DOWNLOAD", False),
            tts_chunk_chars=_positive_int("REY_TTS_CHUNK_CHARS", 320),
            ollama_unload_before_tts=env_bool("REY_OLLAMA_UNLOAD_BEFORE_TTS", True),
            stt_model=os.getenv("REY_STT_MODEL", "small").strip(),
            stt_device=stt_device,
            stt_cpu_fallback=env_bool("REY_STT_CPU_FALLBACK", True),
            stt_unload_after_transcription=env_bool("REY_STT_UNLOAD_AFTER_TRANSCRIPTION", True),
            max_audio_seconds=_positive_int("REY_MAX_AUDIO_SECONDS", 180),
            max_audio_bytes=_positive_int("REY_MAX_AUDIO_BYTES", 15 * 1024 * 1024),
            max_reply_chars=_positive_int("REY_MAX_REPLY_CHARS", 1500),
            ffmpeg_timeout_seconds=_positive_int("REY_FFMPEG_TIMEOUT_SECONDS", 120),
            cli_voice_enabled=env_bool("REY_CLI_VOICE_ENABLED", True),
        )


@dataclass(frozen=True)
class TelegramSettings:
    api_id: int
    api_hash: str
    bot_token: str
    allowed_user_ids: frozenset[int]
    allowed_chat_ids: frozenset[int]
    session_path: Path
    send_text: bool
    send_mp3: bool
    max_pending_requests: int

    @classmethod
    def from_env(cls) -> TelegramSettings:
        from app.config import _load_env

        _load_env()
        user_ids = _numeric_ids("REY_TG_ALLOWED_IDS")
        if not user_ids:
            raise ValueError("REY_TG_ALLOWED_IDS must contain at least one numeric Telegram user ID (fail-closed).")
        chat_ids = _numeric_ids("REY_TG_ALLOWED_CHAT_IDS") or user_ids
        required = {
            "REY_TG_API_ID": os.getenv("REY_TG_API_ID", "").strip(),
            "REY_TG_API_HASH": os.getenv("REY_TG_API_HASH", "").strip(),
            "REY_TG_BOT_TOKEN": os.getenv("REY_TG_BOT_TOKEN", "").strip(),
        }
        missing = [key for key, value in required.items() if not value]
        if missing:
            raise ValueError("Missing Telegram settings: " + ", ".join(missing))
        try:
            api_id = int(required["REY_TG_API_ID"])
        except ValueError as exc:
            raise ValueError("REY_TG_API_ID must be numeric") from exc
        return cls(
            api_id=api_id,
            api_hash=required["REY_TG_API_HASH"],
            bot_token=required["REY_TG_BOT_TOKEN"],
            allowed_user_ids=user_ids,
            allowed_chat_ids=chat_ids,
            session_path=_path_from_env("REY_TG_SESSION_PATH", PROJECT_ROOT / "data" / "rey_bot"),
            send_text=env_bool("REY_TG_SEND_TEXT", True),
            send_mp3=env_bool("REY_TG_SEND_MP3", False),
            max_pending_requests=_positive_int("REY_TG_MAX_PENDING_REQUESTS", 3),
        )
