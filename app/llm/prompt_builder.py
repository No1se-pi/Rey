from __future__ import annotations

from collections.abc import Iterable

from app.config import get_settings
from app.memory.models import Memory, Message
from app.personality.rei_system_prompt import REI_SYSTEM_PROMPT


RESPONSE_SCHEMA_INSTRUCTIONS = """
Формат ответа:
Верни строго один JSON-объект без markdown и без пояснений вокруг него.
Схема:
{
  "text": "естественный ответ Рей пользователю",
  "emotion": "warm|neutral|happy|sad|playful|thinking|confused|serious",
  "action": "idle|listening|thinking|speaking",
  "animation_hint": "soft_smile|look_away|blink|tilt_head|small_laugh|concerned|none",
  "memory_candidates": ["факт, который потенциально стоит запомнить"]
}
Поле text - это единственное, что увидит пользователь. Не упоминай JSON в text.
Не выдумывай сохранённую память. Если памяти недостаточно, не притворяйся, что помнишь.
""".strip()


class PromptBuilder:
    def __init__(self, max_history_messages: int | None = None) -> None:
        settings = get_settings()
        self.max_history_messages = max_history_messages or settings.max_history_messages

    def build_messages(
        self,
        *,
        user_text: str,
        recent_messages: Iterable[Message],
        memories: Iterable[Memory],
        child_context: bool,
        complex_question: bool,
        voice_mode: bool,
    ) -> list[dict[str, str]]:
        system_prompt = self._build_system_prompt(
            memories=memories,
            child_context=child_context,
            complex_question=complex_question,
            voice_mode=voice_mode,
        )

        history = list(recent_messages)[-self.max_history_messages :]
        if history and history[-1].role == "user" and history[-1].content.strip() == user_text.strip():
            history = history[:-1]

        messages = [{"role": "system", "content": system_prompt}]
        messages.extend({"role": item.role, "content": item.content} for item in history)
        messages.append({"role": "user", "content": user_text})
        return messages

    def _build_system_prompt(
        self,
        *,
        memories: Iterable[Memory],
        child_context: bool,
        complex_question: bool,
        voice_mode: bool,
    ) -> str:
        memory_block = self._format_memories(memories)
        runtime_flags = [
            f"child_context={child_context}",
            f"complex_question={complex_question}",
            f"voice_mode={voice_mode}",
        ]

        adaptive_instructions = []
        if child_context:
            adaptive_instructions.append(
                "Похоже, пользователь может быть ребёнком или подростком: отвечай проще, мягче, безопаснее, без грубого мата и без опасных инструкций."
            )
        if complex_question:
            adaptive_instructions.append(
                "Вопрос сложный: ответь вдумчиво, спокойно разложи идею и можешь начать с короткой фразы размышления."
            )
        if voice_mode:
            adaptive_instructions.append(
                "Это голосовой режим: делай text более разговорным, без слишком длинных списков и тяжёлой разметки."
            )

        if not adaptive_instructions:
            adaptive_instructions.append("Отвечай естественно, тепло и по делу.")

        return "\n\n".join(
            [
                REI_SYSTEM_PROMPT,
                "Сохранённая память:\n" + memory_block,
                "Текущие флаги: " + ", ".join(runtime_flags),
                "Адаптация ответа:\n- " + "\n- ".join(adaptive_instructions),
                RESPONSE_SCHEMA_INSTRUCTIONS,
            ]
        )

    def _format_memories(self, memories: Iterable[Memory]) -> str:
        items = list(memories)
        if not items:
            return "Память пока пуста."
        return "\n".join(f"- #{memory.id} {memory.key}: {memory.value}" for memory in items)

