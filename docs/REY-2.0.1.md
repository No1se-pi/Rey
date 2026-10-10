# Rey 2.0.1 — Voice & Telegram

Версия добавляет голосовой CLI и приватного Telegram-бота к существующему `ChatService`. Личность, Ollama, SQLite-история и долговременная память не дублируются. Rust и Rey Mesh в этот релиз не входят.

## Возможности и границы

- CLI: текст, `/mic [секунды]`, `/file <путь>`, текстовый `/text` и голосовой `/voice` режимы.
- Telegram/Telethon 1.x: текст, voice note и аудиовложения только в приватных разрешённых чатах от разрешённых User ID.
- STT: `faster-whisper`, русский язык, VAD, лимиты размера и длительности.
- TTS: Qwen3-TTS VoiceDesign или Base/VoiceClone, пакетный синтез с разбиением длинного текста.
- Один WAV синтезируется один раз, затем FFmpeg создаёт архивный MP3 и настоящий Telegram OGG/Opus.
- Тяжёлые операции сериализованы и выполняются вне event loop Telethon.
- Telegram не предназначен для публичного многопользовательского доступа: таблица долговременной памяти общая.

## Проверенная конфигурация

- Windows 10/11 x64;
- Python 3.11 (проверено) или совместимая поддерживаемая версия;
- NVIDIA RTX 3060 12 ГБ, актуальный драйвер;
- FFmpeg и ffprobe в `PATH` с `libmp3lame` и `libopus`;
- Ollama с моделью из `OLLAMA_MODEL`;
- свободное место для STT/TTS-моделей и аудиоархива.

Qwen3-TTS 1.7B и 7B LLM не следует держать в VRAM одновременно. По умолчанию Rey после ответа просит Ollama выгрузить LLM (`keep_alive=0`), а после синтеза освобождает TTS. Это медленнее, но безопаснее для 12 ГБ VRAM.

## Установка в PowerShell

Из корня репозитория:

```powershell
& 'C:\Program Files\Python311\python.exe' -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install --upgrade pip
python -m pip install -r requirements-2.0.1.txt
Copy-Item .env.example .env
```

Если PowerShell запрещает активацию, команды можно запускать напрямую через `.\.venv\Scripts\python.exe`.

### FFmpeg

Установите Windows-сборку FFmpeg с сайта, указанного на <https://ffmpeg.org/download.html>, добавьте каталог `bin` в `PATH` и проверьте:

```powershell
ffmpeg -version
ffprobe -version
ffmpeg -hide_banner -encoders | Select-String 'libmp3lame|libopus'
```

### Ollama

```powershell
ollama serve
ollama pull qwen2.5:7b-instruct
ollama list
```

В `.env` модель должна совпадать:

```dotenv
OLLAMA_HOST=http://localhost:11434
OLLAMA_MODEL=qwen2.5:7b-instruct
```

## Qwen3-TTS

TTS вынесен в отдельные зависимости, потому что PyTorch и модель занимают много места:

```powershell
.\.venv\Scripts\python.exe -m pip install -r requirements-2.0.1-tts.txt
```

На Windows обычный PyPI может установить CPU-only PyTorch. Для проверенной RTX 3060 с драйвером CUDA 13 установите CUDA-сборку после основных TTS-зависимостей:

```powershell
.\.venv\Scripts\python.exe -m pip install --upgrade --force-reinstall -r requirements-2.0.1-cuda.txt
.\.venv\Scripts\python.exe -c "import torch; print(torch.__version__, torch.version.cuda, torch.cuda.is_available())"
```

Установка пакета не является согласием на скачивание весов. По умолчанию:

```dotenv
REY_TTS_ALLOW_MODEL_DOWNLOAD=false
```

Кэш моделей по умолчанию находится в `data/model_cache` на том же диске, что и репозиторий. Если в Windows уже задан `HF_HOME`, Rey использует его, чтобы не скачивать веса повторно. Устаревший `TRANSFORMERS_CACHE` внутри процесса игнорируется: иначе основной Qwen и вложенный `speech_tokenizer` могут попасть в разные cache tree. Если такой разделённый кэш уже существует, Rey собирает из его файлов единое локальное представление через hard links и не дублирует многогигабайтные веса. При первом осознанном запуске разрешите скачивание модели:

```dotenv
REY_TTS_ALLOW_MODEL_DOWNLOAD=true
```

После появления модели в локальном кэше верните `false`, чтобы случайная смена имени модели не запускала новую многогигабайтную загрузку.

### VoiceDesign

```dotenv
REY_TTS_MODE=design
REY_TTS_MODEL=Qwen/Qwen3-TTS-12Hz-1.7B-VoiceDesign
REY_TTS_DEVICE=auto
```

VoiceDesign создаёт голос по инструкции, но тембр между отдельными запросами может немного меняться.

### Стабильный референс и VoiceClone

Сначала создайте оригинальный синтетический референс:

```powershell
.\.venv\Scripts\python.exe -m app.v201 voice-ref
```

Прослушайте `data/voice_output/reference/rey_reference.wav`. Если голос подходит, настройте Base-модель:

```dotenv
REY_TTS_MODE=clone
REY_TTS_MODEL=Qwen/Qwen3-TTS-12Hz-1.7B-Base
REY_TTS_REFERENCE_AUDIO=D:\полный\путь\rey_reference.wav
REY_TTS_REFERENCE_TEXT=Точная строка из rey_reference.txt
```

Rey один раз вычисляет clone prompt для загруженной модели и использует его повторно для всех сегментов. Для чужого голоса требуется разрешение владельца.

## Telegram через Telethon

1. Создайте бота у `@BotFather` и получите bot token.
2. Получите API ID и API Hash приложения на <https://my.telegram.org>.
3. Узнайте числовые User ID и private Chat ID.
4. Заполните `.env`, не публикуя его:

```dotenv
REY_TG_API_ID=123456
REY_TG_API_HASH=
REY_TG_BOT_TOKEN=
REY_TG_ALLOWED_IDS=123456789
REY_TG_ALLOWED_CHAT_IDS=123456789
```

Если `REY_TG_ALLOWED_CHAT_IDS` пуст, используется список `REY_TG_ALLOWED_IDS`. Без allowlist бот завершится с ошибкой (fail closed). Реальные секреты не логируются. Сессия Telethon хранится в `data/rey_bot.session` и исключена из Git.

Запуск:

```powershell
.\.venv\Scripts\python.exe -m app.v201 telegram
```

Бот использует `auto_reconnect`, обрабатывает один тяжёлый запрос за раз, ограничивает очередь через `REY_TG_MAX_PENDING_REQUESTS`, размер до `REY_MAX_AUDIO_BYTES` и длительность до `REY_MAX_AUDIO_SECONDS`. Реальные отправки нельзя выполнять из тестов.

## CLI

```powershell
.\.venv\Scripts\python.exe -m app.v201 cli
```

Команды:

- `/text` — ответы только текстом, TTS не нужен;
- `/voice` — включить генерацию аудио;
- `/mic 5` — записать пять секунд с микрофона;
- `/file "D:\audio\sample.ogg"` — распознать существующий файл;
- `/memory`, `/history`, `/forget <id>`, `/state` — совместимые команды памяти и состояния;
- `/exit` — выход.

Проверка микрофона без STT-модели:

```powershell
.\.venv\Scripts\python.exe scripts\smoke_v201.py --microphone
```

## Диагностика и smoke-тесты

Без инференса и скачивания моделей:

```powershell
.\.venv\Scripts\python.exe scripts\smoke_v201.py
.\.venv\Scripts\python.exe -m pytest -q --basetemp=.pytest-tmp
```

Отдельные явные проверки:

```powershell
# Уже установленная Ollama-модель
.\.venv\Scripts\python.exe scripts\smoke_v201.py --ollama

# STT: может впервые скачать выбранную Whisper-модель
.\.venv\Scripts\python.exe scripts\smoke_v201.py --audio "D:\audio\sample.wav"

# TTS: работает только с установленным qwen-tts и доступной локальной моделью
.\.venv\Scripts\python.exe scripts\smoke_v201.py --tts
```

Скрипт никогда не подключается к Telegram и не отправляет сообщений.

## Аудиофайлы и очистка

CLI сохраняет WAV, MP3 и OGG в `data/voice_output/`. Telegram-входы всегда обрабатываются во временном каталоге и удаляются. При `REY_KEEP_OUTPUT_AUDIO=true` Telegram-ответы архивируются там же; при `false` они удаляются после отправки. У каждого запроса уникальные имена, чтобы параллельные операции не повреждали файлы.

## Переменные окружения 2.0.1

| Переменная | Назначение | По умолчанию |
|---|---|---|
| `REY_TG_ALLOWED_IDS` | разрешённые User ID через запятую | обязательно для Telegram |
| `REY_TG_ALLOWED_CHAT_IDS` | разрешённые private Chat ID | как User ID |
| `REY_TG_SESSION_PATH` | путь сессии Telethon | `data/rey_bot` |
| `REY_TG_SEND_TEXT` / `REY_TG_SEND_MP3` | дополнительные ответы | `true` / `false` |
| `REY_TG_MAX_PENDING_REQUESTS` | активный запрос плюс очередь | `3` |
| `REY_TTS_MODE` | `design` или `clone` | `design` |
| `REY_TTS_MODEL` | Hugging Face ID или локальный путь | VoiceDesign 1.7B |
| `REY_TTS_DEVICE` | `auto`, `cuda`, `cuda:0`, `cpu` | `auto` |
| `REY_TTS_ALLOW_MODEL_DOWNLOAD` | разрешить загрузку весов | `false` |
| `REY_TTS_CPU_FALLBACK` | повторить TTS на CPU при CUDA-сбое | `false` |
| `REY_TTS_UNLOAD_AFTER_SYNTHESIS` | освобождать TTS после ответа | `true` |
| `REY_OLLAMA_UNLOAD_BEFORE_TTS` | выгружать LLM перед TTS | `true` |
| `REY_STT_MODEL` / `REY_STT_DEVICE` | Whisper-модель и устройство | `small` / `cpu` |
| `REY_STT_CPU_FALLBACK` | повторить STT на CPU при CUDA-сбое | `true` |
| `REY_STT_UNLOAD_AFTER_TRANSCRIPTION` | освобождать STT после входа | `true` |
| `REY_MAX_AUDIO_SECONDS` / `REY_MAX_AUDIO_BYTES` | лимиты входа | `180` / `15728640` |
| `REY_MAX_REPLY_CHARS` | максимум озвучиваемого текста | `1500` |
| `REY_TTS_CHUNK_CHARS` | размер сегмента TTS | `320` |
| `REY_MODEL_CACHE_DIR` | кэш Qwen/Whisper на диске проекта | `data/model_cache` |
| `REY_KEEP_OUTPUT_AUDIO` | хранить архив Telegram-ответов | `true` |
| `REY_CLI_VOICE_ENABLED` | стартовый режим CLI | `true` |
| `REY_FFMPEG_TIMEOUT_SECONDS` | таймаут одной конвертации | `120` |

Полный шаблон — `.env.example`; `.env.rey201.example` содержит только добавления релиза.

## CUDA OOM и типовые ошибки

- Проверьте `nvidia-smi` и `ollama ps`. На RTX 3060 оставляйте обе опции выгрузки включёнными.
- `Qwen3-TTS model is not cached locally` — модель отсутствует, а автоматическая загрузка запрещена.
- `CUDA out of memory` — закройте GPU-приложения, проверьте выгрузку Ollama; CPU fallback включайте сознательно, он очень медленный.
- `ffmpeg/ffprobe not found` — исправьте `PATH` и перезапустите PowerShell.
- пустое распознавание — проверьте микрофон, язык, уровень сигнала и длительность.
- Telegram молчит — проверьте оба allowlist, приватный чат и отсутствие дубликата процесса с той же session.

## Текущие ограничения и путь к Rey Mesh

- Реальный Qwen3-TTS, STT-модель и Telegram требуют отдельной ручной end-to-end проверки.
- VoiceDesign не гарантирует неизменный тембр; для идентичности используйте проверенный clone reference.
- Очередь находится в памяти одного процесса и не переживает перезапуск.
- Долговременная память общая для доверенных пользователей.
- Нет streaming TTS, wake word, Алисы, мобильного клиента и удалённых compute providers.

Дальнейшее развитие: вынести контракты `Transport -> Input Processing -> Dialogue Engine -> Output Processing -> Transport`, добавить отдельные идентичности/пространства памяти, устойчивую очередь, удалённый LLM/TTS provider и только затем Rust Node и синхронизацию Rey Mesh.
