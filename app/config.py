from __future__ import annotations

import os
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

try:
    from dotenv import load_dotenv
except ImportError:  # pragma: no cover - handled by requirements, kept for resilience
    load_dotenv = None


PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _load_env() -> None:
    if load_dotenv is not None:
        load_dotenv(PROJECT_ROOT / ".env")


def _get_bool(name: str, default: bool) -> bool:
    raw = os.getenv(name)
    if raw is None:
        return default
    return raw.strip().lower() in {"1", "true", "yes", "y", "on"}


def _get_int(name: str, default: int) -> int:
    raw = os.getenv(name)
    if raw is None:
        return default
    try:
        return int(raw)
    except ValueError:
        return default


def _get_float(name: str, default: float) -> float:
    raw = os.getenv(name)
    if raw is None:
        return default
    try:
        return float(raw)
    except ValueError:
        return default


def sqlite_url_to_path(database_url: str) -> Path:
    if not database_url.startswith("sqlite:///"):
        raise ValueError("Only sqlite:/// DATABASE_URL is supported in Rei MVP.")

    raw_path = database_url.removeprefix("sqlite:///")
    path = Path(raw_path)
    if not path.is_absolute():
        path = PROJECT_ROOT / path
    return path


@dataclass(frozen=True)
class Settings:
    app_name: str
    app_env: str
    log_level: str
    ollama_host: str
    ollama_model: str
    database_url: str
    stt_model_size: str
    stt_device: str
    stt_compute_type: str
    tts_engine: str
    tts_voice_name: str
    tts_rate: int
    tts_volume: float
    voice_enabled: bool
    memory_enabled: bool
    max_history_messages: int
    child_safe_mode: str
    ollama_timeout_seconds: int = 90

    @property
    def database_path(self) -> Path:
        return sqlite_url_to_path(self.database_url)

    def ensure_directories(self) -> None:
        self.database_path.parent.mkdir(parents=True, exist_ok=True)


@lru_cache(maxsize=1)
def get_settings() -> Settings:
    _load_env()
    return Settings(
        app_name=os.getenv("APP_NAME", "rei"),
        app_env=os.getenv("APP_ENV", "dev"),
        log_level=os.getenv("LOG_LEVEL", "INFO"),
        ollama_host=os.getenv("OLLAMA_HOST", "http://localhost:11434"),
        ollama_model=os.getenv("OLLAMA_MODEL", "qwen2.5:7b-instruct"),
        database_url=os.getenv("DATABASE_URL", "sqlite:///data/rei_memory.sqlite3"),
        stt_model_size=os.getenv("STT_MODEL_SIZE", "small"),
        stt_device=os.getenv("STT_DEVICE", "auto"),
        stt_compute_type=os.getenv("STT_COMPUTE_TYPE", "auto"),
        tts_engine=os.getenv("TTS_ENGINE", "pyttsx3"),
        tts_voice_name=os.getenv("TTS_VOICE_NAME", ""),
        tts_rate=_get_int("TTS_RATE", 175),
        tts_volume=_get_float("TTS_VOLUME", 0.9),
        voice_enabled=_get_bool("VOICE_ENABLED", False),
        memory_enabled=_get_bool("MEMORY_ENABLED", True),
        max_history_messages=_get_int("MAX_HISTORY_MESSAGES", 20),
        child_safe_mode=os.getenv("CHILD_SAFE_MODE", "auto"),
        ollama_timeout_seconds=_get_int("OLLAMA_TIMEOUT_SECONDS", 90),
    )

