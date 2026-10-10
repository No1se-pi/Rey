from app.personality.response_schema import parse_rei_response


def test_legacy_avatar_footer_is_not_exposed_as_reply_text():
    response = parse_rei_response(
        "Всё отлично, спасибо за интерес!\n\nthinking|soft_smile|listening"
    )

    assert response.text == "Всё отлично, спасибо за интерес!"
    assert response.emotion == "thinking"
    assert response.animation_hint == "soft_smile"
    assert response.action == "listening"


def test_ordinary_pipe_text_is_preserved():
    response = parse_rei_response("Варианты: один|два|три")

    assert response.text == "Варианты: один|два|три"
