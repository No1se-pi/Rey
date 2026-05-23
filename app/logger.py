from __future__ import annotations

import logging
from typing import Optional

from app.config import get_settings


LOG_FORMAT = "%(asctime)s | %(levelname)s | %(name)s | %(message)s"
DATE_FORMAT = "%Y-%m-%d %H:%M:%S"


def setup_logging(level: Optional[str] = None) -> logging.Logger:
    settings = get_settings()
    resolved_level = (level or settings.log_level).upper()
    logging.basicConfig(
        level=getattr(logging, resolved_level, logging.INFO),
        format=LOG_FORMAT,
        datefmt=DATE_FORMAT,
        force=True,
    )
    logging.getLogger("urllib3").setLevel(logging.WARNING)
    return logging.getLogger(settings.app_name)

