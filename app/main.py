from __future__ import annotations

import logging

from app.chat.chat_loop import ChatLoop
from app.config import get_settings
from app.logger import setup_logging


def main() -> None:
    settings = get_settings()
    settings.ensure_directories()
    setup_logging(settings.log_level)
    logger = logging.getLogger(__name__)
    logger.info("Starting application app=%s env=%s", settings.app_name, settings.app_env)
    logger.info("Selected LLM model: %s", settings.ollama_model)

    ChatLoop().run()


if __name__ == "__main__":
    main()

