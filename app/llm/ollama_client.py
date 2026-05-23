from __future__ import annotations

import logging
from typing import Any

try:
    import requests
except ImportError:  # pragma: no cover - dependency is in requirements
    requests = None

from app.config import Settings, get_settings


logger = logging.getLogger(__name__)


class OllamaClient:
    def __init__(
        self,
        settings: Settings | None = None,
        host: str | None = None,
        model: str | None = None,
        timeout_seconds: int | None = None,
    ) -> None:
        self.settings = settings or get_settings()
        self.host = (host or self.settings.ollama_host).rstrip("/")
        self.model = model or self.settings.ollama_model
        self.timeout_seconds = timeout_seconds or self.settings.ollama_timeout_seconds

    def chat(self, messages: list[dict[str, str]]) -> str:
        if requests is None:
            logger.error("requests is not installed; Ollama client is unavailable.")
            return "Не могу обратиться к Ollama: не установлен пакет requests."

        endpoint = f"{self.host}/api/chat"
        payload: dict[str, Any] = {
            "model": self.model,
            "messages": messages,
            "stream": False,
        }

        logger.info("Sending request to Ollama model=%s messages=%s", self.model, len(messages))
        try:
            response = requests.post(endpoint, json=payload, timeout=self.timeout_seconds)
            response.raise_for_status()
        except requests.exceptions.Timeout:
            logger.exception("Ollama request timed out.")
            return "Я не дождалась ответа от локальной модели. Проверь, пожалуйста, что Ollama не зависла."
        except requests.exceptions.ConnectionError:
            logger.exception("Ollama connection failed.")
            return "Не могу подключиться к Ollama. Проверь, что `ollama serve` запущен и адрес в .env верный."
        except requests.exceptions.HTTPError as exc:
            logger.exception("Ollama returned HTTP error.")
            return f"Ollama ответила ошибкой HTTP: {exc.response.status_code}. Проверь модель и логи Ollama."
        except requests.exceptions.RequestException:
            logger.exception("Unexpected Ollama request error.")
            return "Что-то пошло не так при запросе к Ollama. Я записала ошибку в лог."

        try:
            data = response.json()
        except ValueError:
            logger.exception("Ollama returned non-JSON response.")
            return "Ollama вернула ответ в неожиданном формате."

        content = data.get("message", {}).get("content")
        if not isinstance(content, str) or not content.strip():
            logger.error("Ollama response has no message.content: %s", data)
            return "Ollama ответила пусто. Похоже, модель не вернула текст."

        return content.strip()

