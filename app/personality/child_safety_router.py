from __future__ import annotations

import re


AGE_PATTERNS = (
    r"\bмне\s+(?:всего\s+)?([5-9]|1[0-7])\b",
    r"\bмне\s+([5-9]|1[0-7])\s+лет\b",
    r"\bя\s+(?:ещ[её]\s+)?(?:реб[её]нок|школьник|школьница|подросток)\b",
    r"\bя\s+в\s+([1-9]|1[0-1])\s*(?:классе|класс)\b",
)

CHILD_CONTEXT_MARKERS = {
    "домашка",
    "домашнее задание",
    "мама сказала",
    "папа сказал",
    "учительница",
    "учитель сказал",
    "в школе",
}


def detect_child_context(text: str) -> bool:
    normalized = text.strip().lower()
    if not normalized:
        return False

    if any(re.search(pattern, normalized) for pattern in AGE_PATTERNS):
        return True

    marker_hits = sum(1 for marker in CHILD_CONTEXT_MARKERS if marker in normalized)
    if marker_hits >= 2:
        return True

    very_simple_question = len(normalized.split()) <= 8 and any(
        marker in normalized for marker in {"почему небо", "как сделать уроки", "что такое секс"}
    )
    return very_simple_question

