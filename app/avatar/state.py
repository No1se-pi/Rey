from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any


logger = logging.getLogger(__name__)


@dataclass
class AvatarState:
    emotion: str = "neutral"
    action: str = "idle"
    animation_hint: str = "none"
    is_speaking: bool = False
    gaze_target: str = "user"
    mouth_activity: float = 0.0
    last_updated: datetime = field(default_factory=lambda: datetime.now(timezone.utc))

    def _update(
        self,
        *,
        emotion: str | None = None,
        action: str | None = None,
        animation_hint: str | None = None,
        is_speaking: bool | None = None,
        mouth_activity: float | None = None,
        gaze_target: str | None = None,
    ) -> None:
        if emotion is not None:
            self.emotion = emotion
        if action is not None:
            self.action = action
        if animation_hint is not None:
            self.animation_hint = animation_hint
        if is_speaking is not None:
            self.is_speaking = is_speaking
        if mouth_activity is not None:
            self.mouth_activity = mouth_activity
        if gaze_target is not None:
            self.gaze_target = gaze_target
        self.last_updated = datetime.now(timezone.utc)
        logger.info("Avatar state changed: %s", self.to_dict())

    def set_listening(self) -> None:
        self._update(action="listening", is_speaking=False, mouth_activity=0.0, animation_hint="blink")

    def set_thinking(self) -> None:
        self._update(
            emotion="thinking",
            action="thinking",
            animation_hint="look_away",
            is_speaking=False,
            mouth_activity=0.0,
            gaze_target="away",
        )

    def set_speaking(self, emotion: str = "neutral", animation_hint: str = "none") -> None:
        self._update(
            emotion=emotion,
            action="speaking",
            animation_hint=animation_hint,
            is_speaking=True,
            mouth_activity=1.0,
            gaze_target="user",
        )

    def set_idle(self) -> None:
        self._update(action="idle", is_speaking=False, mouth_activity=0.0, gaze_target="user")

    def to_dict(self) -> dict[str, Any]:
        return {
            "current_emotion": self.emotion,
            "current_action": self.action,
            "emotion": self.emotion,
            "action": self.action,
            "animation_hint": self.animation_hint,
            "speaking": self.is_speaking,
            "is_speaking": self.is_speaking,
            "mouth_activity": self.mouth_activity,
            "gaze_target": self.gaze_target,
            "last_updated": self.last_updated.isoformat(),
        }

