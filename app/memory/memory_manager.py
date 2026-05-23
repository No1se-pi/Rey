from __future__ import annotations

import logging
import re
from dataclasses import dataclass

from app.memory.db import Database
from app.memory.models import Memory, Message
from app.memory.repository import MemoryRepository


logger = logging.getLogger(__name__)


EXPLICIT_MEMORY_MARKERS = (
    "запомни",
    "помни",
    "не забудь",
    "меня зовут",
    "мое имя",
    "моё имя",
    "я люблю",
    "мне нравится",
    "я работаю над",
    "мой проект",
)

SENSITIVE_MARKERS = {
    "пароль",
    "token",
    "токен",
    "api key",
    "секретный ключ",
    "адрес",
    "живу по",
    "паспорт",
    "снилс",
    "карта",
    "номер карты",
    "cvv",
    "телефон",
    "диагноз",
    "болезнь",
    "здоровье",
    "религ",
    "политик",
    "интим",
    "секс",
}


@dataclass(frozen=True)
class ExtractedMemory:
    key: str
    value: str
    importance: int = 1


class MemoryManager:
    def __init__(self, repository: MemoryRepository | None = None, database_url: str | None = None) -> None:
        if repository is not None:
            self.repository = repository
        else:
            self.repository = MemoryRepository(Database(database_url=database_url))

    def save_message(self, session_id: str, role: str, content: str) -> Message:
        return self.repository.save_message(session_id=session_id, role=role, content=content)

    def get_recent_messages(self, session_id: str, limit: int) -> list[Message]:
        return self.repository.get_recent_messages(session_id=session_id, limit=limit)

    def add_memory(self, key: str, value: str, source_message: str, importance: int = 1) -> Memory:
        return self.repository.add_memory(key=key, value=value, source_message=source_message, importance=importance)

    def get_memories(self, limit: int = 20) -> list[Memory]:
        return self.repository.get_memories(limit=limit)

    def delete_memory(self, memory_id: int) -> bool:
        return self.repository.delete_memory(memory_id=memory_id)

    def maybe_extract_memory(self, user_text: str, assistant_response: object | None = None) -> list[Memory]:
        del assistant_response
        normalized = user_text.strip().lower()
        if not normalized:
            return []

        if not any(marker in normalized for marker in EXPLICIT_MEMORY_MARKERS):
            return []

        if self._is_sensitive(normalized):
            logger.info("Skipped memory extraction because text looked sensitive.")
            self.repository.add_event("memory_skipped_sensitive", {"source_message": user_text})
            return []

        extracted = self._extract_candidates(user_text)
        saved: list[Memory] = []
        for candidate in extracted:
            if not self._is_sensitive(candidate.value.lower()):
                saved.append(
                    self.add_memory(
                        key=candidate.key,
                        value=candidate.value,
                        source_message=user_text,
                        importance=candidate.importance,
                    )
                )
        return saved

    def _is_sensitive(self, normalized_text: str) -> bool:
        return any(marker in normalized_text for marker in SENSITIVE_MARKERS)

    def _extract_candidates(self, user_text: str) -> list[ExtractedMemory]:
        text = user_text.strip()
        candidates: list[ExtractedMemory] = []

        name_match = re.search(
            r"(?:меня зовут|мо[её] имя)\s+([A-Za-zА-Яа-яЁё][A-Za-zА-Яа-яЁё0-9_-]{1,40})",
            text,
            flags=re.IGNORECASE,
        )
        if name_match:
            candidates.append(ExtractedMemory("user_name", self._clean_value(name_match.group(1)), importance=3))

        generic_notes: list[ExtractedMemory] = []
        for pattern, key in (
            (r"(?:я люблю|мне нравится)\s+(.+)", "preference"),
            (r"я работаю над\s+(.+)", "current_work"),
            (r"мой проект\s+(.+)", "project"),
            (r"(?:запомни|помни|не забудь)[,:]?\s*(?:что\s+)?(.+)", "note"),
        ):
            match = re.search(pattern, text, flags=re.IGNORECASE)
            if match:
                value = self._clean_value(match.group(1))
                if value:
                    candidate = ExtractedMemory(key, value, importance=2 if key in {"project", "current_work"} else 1)
                    if key == "note":
                        generic_notes.append(candidate)
                    else:
                        candidates.append(candidate)

        if not candidates:
            candidates.extend(generic_notes)

        return self._deduplicate(candidates)

    def _clean_value(self, value: str) -> str:
        cleaned = re.split(r"[.!?\n]", value.strip(), maxsplit=1)[0].strip(" ,;:-")
        if len(cleaned) > 240:
            cleaned = cleaned[:240].rstrip()
        return cleaned

    def _deduplicate(self, candidates: list[ExtractedMemory]) -> list[ExtractedMemory]:
        seen: set[tuple[str, str]] = set()
        result: list[ExtractedMemory] = []
        for candidate in candidates:
            marker = (candidate.key, candidate.value.lower())
            if candidate.value and marker not in seen:
                seen.add(marker)
                result.append(candidate)
        return result
