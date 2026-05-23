from __future__ import annotations

import logging
from typing import Iterable

from app.avatar.state import AvatarState
from app.chat.session import ChatSession
from app.config import Settings, get_settings
from app.llm.ollama_client import OllamaClient
from app.llm.prompt_builder import PromptBuilder
from app.memory.memory_manager import MemoryManager
from app.memory.models import Memory, Message
from app.personality.child_safety_router import detect_child_context
from app.personality.complexity_router import is_complex_question
from app.personality.emotion_router import estimate_emotion
from app.personality.response_schema import ReiResponse, parse_rei_response


logger = logging.getLogger(__name__)

EMOTION_TO_HINT = {
    "warm": "soft_smile",
    "happy": "soft_smile",
    "sad": "concerned",
    "playful": "small_laugh",
    "thinking": "look_away",
    "confused": "tilt_head",
    "serious": "blink",
    "neutral": "none",
}


class ChatService:
    def __init__(
        self,
        *,
        settings: Settings | None = None,
        session: ChatSession | None = None,
        memory_manager: MemoryManager | None = None,
        llm_client: OllamaClient | None = None,
        prompt_builder: PromptBuilder | None = None,
        avatar_state: AvatarState | None = None,
    ) -> None:
        self.settings = settings or get_settings()
        self.session = session or ChatSession()
        self.memory_manager = memory_manager or MemoryManager()
        self.llm_client = llm_client or OllamaClient(settings=self.settings)
        self.prompt_builder = prompt_builder or PromptBuilder(max_history_messages=self.settings.max_history_messages)
        self.avatar_state = avatar_state or AvatarState()

    def handle_user_text(self, user_text: str, *, voice_mode: bool = False) -> ReiResponse:
        text = user_text.strip()
        if not text:
            return ReiResponse(text="Я здесь. Напиши что-нибудь, и разберёмся.", emotion="warm")

        self.avatar_state.set_listening()
        self.memory_manager.save_message(self.session.session_id, "user", text)

        child_context = self._detect_child_context(text)
        complex_question = is_complex_question(text)

        self.avatar_state.set_thinking()

        recent_messages = self.memory_manager.get_recent_messages(
            self.session.session_id,
            self.settings.max_history_messages,
        )
        memories = self.memory_manager.get_memories(limit=20) if self.settings.memory_enabled else []

        messages = self.prompt_builder.build_messages(
            user_text=text,
            recent_messages=recent_messages,
            memories=memories,
            child_context=child_context,
            complex_question=complex_question,
            voice_mode=voice_mode,
        )

        raw_response = self.llm_client.chat(messages)
        response = parse_rei_response(raw_response)
        self._enrich_response(response=response, user_text=text, complex_question=complex_question)

        logger.info("Structured Rei response: %s", response.to_dict())
        self.memory_manager.save_message(self.session.session_id, "assistant", response.text)
        if self.settings.memory_enabled:
            self.memory_manager.maybe_extract_memory(text, response)

        self.avatar_state.set_speaking(response.emotion, response.animation_hint)
        return response

    def _detect_child_context(self, user_text: str) -> bool:
        mode = self.settings.child_safe_mode.strip().lower()
        if mode in {"true", "1", "on", "always"}:
            return True
        if mode in {"false", "0", "off", "never"}:
            return False
        return detect_child_context(user_text)

    def _enrich_response(self, *, response: ReiResponse, user_text: str, complex_question: bool) -> None:
        routed_emotion = estimate_emotion(user_text, response.text)
        if response.emotion == "neutral" and routed_emotion != "neutral":
            response.emotion = routed_emotion
        if complex_question and response.emotion == "neutral":
            response.emotion = "thinking"
        response.action = "speaking"
        if response.animation_hint == "none":
            response.animation_hint = EMOTION_TO_HINT.get(response.emotion, "none")


class ChatLoop:
    def __init__(self, service: ChatService | None = None) -> None:
        self.service = service or ChatService()

    def run(self) -> None:
        self._write_intro()
        while True:
            try:
                user_text = input("Ты: ").strip()
            except (EOFError, KeyboardInterrupt):
                self.service.avatar_state.set_idle()
                print("\nРей: Ладно, свернёмся. Я рядом, когда снова понадоблюсь.")
                return

            if not user_text:
                continue

            if user_text.startswith("/"):
                should_continue = self._handle_command(user_text)
                if not should_continue:
                    return
                continue

            response = self.service.handle_user_text(user_text, voice_mode=False)
            print(f"Рей: {response.text}")
            self.service.avatar_state.set_idle()

    def _write_intro(self) -> None:
        print("Rei MVP CLI. Команды: /memory, /forget <id>, /history, /state, /voice, /text, /exit")
        print("Рей: Я на месте. Давай разбираться.")

    def _handle_command(self, command: str) -> bool:
        if command == "/exit":
            self.service.avatar_state.set_idle()
            print("Рей: До связи. Береги себя и свои странные прекрасные проекты.")
            return False
        if command == "/memory":
            self._show_memory()
            return True
        if command.startswith("/forget"):
            self._forget_memory(command)
            return True
        if command == "/history":
            self._show_history()
            return True
        if command == "/state":
            self._show_state()
            return True
        if command == "/voice":
            self._start_voice_mode()
            return True
        if command == "/text":
            print("Рей: Текстовый режим уже включён.")
            return True

        print("Рей: Не знаю такую команду. Доступны /memory, /forget <id>, /history, /state, /voice, /text, /exit.")
        return True

    def _show_memory(self) -> None:
        memories = self.service.memory_manager.get_memories(limit=50)
        if not memories:
            print("Рей: В памяти пока пусто.")
            return
        print("Память Рей:")
        for memory in memories:
            print(f"  #{memory.id} [{memory.key}] {memory.value} (importance={memory.importance})")

    def _forget_memory(self, command: str) -> None:
        parts = command.split(maxsplit=1)
        if len(parts) != 2 or not parts[1].strip().isdigit():
            print("Рей: Напиши так: /forget <id>")
            return
        memory_id = int(parts[1].strip())
        deleted = self.service.memory_manager.delete_memory(memory_id)
        if deleted:
            print(f"Рей: Удалила память #{memory_id}.")
        else:
            print(f"Рей: Не нашла память #{memory_id}.")

    def _show_history(self) -> None:
        messages = self.service.memory_manager.get_recent_messages(
            self.service.session.session_id,
            self.service.settings.max_history_messages,
        )
        if not messages:
            print("Рей: История этой сессии пока пустая.")
            return
        print("История сессии:")
        for message in messages:
            print(f"  {message.role}: {message.content}")

    def _show_state(self) -> None:
        state = self.service.avatar_state.to_dict()
        print(
            "Состояние Рей: "
            f"action={state['current_action']}, "
            f"emotion={state['current_emotion']}, "
            f"speaking={state['speaking']}, "
            f"animation_hint={state['animation_hint']}"
        )

    def _start_voice_mode(self) -> None:
        from app.voice.voice_loop import VoiceLoop

        voice_loop = VoiceLoop(chat_service=self.service)
        available, message = voice_loop.check_availability()
        if not available:
            print(f"Рей: {message}")
            return
        voice_loop.run()


def format_memories(memories: Iterable[Memory]) -> str:
    return "\n".join(f"#{memory.id} {memory.key}: {memory.value}" for memory in memories)


def format_messages(messages: Iterable[Message]) -> str:
    return "\n".join(f"{message.role}: {message.content}" for message in messages)
