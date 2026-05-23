from __future__ import annotations

from app.personality.complexity_router import is_complex_question


SADNESS_MARKERS = {
    "грустно",
    "плохо",
    "устал",
    "устала",
    "не получается",
    "я тупой",
    "я тупая",
    "ненавижу себя",
    "ничего не выходит",
    "одиноко",
    "страшно",
}

TECH_MARKERS = {
    "python",
    "api",
    "backend",
    "sqlite",
    "docker",
    "код",
    "ошибка",
    "тест",
    "сервер",
    "архитектура",
    "алгоритм",
    "llm",
    "модель",
}

PLAYFUL_MARKERS = {
    "шутка",
    "мем",
    "игра",
    "геймер",
    "квест",
    "dnd",
    "rpg",
    "пиво",
    "вселенн",
}


def estimate_emotion(user_text: str, assistant_text: str = "") -> str:
    combined = f"{user_text}\n{assistant_text}".lower()

    if any(marker in combined for marker in SADNESS_MARKERS):
        return "warm"

    if any(marker in combined for marker in PLAYFUL_MARKERS):
        return "playful"

    if is_complex_question(user_text):
        return "thinking"

    if any(marker in combined for marker in TECH_MARKERS):
        return "serious"

    return "neutral"

