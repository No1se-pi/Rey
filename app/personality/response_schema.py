from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from typing import Any

logger = logging.getLogger(__name__)

VALID_EMOTIONS = {"warm", "neutral", "happy", "sad", "playful", "thinking", "confused", "serious"}
VALID_ACTIONS = {"idle", "listening", "thinking", "speaking"}
VALID_ANIMATION_HINTS = {
    "soft_smile",
    "look_away",
    "blink",
    "tilt_head",
    "small_laugh",
    "concerned",
    "none",
}


@dataclass
class ReiResponse:
    text: str
    emotion: str = "neutral"
    action: str = "speaking"
    animation_hint: str = "none"
    memory_candidates: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "text": self.text,
            "emotion": self.emotion,
            "action": self.action,
            "animation_hint": self.animation_hint,
            "memory_candidates": self.memory_candidates,
        }


def _extract_json_object(raw: str) -> str | None:
    cleaned = raw.strip()
    if cleaned.startswith("```"):
        lines = cleaned.splitlines()
        if lines and lines[0].startswith("```"):
            lines = lines[1:]
        if lines and lines[-1].strip() == "```":
            lines = lines[:-1]
        cleaned = "\n".join(lines).strip()

    if cleaned.startswith("{") and cleaned.endswith("}"):
        return cleaned

    start = cleaned.find("{")
    if start == -1:
        return None

    depth = 0
    in_string = False
    escape = False
    for index, char in enumerate(cleaned[start:], start=start):
        if escape:
            escape = False
            continue
        if char == "\\":
            escape = True
            continue
        if char == '"':
            in_string = not in_string
            continue
        if in_string:
            continue
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return cleaned[start : index + 1]
    return None


def _coerce_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [str(item).strip() for item in value if str(item).strip()]


def _plain_text_response(raw_response: str) -> ReiResponse:
    """Accept the legacy text footer without exposing its avatar controls."""
    lines = raw_response.strip().splitlines()
    if lines:
        footer = [part.strip().lower() for part in lines[-1].split("|")]
        if (
            len(footer) == 3
            and footer[0] in VALID_EMOTIONS
            and footer[1] in VALID_ANIMATION_HINTS
            and footer[2] in VALID_ACTIONS
        ):
            text = "\n".join(lines[:-1]).strip()
            if text:
                return ReiResponse(text=text, emotion=footer[0], animation_hint=footer[1], action=footer[2])
    return ReiResponse(text=raw_response.strip(), emotion="neutral", action="speaking")


def parse_rei_response(raw_response: str) -> ReiResponse:
    json_payload = _extract_json_object(raw_response)
    if json_payload is None:
        return _plain_text_response(raw_response)

    try:
        payload = json.loads(json_payload)
    except json.JSONDecodeError:
        logger.warning("Failed to parse Rei structured response; falling back to plain text.")
        return _plain_text_response(raw_response)

    if not isinstance(payload, dict):
        return _plain_text_response(raw_response)

    text = str(payload.get("text") or "").strip() or raw_response.strip()
    emotion = str(payload.get("emotion") or "neutral").strip().lower()
    action = str(payload.get("action") or "speaking").strip().lower()
    animation_hint = str(payload.get("animation_hint") or "none").strip().lower()

    if emotion not in VALID_EMOTIONS:
        emotion = "neutral"
    if action not in VALID_ACTIONS:
        action = "speaking"
    if animation_hint not in VALID_ANIMATION_HINTS:
        animation_hint = "none"

    return ReiResponse(
        text=text,
        emotion=emotion,
        action=action,
        animation_hint=animation_hint,
        memory_candidates=_coerce_list(payload.get("memory_candidates")),
    )

