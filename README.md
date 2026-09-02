# FuckExam Screenshot

Небольшой прототип фонового приложения, которое делает полноэкранный скриншот по глобальному сочетанию клавиш.

## Возможности

Программа загружает сочетание клавиш из `config.ini`, ждёт его в фоне и сохраняет каждый снимок в отдельный PNG-файл. Повторное срабатывание при удержании клавиш заблокировано: следующий снимок создаётся только после отпускания хотя бы одной клавиши комбинации.

| Окружение | Backend | Требование |
|---|---|---|
| Wayland на KDE Plasma | `spectacle` | Установленный KDE Spectacle |
| Wayland на GNOME | `gnome-screenshot` | Установленный GNOME Screenshot |
| Другой Wayland-композитор | `grim` | Композитор с поддержкой `wlr-screencopy` |
| X11, Windows, macOS | `mss` | Python-зависимость из `requirements.txt` |

## Установка

```bash
python -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt
```

Для KDE Plasma установи Spectacle. В Arch Linux:

```bash
sudo pacman -S spectacle
```

Для GNOME можно установить `gnome-screenshot`, а для совместимых с `wlr-screencopy` композиторов — `grim`:

```bash
sudo pacman -S gnome-screenshot grim
```

## Запуск

```bash
python main.py
```

По умолчанию используется `Ctrl+Shift+S`, а файлы сохраняются в каталог `screenshots/`. Для остановки приложения нажми `Ctrl+C` в терминале.

На Wayland backend выбирается автоматически: Spectacle для KDE, GNOME Screenshot для GNOME, затем `grim`. Поэтому в KDE Plasma `grim` обычно не нужен.

## Настройка

Измени `config.ini`:

```ini
[screenshot]
hotkey = ctrl+shift+s
output_dir = screenshots
```

Поддерживаются одиночные клавиши, например `g`, и комбинации вроде `ctrl+alt+g`. Для специальных клавиш используются имена `ctrl`, `shift`, `alt`, `cmd`, `enter`, `space` и другие имена из `pynput`.

## Ограничения прототипа

На Wayland глобальный перехват клавиш через `pynput` зависит от используемого compositor и может быть ограничен политикой безопасности. Backend захвата выбирается отдельно от обработчика клавиш: для KDE используется Spectacle, для GNOME — GNOME Screenshot, для совместимых композиторов — `grim`. Следующим этапом можно добавить нативную регистрацию глобальных шорткатов для конкретного рабочего окружения, например KDE или GNOME.

Если выбранный backend не установлен или завершился с ошибкой, приложение выводит сообщение в лог и пробует следующий доступный backend. Если все backend недоступны, причина каждой попытки также попадает в лог.

## Проверка

```bash
python -m unittest discover -s tests -v
```
