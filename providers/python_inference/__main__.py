"""Faster-Whisper and Qwen3-TTS worker. Run with python -m providers.python_inference."""
from __future__ import annotations

import argparse
import base64
import hmac
import json
import os
import sys
import tempfile
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

from app.v201.audio import OfflineSTT, QualityTTS, encode_audio
from app.v201.settings import VoiceSettings


class Inference:
    def __init__(self) -> None:
        self.settings = VoiceSettings.from_env()
        self.stt = OfflineSTT(self.settings)
        self.tts = QualityTTS(self.settings)

    def transcribe(self, audio: Path) -> str:
        text = self.stt.transcribe(audio).strip()
        if not text:
            raise ValueError("Разборчивая речь не найдена")
        return text

    def synthesize(self, text: str, folder: Path):
        text = text.strip()
        if len(text) > self.settings.max_reply_chars:
            text = text[: self.settings.max_reply_chars]
            text = text.rsplit(" ", 1)[0] or text
        wav = self.tts.render(text, folder / "reply.wav")
        return encode_audio(wav, folder, stem="reply", timeout=self.settings.ffmpeg_timeout_seconds)


def serve(inference: Inference, port: int) -> None:
    token = os.environ.get("REY_VOICE_TOKEN", "")
    if len(token.encode()) < 16:
        raise ValueError("REY_VOICE_TOKEN должен содержать минимум 16 байт")

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args) -> None:
            pass

        def send_json(self, status: int, payload: dict) -> None:
            data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_POST(self) -> None:
            if not hmac.compare_digest(self.headers.get("Authorization", "").encode(), f"Bearer {token}".encode()):
                self.send_json(401, {"error": "Unauthorized"})
                return
            maximum = {"/v1/transcribe": inference.settings.max_audio_bytes, "/v1/synthesize": 32768}.get(self.path)
            if maximum is None:
                self.send_json(404, {"error": "Unknown endpoint"})
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= maximum:
                    self.send_json(413, {"error": "Invalid content length"})
                    return
                data = self.rfile.read(length)
                if len(data) != length:
                    return
                with tempfile.TemporaryDirectory(prefix="rey-inference-") as temporary:
                    folder = Path(temporary)
                    if self.path == "/v1/transcribe":
                        audio = folder / "input.audio"
                        audio.write_bytes(data)
                        self.send_json(200, {"text": inference.transcribe(audio)})
                    else:
                        text = json.loads(data.decode("utf-8"))["text"]
                        if not isinstance(text, str):
                            raise ValueError("Expected text")
                        paths = inference.synthesize(text, folder)
                        self.send_json(200, {key: base64.b64encode(getattr(paths, key).read_bytes()).decode("ascii")
                                             for key in ("wav", "mp3", "opus")})
            except (BrokenPipeError, ConnectionResetError):
                return
            except Exception as exc:
                print(f"Inference failed: {type(exc).__name__}", file=sys.stderr)
                self.send_json(422, {"error": "Ошибка инференса. Проверь конфигурацию и лог worker."})

    class LoopbackServer(HTTPServer):
        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(15)
            return connection, address

    with LoopbackServer(("127.0.0.1", port), Handler) as server:
        server.serve_forever()


def main() -> None:
    arguments = argparse.ArgumentParser()
    commands = arguments.add_subparsers(dest="operation", required=True)
    commands.add_parser("transcribe").add_argument("input", type=Path)
    commands.add_parser("synthesize").add_argument("output", type=Path)
    commands.add_parser("serve").add_argument("--port", type=int, default=8722)
    args = arguments.parse_args()
    sys.stdout.reconfigure(encoding="utf-8")
    inference = Inference()
    if args.operation == "transcribe":
        print(json.dumps({"text": inference.transcribe(args.input)}, ensure_ascii=False))
    elif args.operation == "synthesize":
        payload = json.loads(sys.stdin.buffer.read(32769).decode("utf-8"))
        inference.synthesize(payload["text"], args.output)
        print('{"ok":true}')
    else:
        serve(inference, args.port)


if __name__ == "__main__":
    main()
