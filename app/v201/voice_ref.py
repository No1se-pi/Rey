"""Generate a stable, owned Rey voice reference using VoiceDesign."""
from __future__ import annotations

from dataclasses import replace

from .audio import QualityTTS
from .settings import VoiceSettings

REFERENCE_TEXT = (
    'Привет! Я Рей. Мне нравится, когда у нас получается что-то новое. '
    'Расскажи, что ты хочешь сделать сегодня, и мы вместе во всём разберёмся.'
)


def create_reference() -> None:
    settings = VoiceSettings.from_env()
    model = replace(
        settings, tts_mode='design',
        tts_model='Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign',
    )
    folder = settings.output_dir / 'reference'
    folder.mkdir(parents=True, exist_ok=True)
    audio = folder / 'rey_reference.wav'
    transcription = folder / 'rey_reference.txt'
    QualityTTS(model).render(REFERENCE_TEXT, audio)
    transcription.write_text(REFERENCE_TEXT, encoding='utf-8')
    print(f'Создан образец голоса: {audio}')
    print(f'Точная расшифровка: {transcription}')
    print('Прослушай образец. Если нравится, выбери REY_TTS_MODE=clone,')
    print('REY_TTS_MODEL=Qwen/Qwen3-TTS-12Hz-1.7B-Base')
    print(f'REY_TTS_REFERENCE_AUDIO={audio.resolve()}')
    print('REY_TTS_REFERENCE_TEXT=<содержимое rey_reference.txt>')
