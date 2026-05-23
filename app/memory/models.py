from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class Message:
    id: int
    session_id: str
    role: str
    content: str
    created_at: str


@dataclass(frozen=True)
class Memory:
    id: int
    key: str
    value: str
    source_message: str
    importance: int
    created_at: str
    updated_at: str


@dataclass(frozen=True)
class MemoryEvent:
    id: int
    event_type: str
    payload: str
    created_at: str

