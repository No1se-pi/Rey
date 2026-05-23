from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from uuid import uuid4


@dataclass
class ChatSession:
    session_id: str = field(default_factory=lambda: f"session-{uuid4().hex}")
    started_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))

