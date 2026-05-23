from app.personality.emotion_router import estimate_emotion


def test_sad_user_gets_warm_emotion():
    assert estimate_emotion("У меня ничего не получается", "") == "warm"


def test_game_topic_gets_playful_emotion():
    assert estimate_emotion("Давай обсудим RPG и странный квест", "") == "playful"


def test_technical_topic_gets_serious_emotion():
    assert estimate_emotion("Почему падает Python API сервер?", "") in {"thinking", "serious"}


def test_default_emotion_is_neutral():
    assert estimate_emotion("Привет", "Привет") == "neutral"

