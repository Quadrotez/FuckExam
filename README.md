# FuckExam Screenshot

Фоновое приложение для создания полноэкранного скриншота по глобальному хоткею, чтения изображения vision-моделью или OCR-распознавания текста, анализа содержимого через OpenRouter/Groq и отправки результата в Telegram.

## Pipeline

После нажатия хоткея приложение создаёт PNG-скриншот. В режиме `vision = auto` оно отправляет исходное изображение напрямую моделям, которые распознаются как vision-модели; это позволяет лучше читать мелкий текст, колонки и математические выражения. Для остальных моделей используется локальный Tesseract.

| Этап | Реализация |
|---|---|
| Хоткей | `pynput`, по умолчанию `Ctrl+Alt+PrintScreen` |
| Wayland | Spectacle, GNOME Screenshot или grim |
| X11, Windows, macOS | `mss` |
| Изображение в LLM | OpenAI-compatible `image_url` с data URI |
| OCR fallback | CLI `tesseract`, языки и `psm` задаются в конфиге |
| LLM | OpenRouter или Groq через JSON API |
| Уведомление | Telegram Bot API, включая ответ и OCR-текст |
| Сеть | Прямое соединение, HTTP(S)- или SOCKS5-прокси |

## Установка

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

На Arch Linux для OCR установи Tesseract и языковые данные:

```bash
sudo pacman -S tesseract tesseract-data-rus tesseract-data-eng
```

## Конфигурация

Рабочий `config.ini` не хранится в Git:

```bash
cp config.ini.example config.ini
$EDITOR config.ini
```

Ключевые параметры:

```ini
[llm]
provider = openrouter
model = openai/gpt-4o-mini
api_key = ВСТАВЬ_КЛЮЧ_ПРОВАЙДЕРА
# auto, always или never
vision = auto
# Дополнительные подстроки моделей с поддержкой изображений.
vision_models =
temperature = 0.2
max_tokens = 2000

[telegram]
bot_token = ВСТАВЬ_ТОКЕН_TELEGRAM-БОТА
chat_id = ВСТАВЬ_CHAT_ID
# Для длинного OCR лучше оставить пустым.
parse_mode =
include_ocr_text = true

[network]
proxy_url =
connect_timeout = 10
read_timeout = 60
```

Режимы `vision` работают так:

| Значение | Поведение |
|---|---|
| `auto` | Vision используется для известных моделей и моделей, совпавших с `vision_models`; иначе запускается Tesseract |
| `always` | Всегда отправляется исходная картинка. Ошибка API не маскируется OCR fallback |
| `never` | Всегда используется Tesseract |

В `vision_models` можно вручную добавить подстроку имени модели через запятую, например `my-custom-vision-model`. Если модель поддерживает изображения, но её название не распознаётся автоматически, укажи `vision = always` или добавь её в этот список.

Для OpenRouter оставь `provider = openrouter`, а для Groq используй `provider = groq`. Поле `base_url` можно не заполнять: приложение выберет стандартный endpoint провайдера. В `proxy_url` поддерживаются `http://`, `https://`, `socks5://` и `socks5h://`.

## Запуск

```bash
python main.py
```

При нажатии `Ctrl+Alt+PrintScreen` приложение сохранит снимок, передаст его vision-модели либо выполнит OCR fallback, запросит ответ и отправит в Telegram ответ вместе с полным локальным OCR-текстом, если он создавался. Для остановки нажми `Ctrl+C`.

Для автоматической установки можно использовать `install.sh`/`run.sh` на Linux либо `install.bat`/`run.bat` на Windows.

## Проверка

```bash
python -m unittest discover -s tests -v
```

## Ограничения

Глобальный перехват клавиш через `pynput` в Wayland зависит от compositor и его политики безопасности. Автоматическое определение vision основано на имени модели и может быть расширено через `vision_models`; для полной уверенности используй `vision = always`. OCR выполняется локально через Tesseract. Сетевые вызовы выполняются после создания скриншота, а повторные нажатия во время обработки пропускаются. Для диагностики используются этапные логи и конечные `connect_timeout`/`read_timeout`; бесконечного ожидания ответа быть не должно. Секреты не передаются в репозиторий: `config.ini` находится в `.gitignore`.

## Быстрая установка

```bash
# Linux
./install.sh
./run.sh
```

```bat
:: Windows
install.bat
run.bat
```
