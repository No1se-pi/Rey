from __future__ import annotations

import logging

from app.chat.chat_loop import ChatService
from app.voice.errors import VoiceDependencyError
from app.voice.stt import SpeechToText
from app.voice.tts import TextToSpeech


logger = logging.getLogger(__name__)


class VoiceLoop:
    def __init__(self, chat_service: ChatService) -> None:
        self.chat_service = chat_service
        self.avatar_state = chat_service.avatar_state
        self.stt = SpeechToText(settings=chat_service.settings)
        self.tts = TextToSpeech(settings=chat_service.settings)

    def check_availability(self) -> tuple[bool, str]:
        stt_available, stt_message = self.stt.check_availability()
        tts_available, tts_message = self.tts.check_availability()
        if stt_available and tts_available:
            return True, "Голосовой режим доступен."
        messages = []
        if not stt_available:
            messages.append(stt_message)
        if not tts_available:
            messages.append(tts_message)
        return False, " ".join(messages)

    def run(self) -> None:
        print("Рей: Включаю голосовой режим. Говори после приглашения; Ctrl+C вернёт в текст.")
        while True:
            try:
                self.avatar_state.set_listening()
                print("Слушаю...")
                user_text = self.stt.transcribe_from_mic()
                if not user_text:
                    self.avatar_state.set_idle()
                    print("Рей: Не расслышала. Давай ещё раз.")
                    continue

                print(f"Ты: {user_text}")
                if user_text.strip().lower() in {"стоп", "выход", "выйти", "/text", "/exit"}:
                    self.avatar_state.set_idle()
                    print("Рей: Возвращаюсь в текстовый режим.")
                    return

                response = self.chat_service.handle_user_text(user_text, voice_mode=True)
                print(f"Рей: {response.text}")
                self.avatar_state.set_speaking(response.emotion, response.animation_hint)
                self.tts.speak(response.text)
                self.avatar_state.set_idle()
            except KeyboardInterrupt:
                self.avatar_state.set_idle()
                print("\nРей: Окей, голос выключен. Текстовый канал снова открыт.")
                return
            except VoiceDependencyError as exc:
                logger.exception("Voice dependency error.")
                self.avatar_state.set_idle()
                print(f"Рей: {exc}")
                return
            except Exception:
                logger.exception("Voice loop failed.")
                self.avatar_state.set_idle()
                print("Рей: Голосовой режим споткнулся. Я записала ошибку в лог, текстовый режим жив.")
                return

