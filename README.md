# Rei

Rei / Рей - локальный AI companion: тёплая, спокойная, немного ироничная цифровая собеседница с памятью, эмоциями, состояниями аватара и заготовкой под голосовое общение.

MVP не делает Live2D/VRM и не притворяется полноценным человеком. Это CLI-приложение, которое общается через локальную Ollama-модель, хранит историю и избранные факты в SQLite, классифицирует эмоциональное состояние ответа и отдаёт состояние для будущего аватара.

## Возможности MVP

- текстовый чат с Рей через CLI;
- системный промпт личности;
- SQLite-история сообщений;
- простая долговременная память;
- команды `/memory`, `/forget <id>`, `/history`, `/state`, `/voices`, `/voice`, `/text`, `/exit`;
- structured response внутри программы: `text`, `emotion`, `action`, `animation_hint`, `memory_candidates`;
- состояние аватара: `idle`, `listening`, `thinking`, `speaking` и эмоции;
- эвристики сложности вопроса и детского контекста;
- голосовой режим через `faster-whisper` + `pyttsx3`, если установлены опциональные зависимости.

## Установка

```bash
python -m venv .venv
.venv\Scripts\activate
pip install -r requirements.txt
copy .env.example .env
```

Для голосового режима дополнительно:

```bash
pip install faster-whisper sounddevice scipy pyttsx3
```

## Ollama

Убедись, что Ollama установлена и запущена:

```bash
ollama serve
```

Скачай модель:

```bash
ollama pull qwen2.5:7b-instruct
```

Или выбери другую модель в `.env`:

```env
OLLAMA_MODEL=llama3.1:8b
```

## Запуск

```bash
python -m app.main
```

Первый запуск создаст SQLite-файл по пути из `.env`:

```env
DATABASE_URL=sqlite:///data/rei_memory.sqlite3
```

## Настройка `.env`

Основные параметры:

```env
OLLAMA_HOST=http://localhost:11434
OLLAMA_MODEL=qwen2.5:7b-instruct
MEMORY_ENABLED=true
MAX_HISTORY_MESSAGES=20
CHILD_SAFE_MODE=auto
VOICE_ENABLED=false
```

Голос:

```env
STT_MODEL_SIZE=small
STT_DEVICE=auto
STT_COMPUTE_TYPE=auto
TTS_ENGINE=pyttsx3
TTS_RATE=175
TTS_VOLUME=0.9
```

На Windows можно использовать более устойчивый backend:

```env
TTS_ENGINE=windows_sapi
TTS_VOICE_NAME=Microsoft Irina Desktop
TTS_RATE=155
```

## Команды CLI

- `/memory` - показать сохранённую память;
- `/forget <id>` - удалить запись памяти;
- `/history` - показать последние сообщения текущей сессии;
- `/state` - показать текущее состояние Рей для будущего аватара;
- `/voices` - показать доступные TTS-голоса;
- `/voice` - запустить голосовой режим;
- `/text` - вернуться к текстовому режиму;
- `/exit` - выйти.

## Память

Рей сохраняет факты только при явном сигнале, например:

- `запомни, что мой проект называется Rei`;
- `меня зовут Ярослав`;
- `я люблю Python`;
- `я работаю над локальным AI companion`.

Она не должна запоминать случайные одноразовые факты, пароли, токены, точный адрес, интимные темы, здоровье, политику, религию и другие чувствительные данные.

## Голосовой режим

Команда `/voice` проверяет зависимости. Если пакетов нет, текстовый чат продолжит работать, а Рей скажет, что нужно установить:

```bash
pip install faster-whisper sounddevice scipy pyttsx3
```

В MVP запись с микрофона идёт короткими фрагментами, затем текст отправляется в общий chat pipeline, ответ озвучивается через `pyttsx3`.

## Тесты

```bash
pytest
```

## Ограничения

- Эмоции, сложность и детский контекст пока определяются эвристиками.
- Память извлекается простыми правилами, без LLM-классификатора.
- Голосовой режим зависит от локального аудио-устройства и опциональных пакетов.
- Рей не является врачом, терапевтом, юристом или финансовым советником.
- Без запущенной Ollama текстовый интерфейс стартует, но модель не сможет ответить.

## Дальнейшие планы

- Live2D bridge;
- VRM/3D avatar bridge;
- vision-вход;
- long-term memory с приоритетами, дедупликацией и пересборкой фактов;
- отдельный emotion engine;
- web UI;
- замена TTS на Piper, XTTS, Kokoro или Orpheus;
- LLM-based классификация безопасности и памяти.

## Rey 2.0.1 — Voice & Telegram

Новый голосовой CLI и приватный Telegram-бот находятся в `app/v201/` и используют то же диалоговое ядро, личность и SQLite-память. Полная инструкция по установке, Qwen3-TTS, FFmpeg, Telegram, диагностике и smoke-тестам: [docs/REY-2.0.1.md](docs/REY-2.0.1.md).

Быстрый запуск после настройки `.env`:

```powershell
.\.venv\Scripts\python.exe -m app.v201 cli
.\.venv\Scripts\python.exe -m app.v201 telegram
```
