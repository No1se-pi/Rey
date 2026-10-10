"""Audio ingestion, offline synthesis, and output encoding."""
from __future__ import annotations

import gc
import json
import logging
import math
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from time import perf_counter
from uuid import uuid4

from .settings import VoiceSettings

log = logging.getLogger(__name__)


class AudioToolError(RuntimeError):
    """An external audio tool or model failed in a user-safe way."""


@dataclass(frozen=True)
class AudioOutputs:
    wav: Path
    mp3: Path
    opus: Path


def _require_tool(name: str) -> str:
    binary = shutil.which(name)
    if not binary:
        raise AudioToolError(f"{name} not found. Install FFmpeg and add it to PATH.")
    return binary


def require_ffmpeg() -> str:
    return _require_tool("ffmpeg")


def _run(args: list[str], *, timeout: int) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(args, capture_output=True, text=True, check=True, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        raise AudioToolError(f"Audio tool timed out after {timeout} seconds") from exc
    except subprocess.CalledProcessError as exc:
        detail = (exc.stderr or exc.stdout or "unknown error").strip().splitlines()
        log.error("Audio command failed executable=%s detail=%s", Path(args[0]).name, detail[-1] if detail else "unknown")
        raise AudioToolError("FFmpeg could not process the audio file; see the local log for details") from exc


def probe_audio(path: Path, *, timeout: int = 30) -> dict[str, object]:
    if not path.is_file() or path.stat().st_size == 0:
        raise AudioToolError("Audio file is missing or empty")
    result = _run(
        [
            _require_tool("ffprobe"), "-v", "error", "-select_streams", "a:0",
            "-show_entries", "stream=codec_name,sample_rate,duration:format=duration",
            "-of", "json", str(path),
        ],
        timeout=timeout,
    )
    try:
        payload = json.loads(result.stdout)
        stream = payload["streams"][0]
        duration = float(stream.get("duration") or payload.get("format", {}).get("duration"))
    except (ValueError, TypeError, KeyError, IndexError, json.JSONDecodeError) as exc:
        raise AudioToolError("Audio file has no readable audio stream") from exc
    if not math.isfinite(duration) or duration <= 0:
        raise AudioToolError("Audio duration is invalid")
    return {"duration": duration, "codec": str(stream.get("codec_name") or ""), "sample_rate": int(stream.get("sample_rate") or 0)}


def encode_audio(wav: Path, folder: Path, *, stem: str | None = None, timeout: int = 120) -> AudioOutputs:
    """Encode one generated WAV to archive MP3 and Telegram OGG/Opus."""
    binary = require_ffmpeg()
    folder.mkdir(parents=True, exist_ok=True)
    stem = stem or f"rey_reply_{uuid4().hex[:12]}"
    mp3 = folder / f"{stem}.mp3"
    opus = folder / f"{stem}.ogg"
    commands = (
        [binary, "-y", "-hide_banner", "-loglevel", "error", "-i", str(wav), "-vn", "-c:a", "libmp3lame", "-q:a", "2", str(mp3)],
        [binary, "-y", "-hide_banner", "-loglevel", "error", "-i", str(wav), "-vn", "-ac", "1", "-ar", "48000", "-c:a", "libopus", "-b:a", "64k", str(opus)],
    )
    started = perf_counter()
    try:
        for command in commands:
            _run(command, timeout=timeout)
        mp3_info = probe_audio(mp3)
        opus_info = probe_audio(opus)
        if mp3_info["codec"] != "mp3" or opus_info["codec"] != "opus":
            raise AudioToolError("FFmpeg produced an unexpected output codec")
    except Exception:
        mp3.unlink(missing_ok=True)
        opus.unlink(missing_ok=True)
        raise
    log.info("Audio conversion finished seconds=%.3f", perf_counter() - started)
    return AudioOutputs(wav=wav, mp3=mp3, opus=opus)


def audio_duration_seconds(path: Path) -> float:
    return float(probe_audio(path)["duration"])


def _is_cuda_error(exc: BaseException) -> bool:
    message = str(exc).lower()
    return any(marker in message for marker in ("cuda", "cublas", "cudnn", "out of memory", "memory allocation"))


class OfflineSTT:
    def __init__(self, settings: VoiceSettings):
        self.settings = settings
        self._model = None
        self._forced_device: str | None = None

    def _load_model(self):
        if self._model is None:
            from faster_whisper import WhisperModel

            device = self._forced_device or self.settings.stt_device
            self._model = WhisperModel(
                self.settings.stt_model,
                device=device,
                compute_type="int8" if device == "cpu" else "default",
            )
        return self._model

    def transcribe(self, source: Path) -> str:
        if not source.is_file() or source.stat().st_size > self.settings.max_audio_bytes:
            raise ValueError("Audio file missing or exceeds configured size limit")
        length = audio_duration_seconds(source)
        if not 0.1 <= length <= self.settings.max_audio_seconds:
            raise ValueError(f"Audio duration must be 0.1-{self.settings.max_audio_seconds} seconds")
        started = perf_counter()
        try:
            try:
                segments, _ = self._load_model().transcribe(str(source), language="ru", vad_filter=True)
                text = " ".join(segment.text.strip() for segment in segments if segment.text.strip()).strip()
            except RuntimeError as exc:
                if not self.settings.stt_cpu_fallback or self._forced_device == "cpu" or not _is_cuda_error(exc):
                    raise
                log.warning("STT CUDA failed; retrying once on CPU: %s", exc)
                self._model = None
                self._forced_device = "cpu"
                segments, _ = self._load_model().transcribe(str(source), language="ru", vad_filter=True)
                text = " ".join(segment.text.strip() for segment in segments if segment.text.strip()).strip()
            log.info("STT finished seconds=%.3f audio_seconds=%.3f text_chars=%d", perf_counter() - started, length, len(text))
            return text
        finally:
            if self.settings.stt_unload_after_transcription:
                self._model = None
                gc.collect()


class QualityTTS:
    """Synchronous Qwen3-TTS renderer. Call it outside async event loops."""

    _MODEL_METADATA = (
        "config.json",
        "generation_config.json",
        "merges.txt",
        "preprocessor_config.json",
        "tokenizer_config.json",
        "vocab.json",
        "speech_tokenizer/config.json",
        "speech_tokenizer/configuration.json",
        "speech_tokenizer/preprocessor_config.json",
    )

    def __init__(self, settings: VoiceSettings):
        self.settings = settings
        self._model = None
        self._clone_prompt = None
        self._forced_device: str | None = None

    def _device(self) -> str:
        device = self._forced_device or self.settings.tts_device
        if device != "auto":
            return "cuda:0" if device == "cuda" else device
        import torch

        return "cuda:0" if torch.cuda.is_available() else "cpu"

    @staticmethod
    def _snapshot_is_complete(snapshot: Path) -> bool:
        tokenizer = snapshot / "speech_tokenizer"
        return (
            all((snapshot / relative).is_file() for relative in QualityTTS._MODEL_METADATA)
            and any(snapshot.glob("*.safetensors"))
            and any(tokenizer.glob("*.safetensors"))
        )

    def _cached_snapshots(self) -> dict[str, list[Path]]:
        repo_folder = "models--" + self.settings.tts_model.replace("/", "--")
        cache_roots = [self.settings.model_cache_dir / "hub"]
        legacy = self.settings.legacy_transformers_cache_dir
        if legacy and legacy not in cache_roots:
            cache_roots.append(legacy)
        conventional_legacy = self.settings.model_cache_dir / "transformers"
        if conventional_legacy not in cache_roots:
            cache_roots.append(conventional_legacy)

        revisions: dict[str, list[Path]] = {}
        for cache_root in cache_roots:
            snapshots = cache_root / repo_folder / "snapshots"
            if not snapshots.is_dir():
                continue
            for snapshot in snapshots.iterdir():
                if snapshot.is_dir():
                    revisions.setdefault(snapshot.name, []).append(snapshot)
        return revisions

    def _merge_split_snapshot(self, revision: str, sources: list[Path]) -> Path | None:
        """Create a hard-linked local view when old cache variables split one model."""
        repo_folder = "models--" + self.settings.tts_model.replace("/", "--")
        destination = self.settings.model_cache_dir / "rey_merged" / repo_folder / revision
        destination.mkdir(parents=True, exist_ok=True)
        for source in sources:
            for item in source.rglob("*"):
                if not item.is_file():
                    continue
                relative = item.relative_to(source)
                target = destination / relative
                if target.exists():
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                resolved = item.resolve()
                try:
                    os.link(resolved, target)
                except FileExistsError:
                    pass
                except OSError:
                    shutil.copy2(resolved, target)
        if (
            self.settings.tts_allow_model_download
            and any(destination.glob("*.safetensors"))
            and any((destination / "speech_tokenizer").glob("*.safetensors"))
        ):
            from huggingface_hub import hf_hub_download

            for relative in self._MODEL_METADATA:
                if not (destination / relative).is_file():
                    hf_hub_download(
                        repo_id=self.settings.tts_model,
                        filename=relative,
                        revision=revision,
                        local_dir=destination,
                    )
        return destination if self._snapshot_is_complete(destination) else None

    def _local_snapshot(self) -> Path | None:
        for revision, snapshots in self._cached_snapshots().items():
            for snapshot in snapshots:
                if self._snapshot_is_complete(snapshot):
                    return snapshot
            merged = self._merge_split_snapshot(revision, snapshots)
            if merged:
                log.info("Recovered split Hugging Face cache into local model view path=%s", merged)
                return merged
        return None

    def _model_source(self) -> str:
        configured = Path(self.settings.tts_model).expanduser()
        if configured.is_dir():
            return str(configured.resolve())
        cached = self._local_snapshot()
        if cached:
            return str(cached.resolve())
        if self.settings.tts_allow_model_download:
            return self.settings.tts_model
        try:
            from huggingface_hub import snapshot_download

            return snapshot_download(repo_id=self.settings.tts_model, local_files_only=True)
        except Exception as exc:
            raise RuntimeError(
                "Qwen3-TTS model is not cached locally. Set REY_TTS_ALLOW_MODEL_DOWNLOAD=true only when you are ready for a multi-gigabyte download."
            ) from exc

    def _load_model(self):
        if self._model is not None:
            return self._model
        # Defensive repeat in case another module changed the process
        # environment after VoiceSettings was created.
        os.environ.pop("TRANSFORMERS_CACHE", None)
        os.environ.pop("PYTORCH_TRANSFORMERS_CACHE", None)
        import torch
        from qwen_tts import Qwen3TTSModel

        device = self._device()
        dtype = torch.float32
        if device.startswith("cuda"):
            dtype = torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16
        log.info("Loading TTS model model=%s device=%s", self.settings.tts_model, device)
        self._model = Qwen3TTSModel.from_pretrained(
            self._model_source(),
            device_map=device,
            dtype=dtype,
            attn_implementation="sdpa",
        )
        return self._model

    @staticmethod
    def _split(text: str, limit: int = 320) -> list[str]:
        if limit < 10:
            raise ValueError("Chunk limit is too small")
        sentences = re.split(r"(?<=[.!?])\s+", text.strip())
        chunks: list[str] = []
        current = ""

        for sentence in filter(None, (item.strip() for item in sentences)):
            words = sentence.split()
            for word in words:
                if len(word) > limit:
                    if current:
                        chunks.append(current)
                        current = ""
                    chunks.extend(word[start : start + limit] for start in range(0, len(word), limit))
                    continue
                proposed = f"{current} {word}".strip()
                if len(proposed) <= limit:
                    current = proposed
                else:
                    chunks.append(current)
                    current = word
        if current:
            chunks.append(current)
        return chunks

    def _clone_voice_prompt(self, model):
        if self._clone_prompt is None:
            self._clone_prompt = model.create_voice_clone_prompt(
                ref_audio=str(self.settings.reference_audio),
                ref_text=self.settings.reference_text,
                x_vector_only_mode=False,
            )
        return self._clone_prompt

    def _release_model(self) -> None:
        self._clone_prompt = None
        self._model = None
        gc.collect()
        try:
            import torch

            if torch.cuda.is_available():
                torch.cuda.empty_cache()
        except ImportError:
            pass

    def _render_once(self, text: str, destination: Path) -> Path:
        import numpy as np
        import soundfile as sf

        model = self._load_model()
        clone_prompt = self._clone_voice_prompt(model) if self.settings.tts_mode == "clone" else None
        waves: list[object] = []
        sample_rate: int | None = None
        chunks = self._split(text, self.settings.tts_chunk_chars)
        for index, chunk in enumerate(chunks):
            if clone_prompt is not None:
                segments, rate = model.generate_voice_clone(text=chunk, language="Russian", voice_clone_prompt=clone_prompt)
            else:
                segments, rate = model.generate_voice_design(
                    text=chunk, language="Russian", instruct=self.settings.tts_instruction
                )
            if not segments:
                raise RuntimeError(f"TTS returned no audio for segment {index + 1}")
            if sample_rate is not None and rate != sample_rate:
                raise RuntimeError("TTS unexpectedly changed sample rate across chunks")
            sample_rate = int(rate)
            wave = np.asarray(segments[0], dtype=np.float32).reshape(-1)
            if wave.size == 0 or not np.isfinite(wave).all():
                raise RuntimeError(f"TTS returned invalid audio for segment {index + 1}")
            waves.append(wave)
            if index < len(chunks) - 1:
                waves.append(np.zeros(int(sample_rate * 0.20), dtype=np.float32))
        if not sample_rate:
            raise RuntimeError("TTS returned no sample rate")
        destination.parent.mkdir(parents=True, exist_ok=True)
        temporary = destination.with_name(f".{destination.stem}.{uuid4().hex}.tmp.wav")
        try:
            sf.write(str(temporary), np.concatenate(waves), sample_rate)
            os.replace(temporary, destination)
        finally:
            temporary.unlink(missing_ok=True)
        return destination

    def render(self, text: str, destination: Path) -> Path:
        text = text.strip()
        if not text:
            raise ValueError("Cannot synthesize an empty reply")
        if len(text) > self.settings.max_reply_chars:
            raise ValueError("Response longer than REY_MAX_REPLY_CHARS")
        if self.settings.tts_mode == "clone":
            if not self.settings.reference_audio or not self.settings.reference_audio.is_file():
                raise ValueError("Clone mode requires an existing REY_TTS_REFERENCE_AUDIO")
            if not self.settings.reference_text:
                raise ValueError("Clone mode requires REY_TTS_REFERENCE_TEXT")
        started = perf_counter()
        try:
            try:
                result = self._render_once(text, destination)
            except RuntimeError as exc:
                if not self.settings.tts_cpu_fallback or self._device() == "cpu" or not _is_cuda_error(exc):
                    raise
                log.warning("TTS CUDA failed; retrying once on CPU: %s", exc)
                self._release_model()
                self._forced_device = "cpu"
                result = self._render_once(text, destination)
            log.info("TTS finished seconds=%.3f text_chars=%d", perf_counter() - started, len(text))
            return result
        finally:
            if self.settings.tts_unload_after_synthesis:
                self._release_model()
