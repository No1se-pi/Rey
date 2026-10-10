"""Rey 2.0.1 tests that require no network, Telegram account, or ML model."""
from __future__ import annotations

import asyncio
import os
import shutil
import struct
import threading
import time
import wave
from concurrent.futures import ThreadPoolExecutor
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace

import pytest

from app.v201.audio import AudioOutputs, QualityTTS, encode_audio, probe_audio
from app.v201.pipeline import ReyPipeline, ReyReply
from app.v201.settings import TelegramSettings, VoiceSettings
from app.v201.telegram_bot import TelegramMessageHandler, is_supported_audio
from app.voice.audio_io import record_wav
from app.voice.errors import VoiceDependencyError


def voice_settings(monkeypatch, **changes) -> VoiceSettings:
    monkeypatch.setenv("REY_TTS_ALLOW_MODEL_DOWNLOAD", "false")
    return replace(VoiceSettings.from_env(), **changes)


def test_tts_chunks_respect_limit():
    chunks = QualityTTS._split("Привет! Как дела? " + "Я работаю над новым проектом. " * 18, 100)
    assert chunks
    assert all(0 < len(chunk) <= 100 for chunk in chunks)
    assert "Привет!" in chunks[0]
    assert "проектом." in chunks[-1]


def test_clone_prompt_is_reused(monkeypatch, tmp_path):
    reference = tmp_path / "reference.wav"
    reference.touch()
    tts = QualityTTS(voice_settings(monkeypatch, tts_mode="clone", reference_audio=reference, reference_text="Текст"))

    class Model:
        calls = 0

        def create_voice_clone_prompt(self, **kwargs):
            self.calls += 1
            return ["cached-prompt"]

    model = Model()
    assert tts._clone_voice_prompt(model) == ["cached-prompt"]
    assert tts._clone_voice_prompt(model) == ["cached-prompt"]
    assert model.calls == 1


def test_tts_model_download_is_fail_closed(monkeypatch):
    tts = QualityTTS(voice_settings(monkeypatch, tts_model="vendor/not-cached", tts_allow_model_download=False))
    monkeypatch.setattr("huggingface_hub.snapshot_download", lambda **_kwargs: (_ for _ in ()).throw(FileNotFoundError()))
    with pytest.raises(RuntimeError, match="not cached locally"):
        tts._model_source()


def test_tts_local_model_path_never_uses_network(monkeypatch, tmp_path):
    model_dir = tmp_path / "model"
    model_dir.mkdir()
    tts = QualityTTS(voice_settings(monkeypatch, tts_model=str(model_dir), tts_allow_model_download=False))
    assert tts._model_source() == str(model_dir.resolve())


def test_tts_recovers_model_split_between_huggingface_caches(monkeypatch, tmp_path):
    model = "vendor/model"
    revision = "abc123"
    hub_snapshot = tmp_path / "hub" / "models--vendor--model" / "snapshots" / revision
    legacy_snapshot = tmp_path / "legacy" / "models--vendor--model" / "snapshots" / revision
    tokenizer = hub_snapshot / "speech_tokenizer"
    tokenizer.mkdir(parents=True)
    legacy_snapshot.mkdir(parents=True)
    for name in ("config.json", "configuration.json", "preprocessor_config.json", "model.safetensors"):
        (tokenizer / name).write_bytes(b"tokenizer")
    for name in (
        "config.json",
        "generation_config.json",
        "merges.txt",
        "preprocessor_config.json",
        "tokenizer_config.json",
        "vocab.json",
        "model.safetensors",
    ):
        (legacy_snapshot / name).write_bytes(b"model")

    settings = voice_settings(
        monkeypatch,
        tts_model=model,
        model_cache_dir=tmp_path,
        legacy_transformers_cache_dir=tmp_path / "legacy",
    )
    source = Path(QualityTTS(settings)._model_source())

    assert source.parent.name == "models--vendor--model"
    assert (source / "model.safetensors").is_file()
    assert (source / "speech_tokenizer" / "preprocessor_config.json").is_file()
    assert (source / "speech_tokenizer" / "model.safetensors").is_file()


def test_telegram_fail_closed(monkeypatch):
    for key in ("REY_TG_ALLOWED_IDS", "REY_TG_API_ID", "REY_TG_API_HASH", "REY_TG_BOT_TOKEN"):
        monkeypatch.setenv(key, "")
    with pytest.raises(ValueError, match="ALLOWED_IDS"):
        TelegramSettings.from_env()


def test_settings_reject_invalid_limits(monkeypatch):
    monkeypatch.setenv("REY_MAX_AUDIO_SECONDS", "zero")
    with pytest.raises(ValueError, match="REY_MAX_AUDIO_SECONDS"):
        VoiceSettings.from_env()


def test_settings_unifies_huggingface_cache(monkeypatch, tmp_path):
    existing = tmp_path / "existing-hf-cache"
    monkeypatch.setenv("HF_HOME", str(existing))
    monkeypatch.setenv("TRANSFORMERS_CACHE", str(tmp_path / "legacy-transformers-cache"))
    settings = VoiceSettings.from_env()
    assert settings.model_cache_dir == existing.resolve()
    assert settings.legacy_transformers_cache_dir == (tmp_path / "legacy-transformers-cache").resolve()
    assert os.environ["HF_HUB_CACHE"] == str(existing.resolve() / "hub")
    assert "TRANSFORMERS_CACHE" not in os.environ


def test_media_filter():
    assert is_supported_audio(SimpleNamespace(voice=True, audio=False, file=None))
    assert is_supported_audio(SimpleNamespace(voice=False, audio=False, file=SimpleNamespace(mime_type="audio/ogg")))
    assert not is_supported_audio(SimpleNamespace(voice=False, audio=False, file=SimpleNamespace(mime_type="image/png")))


def test_encode_creates_mp3_and_opus(monkeypatch, tmp_path):
    wav = tmp_path / "reply.wav"
    wav.touch()
    runs = []

    def fake_run(args, **kwargs):
        runs.append(args)
        Path(args[-1]).write_bytes(b"test")
        return SimpleNamespace(stdout="")

    monkeypatch.setattr("app.v201.audio.require_ffmpeg", lambda: "ffmpeg")
    monkeypatch.setattr("app.v201.audio._run", fake_run)
    monkeypatch.setattr(
        "app.v201.audio.probe_audio",
        lambda path: {"duration": 1.0, "codec": "opus" if path.suffix == ".ogg" else "mp3", "sample_rate": 48000},
    )
    out = encode_audio(wav, tmp_path)
    assert out.mp3.exists() and out.opus.exists()
    assert out.mp3.stem == out.opus.stem
    assert any("libmp3lame" in args for args in runs)
    assert any("libopus" in args for args in runs)


@pytest.mark.skipif(not shutil.which("ffmpeg") or not shutil.which("ffprobe"), reason="FFmpeg is not installed")
def test_real_ffmpeg_outputs_valid_codecs(tmp_path):
    wav_path = tmp_path / "tone.wav"
    with wave.open(str(wav_path), "wb") as audio:
        audio.setnchannels(1)
        audio.setsampwidth(2)
        audio.setframerate(24000)
        audio.writeframes(b"".join(struct.pack("<h", 0) for _ in range(4800)))
    outputs = encode_audio(wav_path, tmp_path, timeout=30)
    assert probe_audio(outputs.mp3)["codec"] == "mp3"
    assert probe_audio(outputs.opus)["codec"] == "opus"
    assert probe_audio(outputs.opus)["sample_rate"] == 48000


def test_one_pipeline_shared_between_text_and_voice(monkeypatch, tmp_path):
    settings = voice_settings(monkeypatch, ollama_unload_before_tts=False)
    services = []

    class FakeService:
        def __init__(self, session_id):
            self.session_id = session_id
            services.append(self)

        def handle_user_text(self, text, *, voice_mode):
            return SimpleNamespace(text="Хорошо, " + text)

    class FakeTTS:
        def render(self, text, destination):
            destination.write_bytes(b"wav")
            return destination

    def fake_encode(wav, folder, **kwargs):
        return AudioOutputs(wav, folder / "x.mp3", folder / "x.ogg")

    monkeypatch.setattr("app.v201.pipeline.encode_audio", fake_encode)
    pipeline = ReyPipeline(settings, service_factory=FakeService, tts=FakeTTS())
    response = pipeline.reply("Привет", "same-user", tmp_path)
    response2 = pipeline.reply("Опять привет", "same-user", tmp_path, synthesize_audio=False)
    assert response.text == "Хорошо, Привет" and response.audio is not None
    assert response2.text == "Хорошо, Опять привет" and response2.audio is None
    assert len(services) == 1


def test_pipeline_serializes_concurrent_generation(monkeypatch, tmp_path):
    settings = voice_settings(monkeypatch, ollama_unload_before_tts=False)
    active = 0
    maximum = 0
    state_lock = threading.Lock()

    class SlowService:
        def __init__(self, _session_id):
            pass

        def handle_user_text(self, text, *, voice_mode):
            nonlocal active, maximum
            with state_lock:
                active += 1
                maximum = max(maximum, active)
            time.sleep(0.04)
            with state_lock:
                active -= 1
            return SimpleNamespace(text=text)

    pipeline = ReyPipeline(settings, service_factory=SlowService)
    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(pipeline.reply, str(index), str(index), tmp_path, synthesize_audio=False) for index in range(2)]
        assert [future.result().text for future in futures] == ["0", "1"]
    assert maximum == 1


def test_missing_microphone_dependency_is_clear(monkeypatch, tmp_path):
    monkeypatch.setattr("app.voice.audio_io.has_audio_recording_dependencies", lambda: (False, ["sounddevice"]))
    with pytest.raises(VoiceDependencyError, match="sounddevice"):
        record_wav(tmp_path / "input.wav", duration_seconds=0.1)


class FakeAction:
    async def __aenter__(self):
        return self

    async def __aexit__(self, *_args):
        return False


class FakeClient:
    def __init__(self):
        self.sent = []

    def action(self, *_args):
        return FakeAction()

    async def send_file(self, chat_id, file, **kwargs):
        assert Path(file).is_file()
        self.sent.append((chat_id, Path(file).suffix, kwargs))


class FakeEvent:
    def __init__(self, text="Привет", *, sender_id=7, chat_id=7, private=True, message=None):
        self.raw_text = text
        self.sender_id = sender_id
        self.chat_id = chat_id
        self.is_private = private
        self.message = message or SimpleNamespace(id=10, voice=False, audio=False, file=None)
        self.replies = []

    async def reply(self, text):
        self.replies.append(text)


def telegram_settings(tmp_path) -> TelegramSettings:
    return TelegramSettings(
        api_id=1,
        api_hash="hash",
        bot_token="token",
        allowed_user_ids=frozenset({7}),
        allowed_chat_ids=frozenset({7}),
        session_path=tmp_path / "session",
        send_text=True,
        send_mp3=False,
        max_pending_requests=3,
    )


def test_telegram_rejects_unauthorized_without_response(monkeypatch, tmp_path):
    class Pipeline:
        def reply(self, *_args, **_kwargs):
            raise AssertionError("pipeline must not run")

    event = FakeEvent(sender_id=8, chat_id=8)
    handler = TelegramMessageHandler(FakeClient(), telegram_settings(tmp_path), voice_settings(monkeypatch), Pipeline())
    asyncio.run(handler.handle(event))
    assert event.replies == []


def test_telegram_text_handler_sends_real_voice_note_flag(monkeypatch, tmp_path):
    class Pipeline:
        def reply(self, text, session_key, output_dir, **_kwargs):
            output_dir.mkdir(parents=True, exist_ok=True)
            wav = output_dir / "reply.wav"
            mp3 = output_dir / "reply.mp3"
            opus = output_dir / "reply.ogg"
            for path in (wav, mp3, opus):
                path.write_bytes(b"audio")
            return ReyReply("Ответ", AudioOutputs(wav, mp3, opus))

    client = FakeClient()
    event = FakeEvent()
    voice = voice_settings(monkeypatch, output_dir=tmp_path / "audio", keep_output_audio=False)
    handler = TelegramMessageHandler(client, telegram_settings(tmp_path), voice, Pipeline())
    asyncio.run(handler.handle(event))
    assert event.replies == ["Ответ"]
    assert client.sent[0][1] == ".ogg"
    assert client.sent[0][2]["voice_note"] is True


def test_telegram_voice_privacy_falls_back_to_mp3_document(monkeypatch, tmp_path):
    from telethon.errors import VoiceMessagesForbiddenError

    class Pipeline:
        def reply(self, text, session_key, output_dir, **_kwargs):
            output_dir.mkdir(parents=True, exist_ok=True)
            wav = output_dir / "reply.wav"
            mp3 = output_dir / "reply.mp3"
            opus = output_dir / "reply.ogg"
            for path in (wav, mp3, opus):
                path.write_bytes(b"audio")
            return ReyReply("Ответ", AudioOutputs(wav, mp3, opus))

    class PrivacyClient(FakeClient):
        async def send_file(self, chat_id, file, **kwargs):
            if kwargs.get("voice_note"):
                raise VoiceMessagesForbiddenError(request=None)
            await super().send_file(chat_id, file, **kwargs)

    client = PrivacyClient()
    event = FakeEvent()
    voice = voice_settings(monkeypatch, output_dir=tmp_path / "audio", keep_output_audio=False)
    handler = TelegramMessageHandler(client, telegram_settings(tmp_path), voice, Pipeline())
    asyncio.run(handler.handle(event))

    assert event.replies == ["Ответ"]
    assert client.sent == [(7, ".mp3", {"force_document": True, "reply_to": 10})]


def test_telegram_rejects_oversized_audio_before_download(monkeypatch, tmp_path):
    class Message:
        id = 2
        voice = True
        audio = False
        file = SimpleNamespace(size=101, mime_type="audio/ogg")

        async def download_media(self, **_kwargs):
            raise AssertionError("oversized media must not be downloaded")

    event = FakeEvent(text="", message=Message())
    voice = voice_settings(monkeypatch, max_audio_bytes=100)
    handler = TelegramMessageHandler(FakeClient(), telegram_settings(tmp_path), voice, SimpleNamespace())
    asyncio.run(handler.handle(event))
    assert "слишком большой" in event.replies[0]


def test_telegram_rejects_when_queue_is_full(monkeypatch, tmp_path):
    event = FakeEvent()
    settings = replace(telegram_settings(tmp_path), max_pending_requests=1)
    handler = TelegramMessageHandler(FakeClient(), settings, voice_settings(monkeypatch), SimpleNamespace())
    handler._pending = 1
    asyncio.run(handler.handle(event))
    assert "Очередь" in event.replies[0]
