from __future__ import annotations

import re


COMPLEX_KEYWORDS = {
    "архитектура",
    "как лучше",
    "почему",
    "сравни",
    "объясни",
    "безопасность",
    "проект",
    "алгоритм",
    "анализ",
    "спроектируй",
    "дизайн системы",
    "system design",
    "план",
    "стратегия",
    "оптимиз",
    "масштаб",
    "интеграц",
}

PLAN_PATTERNS = (
    r"\bпошагов",
    r"\bразложи\b",
    r"\bпроанализ",
    r"\bсоставь план\b",
    r"\bчто выбрать\b",
    r"\bкакие варианты\b",
)


def is_complex_question(text: str) -> bool:
    normalized = text.strip().lower()
    if not normalized:
        return False

    words = re.findall(r"\w+", normalized, flags=re.UNICODE)
    if len(words) >= 28 or len(normalized) >= 180:
        return True

    if any(keyword in normalized for keyword in COMPLEX_KEYWORDS):
        return True

    if any(re.search(pattern, normalized) for pattern in PLAN_PATTERNS):
        return True

    condition_markers = normalized.count(" если ") + normalized.count(" но ") + normalized.count(" при этом ")
    punctuation_markers = normalized.count(",") + normalized.count(";") + normalized.count(":")
    return condition_markers >= 2 or punctuation_markers >= 4

