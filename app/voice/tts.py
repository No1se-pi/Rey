from __future__ import annotations

import importlib.util
import logging
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from app.config import Settings, get_settings
from app.voice.errors import VoiceDependencyError


logger = logging.getLogger(__name__)


class TextToSpeech:
    def __init__(self, settings: Settings | None = None) -> None:
        self.settings = settings or get_settings()
        self._engine = None

    def check_availability(self) -> tuple[bool, str]:
        if self.settings.tts_engine == "windows_sapi":
            if self._powershell_executable() is None:
                return False, "Для windows_sapi нужен powershell в PATH."
            return True, "TTS доступен через Windows SAPI."
        if self.settings.tts_engine != "pyttsx3":
            return False, f"TTS_ENGINE={self.settings.tts_engine} пока не поддержан в MVP. Доступны pyttsx3 и windows_sapi."
        if importlib.util.find_spec("pyttsx3") is None:
            return False, "Для озвучки поставь пакет: pyttsx3"
        return True, "TTS доступен."

    def speak(self, text: str) -> None:
        available, message = self.check_availability()
        if not available:
            raise VoiceDependencyError(message)
        if not text.strip():
            return

        logger.info("TTS started text_length=%s", len(text))
        if self.settings.tts_engine == "windows_sapi":
            self._speak_windows_sapi(text)
        else:
            self._speak_pyttsx3(text)
        logger.info("TTS finished")

    def list_voices(self) -> list[dict[str, str]]:
        if self.settings.tts_engine == "windows_sapi":
            return self._list_windows_sapi_voices()
        return self._list_pyttsx3_voices()

    def _speak_pyttsx3(self, text: str) -> None:
        engine = self._get_engine()
        for chunk in self._split_text(text):
            engine.say(chunk)
            engine.runAndWait()
            engine.stop()

    def _get_engine(self):
        if self._engine is not None:
            return self._engine

        import pyttsx3

        engine = pyttsx3.init()
        engine.setProperty("rate", self.settings.tts_rate)
        engine.setProperty("volume", self.settings.tts_volume)
        if self.settings.tts_voice_name:
            self._select_voice(engine, self._normalize_voice_name(self.settings.tts_voice_name))
        self._engine = engine
        return self._engine

    def _speak_windows_sapi(self, text: str) -> None:
        powershell = self._powershell_executable()
        if powershell is None:
            raise VoiceDependencyError("Для windows_sapi нужен powershell в PATH.")

        with tempfile.NamedTemporaryFile("w", suffix=".txt", encoding="utf-8", delete=False) as temp_file:
            temp_file.write(text)
            text_path = Path(temp_file.name)

        script_path: Path | None = None
        script = r"""
param(
    [string]$TextPath,
    [string]$VoiceName,
    [int]$Rate,
    [int]$Volume
)
Add-Type -AssemblyName System.Speech
$speaker = New-Object System.Speech.Synthesis.SpeechSynthesizer
$speaker.Rate = [Math]::Max(-10, [Math]::Min(10, $Rate))
$speaker.Volume = [Math]::Max(0, [Math]::Min(100, $Volume))
if (-not [string]::IsNullOrWhiteSpace($VoiceName)) {
    $speaker.SelectVoice($VoiceName)
} else {
    $ruVoice = $speaker.GetInstalledVoices() |
        Where-Object { $_.VoiceInfo.Culture.Name -eq 'ru-RU' } |
        Select-Object -First 1
    if ($ruVoice -ne $null) {
        $speaker.SelectVoice($ruVoice.VoiceInfo.Name)
    }
}
$text = Get-Content -LiteralPath $TextPath -Raw -Encoding UTF8
$speaker.Speak($text)
$speaker.Dispose()
"""
        try:
            with tempfile.NamedTemporaryFile("w", suffix=".ps1", encoding="utf-8", delete=False) as script_file:
                script_file.write(script)
                script_path = Path(script_file.name)

            completed = subprocess.run(
                [
                    powershell,
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    str(script_path),
                    "-TextPath",
                    str(text_path),
                    "-VoiceName",
                    self._normalize_voice_name(self.settings.tts_voice_name),
                    "-Rate",
                    str(self._sapi_rate()),
                    "-Volume",
                    str(self._sapi_volume()),
                ],
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=max(20, len(text) // 8),
            )
            if completed.returncode != 0:
                logger.error("Windows SAPI TTS failed: %s", completed.stderr.strip())
                raise VoiceDependencyError("Windows SAPI не смог озвучить ответ. Подробности в логе.")
        finally:
            try:
                text_path.unlink(missing_ok=True)
            except OSError:
                logger.warning("Failed to remove temporary TTS text file: %s", text_path)
            if script_path is not None:
                try:
                    script_path.unlink(missing_ok=True)
                except OSError:
                    logger.warning("Failed to remove temporary TTS script file: %s", script_path)

    def _select_voice(self, engine, voice_name: str) -> None:
        desired = voice_name.lower()
        for voice in engine.getProperty("voices"):
            voice_id = str(getattr(voice, "id", "")).lower()
            name = str(getattr(voice, "name", "")).lower()
            if desired in voice_id or desired in name:
                engine.setProperty("voice", voice.id)
                return
        logger.warning("Requested TTS voice not found: %s", voice_name)

    def _list_pyttsx3_voices(self) -> list[dict[str, str]]:
        engine = self._get_engine()
        result: list[dict[str, str]] = []
        for voice in engine.getProperty("voices"):
            result.append(
                {
                    "name": str(getattr(voice, "name", "")),
                    "id": str(getattr(voice, "id", "")),
                    "language": ",".join(str(item) for item in getattr(voice, "languages", []) or []),
                }
            )
        return result

    def _list_windows_sapi_voices(self) -> list[dict[str, str]]:
        powershell = self._powershell_executable()
        if powershell is None:
            return []

        script = r"""
Add-Type -AssemblyName System.Speech
$speaker = New-Object System.Speech.Synthesis.SpeechSynthesizer
$speaker.GetInstalledVoices() | ForEach-Object {
    $v = $_.VoiceInfo
    "$($v.Name)|$($v.Culture.Name)|$($v.Gender)|$($v.Age)"
}
$speaker.Dispose()
"""
        completed = subprocess.run(
            [powershell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=10,
        )
        if completed.returncode != 0:
            logger.error("Failed to list Windows SAPI voices: %s", completed.stderr.strip())
            return []

        voices = []
        for line in completed.stdout.splitlines():
            name, culture, gender, age = (line.split("|") + ["", "", "", ""])[:4]
            voices.append({"name": name, "id": name, "language": culture, "gender": gender, "age": age})
        return voices

    def _split_text(self, text: str) -> list[str]:
        chunks = [chunk.strip() for chunk in text.replace("\r", "\n").split("\n") if chunk.strip()]
        return chunks or [text.strip()]

    def _sapi_rate(self) -> int:
        return max(-10, min(10, round((self.settings.tts_rate - 175) / 20)))

    def _sapi_volume(self) -> int:
        return max(0, min(100, round(self.settings.tts_volume * 100)))

    def _powershell_executable(self) -> str | None:
        return shutil.which("powershell") or shutil.which("pwsh")

    def _normalize_voice_name(self, voice_name: str) -> str:
        cleaned = re.sub(r"\s+\[[^\]]+\].*$", "", voice_name.strip())
        return re.sub(r"\s+", " ", cleaned)
