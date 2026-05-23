from app.llm.prompt_builder import PromptBuilder
from app.memory.models import Memory, Message
from app.personality.rei_system_prompt import REI_SYSTEM_PROMPT


def test_prompt_contains_system_memory_and_user_text():
    builder = PromptBuilder(max_history_messages=5)
    memories = [
        Memory(
            id=1,
            key="user_name",
            value="Ярослав",
            source_message="меня зовут Ярослав",
            importance=3,
            created_at="now",
            updated_at="now",
        )
    ]
    recent_messages = [
        Message(id=1, session_id="s1", role="user", content="Привет", created_at="now"),
        Message(id=2, session_id="s1", role="assistant", content="Привет, я рядом.", created_at="now"),
    ]

    messages = builder.build_messages(
        user_text="Что ты обо мне помнишь?",
        recent_messages=recent_messages,
        memories=memories,
        child_context=False,
        complex_question=False,
        voice_mode=False,
    )

    assert messages[0]["role"] == "system"
    assert REI_SYSTEM_PROMPT in messages[0]["content"]
    assert "Ярослав" in messages[0]["content"]
    assert messages[-1] == {"role": "user", "content": "Что ты обо мне помнишь?"}
    assert "JSON" in messages[0]["content"]

