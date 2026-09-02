# FuckExam Screenshot

Фоновое приложение для создания полноэкранного скриншота по глобальному хоткею, OCR-распознавания текста, анализа распознанного текста через OpenRouter или Groq и отправки результата в Telegram.

## Pipeline

После нажатия хоткея приложение последовательно выполняет четыре шага: создаёт PNG-скриншот, распознаёт весь видимый текст через Tesseract, передаёт распознанный текст выбранной OpenAI-compatible LLM API и отправляет в Telegram ответ нейронки вместе с OCR-текстом.

| Этап | Реализация |
|---|---|
| Хоткей | `pynput`, по умолчанию `Ctrl+Alt+PrintScreen` |
| Wayland KDE | Spectacle, затем fallback на другие backend |
| Wayland GNOME | GNOME Screenshot, затем fallback на другие backend |
| Другой Wayland | `grim`, если compositor поддерживает `wlr-screencopy` |
| X11, Windows, macOS | `mss` |
| OCR | CLI `tesseract`, языки задаются в конфиге |
| LLM | OpenRouter или Groq через JSON API |
| Уведомление | Telegram Bot API с автоматическим разбиением длинных сообщений |
| Сеть | Прямое соединение, HTTP(S)- или SOCKS5-прокси |

## Установка

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

На Arch Linux установи Tesseract с русскими и английскими языковыми данными:

```bash
sudo pacman -S tesseract tesseract-data-rus tesseract-data-eng
```

Для Wayland также нужен один из backend захвата. Для KDE Plasma рекомендуется:

```bash
sudo pacman -S spectacle
```

Для GNOME:

```bash
sudo pacman -S gnome-screenshot
```

Для совместимых с `wlr-screencopy` композиторов:

```bash
sudo pacman -S grim
```

## Конфигурация

Рабочий `config.ini` намеренно не хранится в Git. Создай его из шаблона:

```bash
cp config.ini.example config.ini
$EDITOR config.ini
```

Шаблон содержит следующие основные параметры:

```ini
[screenshot]
hotkey = ctrl+alt+printscreen
output_dir = screenshots

[ocr]
command = tesseract
language = rus+eng
psm = 6

[llm]
provider = openrouter
model = openai/gpt-4o-mini
api_key = ВСТАВЬ_КЛЮЧ_ПРОВАЙДЕРА
temperature = 0.2
max_tokens = 2000
system_prompt = Ты полезный ассистент. Проанализируй распознанный текст со скриншота и дай краткий, точный ответ на русском языке.

[telegram]
bot_token = ВСТАВЬ_ТОКЕН_TELEGRAM-БОТА
chat_id = ВСТАВЬ_CHAT_ID
parse_mode = HTML
include_ocr_text = true

[network]
proxy_url =
timeout = 60
```

Для OpenRouter оставь `provider = openrouter`, укажи OpenRouter API key и выбранную модель. Для Groq укажи `provider = groq`, Groq API key и модель, доступную в твоём аккаунте. Поле `base_url` можно не заполнять: приложение само выберет стандартный endpoint провайдера. Оно поддерживает также явный пользовательский endpoint.

В `proxy_url` можно указать, например, `http://127.0.0.1:8080`, `socks5://127.0.0.1:1080` или `socks5h://127.0.0.1:1080`. Для SOCKS5 используется extra-зависимость `requests[socks]`, уже включённая в `requirements.txt`.

Telegram-бот должен иметь доступ к чату. `chat_id` можно получить через Telegram API или вспомогательного бота после отправки сообщения целевому боту. Значения `api_key`, `bot_token` и proxy credentials не выводятся в лог и не должны публиковаться.

## Запуск

```bash
python main.py
```

При нажатии `Ctrl+Alt+PrintScreen` приложение сохранит снимок в `screenshots/`, распознает текст, запросит ответ у выбранной модели и отправит в Telegram как ответ, так и весь распознанный текст. Для остановки нажми `Ctrl+C` в терминале.

## Проверка

```bash
python -m unittest discover -s tests -v
```

## Ограничения

Глобальный перехват клавиш через `pynput` в Wayland зависит от compositor и его политики безопасности. OCR выполняется локально через установленный Tesseract. Сетевые вызовы к LLM и Telegram выполняются последовательно после создания скриншота, поэтому во время обработки повторные нажатия пропускаются. Токены не передаются в репозиторий: файл `config.ini` добавлен в `.gitignore`.
