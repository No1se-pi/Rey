from __future__ import annotations

import json
import logging
from datetime import datetime, timezone
from typing import Any

from app.memory.db import Database
from app.memory.models import Memory, MemoryEvent, Message


logger = logging.getLogger(__name__)


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _message_from_row(row: Any) -> Message:
    return Message(
        id=row["id"],
        session_id=row["session_id"],
        role=row["role"],
        content=row["content"],
        created_at=row["created_at"],
    )


def _memory_from_row(row: Any) -> Memory:
    return Memory(
        id=row["id"],
        key=row["key"],
        value=row["value"],
        source_message=row["source_message"],
        importance=row["importance"],
        created_at=row["created_at"],
        updated_at=row["updated_at"],
    )


def _event_from_row(row: Any) -> MemoryEvent:
    return MemoryEvent(
        id=row["id"],
        event_type=row["event_type"],
        payload=row["payload"],
        created_at=row["created_at"],
    )


class MemoryRepository:
    def __init__(self, database: Database | None = None) -> None:
        self.database = database or Database()

    def save_message(self, session_id: str, role: str, content: str) -> Message:
        created_at = _now()
        with self.database.connect() as connection:
            cursor = connection.execute(
                """
                INSERT INTO messages(session_id, role, content, created_at)
                VALUES (?, ?, ?, ?)
                """,
                (session_id, role, content, created_at),
            )
            message_id = int(cursor.lastrowid)
        return Message(message_id, session_id, role, content, created_at)

    def get_recent_messages(self, session_id: str, limit: int) -> list[Message]:
        with self.database.connect() as connection:
            rows = connection.execute(
                """
                SELECT id, session_id, role, content, created_at
                FROM messages
                WHERE session_id = ?
                ORDER BY id DESC
                LIMIT ?
                """,
                (session_id, limit),
            ).fetchall()
        return [_message_from_row(row) for row in reversed(rows)]

    def add_memory(self, key: str, value: str, source_message: str, importance: int = 1) -> Memory:
        timestamp = _now()
        with self.database.connect() as connection:
            cursor = connection.execute(
                """
                INSERT INTO memories(key, value, source_message, importance, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?)
                """,
                (key, value, source_message, importance, timestamp, timestamp),
            )
            memory_id = int(cursor.lastrowid)
        logger.info("Memory saved id=%s key=%s importance=%s", memory_id, key, importance)
        self.add_event("memory_saved", {"id": memory_id, "key": key, "importance": importance})
        return Memory(memory_id, key, value, source_message, importance, timestamp, timestamp)

    def get_memories(self, limit: int = 20) -> list[Memory]:
        with self.database.connect() as connection:
            rows = connection.execute(
                """
                SELECT id, key, value, source_message, importance, created_at, updated_at
                FROM memories
                ORDER BY importance DESC, updated_at DESC, id DESC
                LIMIT ?
                """,
                (limit,),
            ).fetchall()
        return [_memory_from_row(row) for row in rows]

    def delete_memory(self, memory_id: int) -> bool:
        with self.database.connect() as connection:
            cursor = connection.execute("DELETE FROM memories WHERE id = ?", (memory_id,))
            deleted = cursor.rowcount > 0
        if deleted:
            logger.info("Memory deleted id=%s", memory_id)
            self.add_event("memory_deleted", {"id": memory_id})
        return deleted

    def add_event(self, event_type: str, payload: dict[str, Any] | str) -> MemoryEvent:
        created_at = _now()
        payload_text = payload if isinstance(payload, str) else json.dumps(payload, ensure_ascii=False)
        with self.database.connect() as connection:
            cursor = connection.execute(
                """
                INSERT INTO memory_events(event_type, payload, created_at)
                VALUES (?, ?, ?)
                """,
                (event_type, payload_text, created_at),
            )
            event_id = int(cursor.lastrowid)
        return MemoryEvent(event_id, event_type, payload_text, created_at)

