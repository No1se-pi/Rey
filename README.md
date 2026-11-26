# Rey 3.0 · Ghost & Shell

Нативный AI-компаньон на Rust для Windows и Linux. Rey Desktop объединяет диалог, голос, аватар, Markdown-память, шахматы и сёги. Ghost хранит личность и память. Shell управляет окнами, провайдерами, аудио и игровыми движками.

## Запуск

Установи [Rust](https://rustup.rs/) и выбери поставщик модели. Для Ollama используй [официальную установку](https://ollama.com/download). С существующим llama-server можно сразу выбрать llama.cpp. Выполняй команды из корня репозитория.

### Windows 10/11 x86-64

Для сборки установи [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) с компонентом **Desktop development with C++** и Windows SDK. PowerShell:

```powershell
if (-not (Test-Path .env)) { Copy-Item .env.example .env }
# Для Ollama
ollama pull qwen2.5:7b-instruct
cargo run --release --bin rey-desktop
```

При остановленной Ollama запусти `ollama serve` в отдельном терминале. Для кандзи установи японские дополнительные шрифты Windows. В игре можно переключиться на латинские обозначения.

### Linux x86-64

Для Debian/Ubuntu:

```bash
sudo apt install build-essential pkg-config libgl1-mesa-dev libxkbcommon-dev libxcb1-dev libx11-dev libxi-dev libxrandr-dev libwayland-dev fontconfig fonts-dejavu-core fonts-noto-cjk
test -e .env || cp .env.example .env
# Для Ollama
ollama pull qwen2.5:7b-instruct
cargo run --release --bin rey-desktop
```

Используй активный сеанс X11 или Wayland. Для диалога выбора файла на Linux требуется работающий XDG Desktop Portal с backend для рабочего стола. Для звука поддерживаются PulseAudio/PipeWire и ALSA. Если две команды Ollama разделены между терминалами, оставь `ollama serve` запущенной.

### Подключение llama.cpp

Если `llama-server` уже запущен, укажи в локальном `.env`:

```dotenv
REY_LLM_PROVIDER=llama_cpp
REY_LLAMA_CPP_HOST=http://127.0.0.1:8080
# При необходимости укажи имя модели и файл ключа, переданный llama-server через --api-key-file.
# REY_LLAMA_CPP_MODEL=имя-модели
# REY_LLAMA_CPP_API_KEY_FILE=/путь/к/файлу-ключа
```

Приложение поддерживает адрес как с суффиксом `/v1`, так и без него. Без явно заданной модели выбирается первая из списка сервера. Для инструментов памяти запусти llama-server с `--jinja` и шаблоном выбранной модели, который поддерживает вызов инструментов. Рей запрашивает обычные ответы без режима размышления. Переключить провайдер можно в «Настройках». [Документация llama-server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)

Если Ollama распакована в `data/ollama` на Linux, её можно запустить из корня проекта командой:

```bash
OLLAMA_HOST=127.0.0.1:11434 OLLAMA_MODELS="$PWD/data/ollama/models" OLLAMA_NO_CLOUD=1 ./data/ollama/bin/ollama serve
```

## Диалог и данные

Укажи адрес Ollama и модель в `.env` или в разделе «Настройки». Изменения в настройках Desktop действуют до закрытия окна. История и заметки сохраняются между запусками. Раздел «Память» поддерживает ссылки `[[123]]` по номеру заметки. Резервная копия доступна в разделе «Система». Историю диалога и заметки можно удалить из их разделов.

```bash
cargo run --bin rey -- ghost
cargo run --bin rey -- models
cargo run --bin rey -- chat
cargo run --bin rey -- chat --session <id>
cargo run --bin rey -- backup rey-backup.db
```

Память Rust по умолчанию находится в `ghost/memory.db`. Перед первым запуском с имеющейся базой Python выполни `cargo run --bin rey -- migrate`. Если новая база уже есть, импортируй в другой файл через `--to` и укажи его в `REY_DATABASE`. Снимок включает прежние идентификаторы, сообщения, факты и события. Подробности в [плане миграции](docs/MIGRATION.md).

Для удалённого провайдера автоматически скрываются история диалога и инструменты локальной памяти. `REY_ALLOW_REMOTE_MEMORY=true` явно разрешает их передачу. В удалённый голосовой worker уходят выбранное аудио или текст для озвучивания.

## Голос

Установи Python и FFmpeg. Worker ищет `ffmpeg`, `ffprobe` и `sox` в `data/bin/` и PATH. Используй одну виртуальную среду:

```bash
python -m venv .venv
# Linux
.venv/bin/python -m pip install -r requirements-2.0.1.txt
.venv/bin/python -m pip install -r requirements-2.0.1-tts.txt
```

В PowerShell выполни те же команды установки через `./.venv/Scripts/python.exe`. Другой интерпретатор задаётся через `REY_PYTHON`. Для RTX 3060, CUDA, параметров Qwen3-TTS, VoiceDesign и VoiceClone используй [инструкцию Rey 2.0.1](docs/REY-2.0.1.md). Настройки референса читаются из действующего `.env`.

В Desktop нажми «Микрофон» дважды, чтобы начать и закончить запись, или выбери аудиофайл. Флажок «Озвучивать» включает воспроизведение ответа. WAV, MP3 и OGG/Opus сохраняются в `REY_VOICE_OUTPUT_DIR`, если включён `REY_KEEP_OUTPUT_AUDIO`. CLI поддерживает `rey transcribe файл.wav` и `rey speak "Текст"`.

Подробности по IPC, удалённому запуску и Telegram описаны в [документации голосового worker](docs/VOICE.md).

## Аватар

Распакованный пакет v1.1 находится в `assets/avatar/`. Desktop подключает все 11 состояний, моргание и кадры рта. Для нового пакета достаточно заменить PNG и `manifest.json`, затем нажать «Перезагрузить ассеты аватара». Пути к кадрам относительны каталогу манифеста. Поддерживаются прозрачный PNG, массив `frames` с `fps`, `animations.blink` и `animations.speech`. При недоступном кадре используются `fallback_state`, `idle` или нейтральная заглушка. Формат и исходные сведения о пакете приведены в [описании ассетов](assets/PACK-v1.1.md).

## Игры

1. Скачай [Stockfish](https://github.com/official-stockfish/Stockfish/releases) и [YaneuraOu](https://github.com/yaneurao/YaneuraOu/releases) под свою платформу. Оба движка распространяются отдельно под GPLv3. Для YaneuraOu положи файлы оценки и книги в расположение, рекомендованное выбранной сборкой.
2. Готовые файлы `stockfish` и `YaneuraOu` можно положить в `data/engines/`, добавив `.exe` на Windows. Другие пути указываются в настройках или через `REY_CHESS_ENGINE` и `REY_SHOGI_ENGINE`. Поддерживаются и команды из PATH. Desktop проверяет наличие движка до начала партии. При сбое во время игры часы приостанавливаются до повтора расчёта.
3. В разделе «Игры» выбери сторону, часы, глубину поиска и режим. В режиме «Анализ» доступны ходы обеих сторон. Совет движка, чат о позиции, импорт и экспорт доступны на экране партии.

Для Linux на x86-64 с AVX2 можно собрать YaneuraOu V9.00 с материальной оценкой командой `make -j4 YANEURAOU_EDITION=YANEURAOU_ENGINE_MATERIAL TARGET_CPU=AVX2 COMPILER=g++` в каталоге `source`. Эта версия работает без скачивания весов. Подробности правил, форматов и ограничений в [документации игр](docs/GAMES.md).

## Проверки

```bash
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --release --workspace --locked
```

[Отчёт о реализации](docs/REY-3.0.md) · [История изменений](CHANGELOG.md) · [Rey 2.0.1 и Telegram](docs/REY-2.0.1.md)
