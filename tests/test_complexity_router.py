from app.personality.complexity_router import is_complex_question


def test_complex_question_by_keyword():
    assert is_complex_question("Как лучше спроектировать архитектуру памяти для AI companion?")


def test_complex_question_by_length():
    text = (
        "Объясни, пожалуйста, почему в проекте с несколькими сервисами, локальной базой, "
        "голосовым вводом и LLM нужно разделять слои, и как потом это тестировать"
    )
    assert is_complex_question(text)


def test_simple_question_is_not_complex():
    assert not is_complex_question("Как дела?")

