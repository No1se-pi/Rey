"""Private Telegram bot transport using Telethon 1.x."""
from __future__ import annotations

import asyncio
import logging
import tempfile
from datetime import datetime
from pathlib import Path
from uuid import uuid4

from .audio import AudioToolError
from .pipeline import ReyPipeline
from .settings import TelegramSettings, VoiceSettings

log = logging.getLogger(__name__)


def is_supported_audio(message) -> bool:
    """Accept native voice notes and audio attachments, not arbitrary documents."""
    if getattr(message, "voice", None) or getattr(message, "audio", None):
        return True
    media_file = getattr(message, "file", None)
    mime_type = (getattr(media_file, "mime_type", "") or "").lower()
    return bool(media_file and mime_type.startswith("audio/"))


class TelegramMessageHandler:
    """Testable adapter around a transport client and the shared pipeline."""

    def __init__(self, client, telegram: TelegramSettings, voice: VoiceSettings, pipeline: ReyPipeline):
        self.client = client
        self.telegram = telegram
        self.voice = voice
        self.pipeline = pipeline
        self.inference_lock = asyncio.Lock()
        self._pending_guard = asyncio.Lock()
        self._pending = 0

    def _authorized(self, event) -> bool:
        return bool(
            getattr(event, "is_private", False)
            and getattr(event, "sender_id", None) in self.telegram.allowed_user_ids
            and getattr(event, "chat_id", None) in self.telegram.allowed_chat_ids
        )

    async def _send_reply_audio(self, event, audio) -> None:
        mp3_sent = False
        try:
            await self.client.send_file(
                event.chat_id,
                str(audio.opus),
                voice_note=True,
                reply_to=event.message.id,
            )
        except Exception as exc:
            from telethon.errors import VoiceMessagesForbiddenError

            if not isinstance(exc, VoiceMessagesForbiddenError):
                raise
            log.warning(
                "Telegram privacy settings rejected voice note; sending MP3 document chat_id=%s",
                event.chat_id,
            )
            await self.client.send_file(
                event.chat_id,
                str(audio.mp3),
                force_document=True,
                reply_to=event.message.id,
            )
            mp3_sent = True
        if self.telegram.send_mp3 and not mp3_sent:
            await self.client.send_file(
                event.chat_id,
                str(audio.mp3),
                force_document=True,
                reply_to=event.message.id,
            )

    async def handle(self, event) -> None:
        if not self._authorized(event):
            return
        message = event.message
        raw_text = (getattr(event, "raw_text", "") or "").strip()
        if raw_text in {"/start", "/help"}:
            await event.reply("Я Рей 2.0.1. Пришли текст, голосовое или аудиофайл. Ответ: текст и OGG/Opus voice note.")
            return
        has_audio = is_supported_audio(message)
        if not has_audio and not raw_text:
            await event.reply("Пока принимаю только текст, аудиофайлы и голосовые сообщения.")
            return
        media_size = getattr(getattr(message, "file", None), "size", None)
        if has_audio and media_size and media_size > self.voice.max_audio_bytes:
            await event.reply("Аудиофайл слишком большой для текущих ограничений Рей.")
            return

        async with self._pending_guard:
            if self._pending >= self.telegram.max_pending_requests:
                await event.reply("Очередь Рей заполнена. Повтори запрос немного позже.")
                return
            self._pending += 1
        try:
            await self._process_authorized(event, message, raw_text, has_audio)
        finally:
            async with self._pending_guard:
                self._pending -= 1

    async def _process_authorized(self, event, message, raw_text: str, has_audio: bool) -> None:
        async with self.inference_lock:
            temp_root = self.voice.output_dir / ".tmp"
            temp_root.mkdir(parents=True, exist_ok=True)
            try:
                with tempfile.TemporaryDirectory(prefix="telegram-", dir=temp_root) as folder:
                    temporary = Path(folder)
                    if has_audio:
                        async with self.client.action(event.chat_id, "typing"):
                            file_path = await message.download_media(file=str(temporary / "received.audio"))
                            if not file_path:
                                raise ValueError("Telegram did not return a downloaded file")
                            downloaded = Path(file_path)
                            if not downloaded.is_file() or downloaded.stat().st_size > self.voice.max_audio_bytes:
                                raise ValueError("Downloaded audio is missing or exceeds the size limit")
                            user_text = await asyncio.to_thread(self.pipeline.transcribe, downloaded)
                    else:
                        user_text = raw_text

                    if self.voice.keep_output_audio:
                        output_dir = self.voice.output_dir / "telegram" / f"{datetime.now():%Y%m%d_%H%M%S}_{uuid4().hex[:8]}"
                    else:
                        output_dir = temporary
                    async with self.client.action(event.chat_id, "record-audio"):
                        response = await asyncio.to_thread(
                            self.pipeline.reply,
                            user_text,
                            f"telegram-{event.sender_id}",
                            output_dir,
                            recognized_text=user_text if has_audio else None,
                        )
                    if response.audio is None:
                        raise RuntimeError("Telegram reply did not contain audio")
                    if self.telegram.send_text:
                        await event.reply(response.text[:4000])
                    await self._send_reply_audio(event, response.audio)
            except (ValueError, FileNotFoundError, AudioToolError):
                log.exception("Rey Telegram input/audio error")
                await event.reply("Не удалось обработать аудио. Проверь формат, длительность и повтори попытку.")
            except RuntimeError:
                log.exception("Rey Telegram inference error")
                await event.reply("Не удалось сформировать голосовой ответ. Подробности записаны в локальный лог.")
            except Exception:
                log.exception("Rey Telegram unexpected error")
                await event.reply("Произошла внутренняя ошибка. Подробности записаны в локальный лог.")


async def run_telegram() -> None:
    settings = TelegramSettings.from_env()
    voice_settings = VoiceSettings.from_env()
    from telethon import TelegramClient, events

    settings.session_path.parent.mkdir(parents=True, exist_ok=True)
    pipeline = ReyPipeline(voice_settings)
    client = TelegramClient(
        str(settings.session_path),
        settings.api_id,
        settings.api_hash,
        auto_reconnect=True,
        connection_retries=5,
        retry_delay=2,
    )
    handler = TelegramMessageHandler(client, settings, voice_settings, pipeline)

    @client.on(events.NewMessage(incoming=True))
    async def on_message(event):
        await handler.handle(event)

    await client.start(bot_token=settings.bot_token)
    log.info("Rey 2.0.1 Telegram bot started allowed_users=%d allowed_chats=%d", len(settings.allowed_user_ids), len(settings.allowed_chat_ids))
    await client.run_until_disconnected()
