# Голос и Telegram

Rust запускает `python -m providers.python_inference transcribe <файл>` или `python -m providers.python_inference synthesize <каталог>`. Для синтеза stdin содержит UTF‑8 JSON с полем `text`. Worker возвращает JSON в stdout, диагностику в stderr и три файла в выделенной папке. Faster‑Whisper и Qwen3‑TTS используют настройки Rey 2.0.1. Локальный worker завершается после операции и освобождает ресурсы модели. В одном экземпляре Rey запросы к worker выполняются последовательно.

Для постоянного сервера задай `REY_VOICE_TOKEN` длиной от 16 байт и запусти:

```bash
python -m providers.python_inference serve --port 8722
```

Сервер слушает `127.0.0.1`. Для доступа с другой машины размести его за HTTPS-прокси с ограничением размера запросов. На клиенте задай `REY_VOICE_URL=https://<сервер>` и тот же `REY_VOICE_TOKEN`. `POST /v1/transcribe` принимает аудиофайл как тело запроса. `POST /v1/synthesize` принимает JSON с полем `text` и возвращает JSON с Base64-полями `wav`, `mp3` и `opus`. Оба маршрута проверяют Bearer-токен. Отмена в Desktop прекращает ожидание HTTP-ответа. Запущенный удалённый инференс освобождает ресурсы по завершении стороны сервера.

Telethon остаётся отдельным адаптером Rey 2.0.1. Запуск и allowlist описаны в [документации прежнего адаптера](REY-2.0.1.md). Он использует прежний Python-пайплайн и прежнюю SQLite-БД. Оба поколения поддерживают OGG/Opus и MP3.
