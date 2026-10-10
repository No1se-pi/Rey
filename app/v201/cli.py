"""Terminal transport: text, microphone, and existing audio files."""
from __future__ import annotations

import logging
from datetime import datetime
from pathlib import Path
from uuid import uuid4

from app.voice.audio_io import record_wav
from app.voice.errors import VoiceDependencyError

from .pipeline import ReyPipeline
from .settings import VoiceSettings

log = logging.getLogger(__name__)
SESSION_KEY = "cli-local"


def _show_legacy_command(pipeline: ReyPipeline, command: str) -> bool:
    service = pipeline.service_for(SESSION_KEY)
    if command == "/memory":
        memories = service.memory_manager.get_memories(limit=50)
        print("Память Рей:" if memories else "Рей: В памяти пока пусто.")
        for memory in memories:
            print(f"  #{memory.id} [{memory.key}] {memory.value} (importance={memory.importance})")
        return True
    if command == "/history":
        messages = service.memory_manager.get_recent_messages(SESSION_KEY, service.settings.max_history_messages)
        print("История сессии:" if messages else "Рей: История этой сессии пока пустая.")
        for message in messages:
            print(f"  {message.role}: {message.content}")
        return True
    if command.startswith("/forget"):
        parts = command.split(maxsplit=1)
        if len(parts) != 2 or not parts[1].isdigit():
            print("Рей: Напиши так: /forget <id>")
        else:
            deleted = service.memory_manager.delete_memory(int(parts[1]))
            print("Рей: Память удалена." if deleted else "Рей: Не нашла такую память.")
        return True
    if command == "/state":
        print("Состояние Рей:", service.avatar_state.to_dict())
        return True
    return False


def run_cli() -> None:
    settings = VoiceSettings.from_env()
    pipeline = ReyPipeline(settings)
    voice_enabled = settings.cli_voice_enabled
    print("Рей 2.0.1 — /mic [секунд], /file <путь>, /text, /voice, /memory, /history, /forget <id>, /state, /exit")
    print("Голосовой ответ:", "включён" if voice_enabled else "выключен")
    print("Аудио сохраняется в:", settings.output_dir)
    while True:
        try:
            command = input("Ты: ").strip()
        except (KeyboardInterrupt, EOFError):
            print("\nРей: До встречи!")
            return
        if not command:
            continue
        if command.lower() in {"/exit", "/quit"}:
            return
        if command == "/text":
            voice_enabled = False
            print("Рей: Текстовый режим включён.")
            continue
        if command == "/voice":
            voice_enabled = True
            print("Рей: Голосовые ответы включены.")
            continue
        if _show_legacy_command(pipeline, command):
            continue

        output_dir = settings.output_dir / f"{datetime.now():%Y%m%d_%H%M%S}_{uuid4().hex[:8]}"
        recognized = None
        try:
            if command.startswith("/file "):
                filepath = Path(command[6:].strip().strip('"')).expanduser()
                recognized = pipeline.transcribe(filepath)
                command = recognized
                print("Распознано:", recognized)
            elif command == "/mic" or command.startswith("/mic "):
                duration = 5.0 if command == "/mic" else float(command[5:].strip())
                if duration <= 0 or duration > settings.max_audio_seconds:
                    raise ValueError("Invalid microphone recording duration")
                output_dir.mkdir(parents=True, exist_ok=True)
                source = record_wav(output_dir / "input.wav", duration_seconds=duration)
                recognized = pipeline.transcribe(source)
                command = recognized
                print("Распознано:", recognized)
            elif command.startswith("/"):
                print("Неизвестная команда. Используй /mic, /file, /text, /voice, /memory, /history, /forget, /state или /exit.")
                continue
            reply = pipeline.reply(
                command,
                SESSION_KEY,
                output_dir,
                recognized_text=recognized,
                synthesize_audio=voice_enabled,
            )
            print("Рей:", reply.text)
            if reply.audio:
                print("MP3:", reply.audio.mp3)
                print("Telegram OGG/Opus:", reply.audio.opus)
        except (ValueError, FileNotFoundError, VoiceDependencyError, RuntimeError) as exc:
            log.exception("Failed to handle CLI command")
            print("Ошибка:", exc)
        except Exception:
            log.exception("Unexpected CLI failure")
            print("Ошибка: непредвиденный сбой. Подробности записаны в локальный лог.")
