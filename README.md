# FuckExam

Тестовая desktop-версия наблюдателя VM с GUI в чёрно-красно-оранжевой палитре.

## Что есть в прототипе

- режим **HOST / VM** для запуска локальной трансляции;
- автоматический список зарегистрированных VirtualBox VM через `VBoxManage list vms`;
- захват конкретных окон по заголовку через `xdotool` на X11 или `hyprctl` на Hyprland Wayland;
- захват области VM и области окна приложения;
- live-превью с компоновкой «VM + APP»;
- локальная запись через FFmpeg в `FuckExamData/recordings`;
- режим **VIEWER** для подключения по IP и TCP-порту;
- односторонний чат `viewer → host`;
- SQLite база в `FuckExamData/fuckexam.sqlite3`;
- все runtime-данные создаются в portable-каталоге рядом с исполняемым файлом.

При первом запуске открывается проверка окружения: дистрибутив, desktop session, Wayland/X11, compositor, VirtualBox, FFmpeg, PipeWire, WirePlumber и XDG Desktop Portal. Для Arch Linux недостающие пакеты можно установить кнопкой через `pkexec`/`sudo`. После установки проверку нужно повторить.

Для записи нужен установленный `ffmpeg`, доступный в `PATH`. Если FFmpeg отсутствует или завершается с ошибкой кодека, приложение показывает причину вместо тихого создания пустого файла.

На Wayland для настоящей записи с системным уведомлением установите `gpu-screen-recorder`:

```bash
sudo pacman -S gpu-screen-recorder
```

Приложение использует `gpu-screen-recorder -w portal`: Wayland показывает системный диалог выбора источника, а запись идёт через xdg-desktop-portal + PipeWire. Для VM и приложения будут запрошены два источника. На Wayland приложение больше не использует тихий fallback на Pillow/grim для записи.

Диагностика запуска и захвата сохраняется в `FuckExamData/fuckexam-runtime.log`. Для каждого native recorder также создаётся `FuckExamData/recordings/wayland-recorder-*.log`; в нём фиксируются PID, output MP4, portal token и ошибки `gpu-screen-recorder`. Если окно не удалось определить через API compositor, приложение теперь не подменяет его всей fallback-областью: в preview будет сообщение об ошибке, а причина попадёт в runtime-log.

Для X11 установите `xdotool`, а для Hyprland Wayland нужен штатный `hyprctl`:

```bash
sudo pacman -S xdotool
```

В host-интерфейсе одна кнопка `START SESSION` запускает и трансляцию, и локальную запись; `STOP SESSION` останавливает всё вместе. `App window title` по умолчанию равен `FuckExam`; у VirtualBox используется имя выбранной VM. Поиск выполняется по части заголовка, поэтому в кадр попадают именно найденные окна, а не весь экран. Если compositor не предоставляет API геометрии окон, используются указанные fallback-координаты.

Это тестовый локальный транспорт для проверки UX. Он рассчитан на одного зрителя и не заменяет production WebRTC/SFU. Для дальнейшего релиза TCP-слой можно заменить на LiveKit Cloud, Cloudflare Realtime или self-hosted LiveKit.

## Запуск из исходников

```bash
python -m venv .venv
# Windows: .venv\\Scripts\\activate
# Linux/macOS: source .venv/bin/activate
python -m pip install -e .
python -m FuckExam
```

На машине ведущего нажмите **START BROADCAST**. На машине зрителя укажите IP ведущего и тот же порт, затем нажмите **CONNECT AS VIEWER**. Порт должен быть доступен в firewall ведущего.

Поля захвата задаются как `x,y,width,height`. Для теста оставлены безопасные значения по умолчанию; их можно поменять под расположение окон.

## Портативность

Приложение не использует домашнюю директорию пользователя для runtime-данных. Папка `FuckExamData` создаётся рядом с исполняемым файлом: для Windows one-folder build — рядом с `.exe`, для AppImage — рядом с `.AppImage`, а при прямом запуске `AppDir/AppRun` — рядом с `AppDir`. Каталог должен быть доступен для записи.

## Сборка

### Windows EXE

В PowerShell на Windows:

```powershell
Set-ExecutionPolicy -Scope Process Bypass
./build_exe.ps1
```

Результат: `dist/FuckExam/FuckExam.exe`. Это portable one-folder build; рядом с exe создаётся `FuckExamData`.

### Linux AppImage

На Linux:

```bash
chmod +x build_appimage.sh
./build_appimage.sh
```

Результат: `dist/FuckExam-x86_64.AppImage`. Скрипт использует PyInstaller и скачивает `appimagetool` только если его нет в `PATH` или `tools/`.

Одной командой из корня репозитория можно синхронизировать изменения, пересобрать и запустить последнюю версию:

```bash
git pull --ff-only origin master && rm -rf AppDir build dist && ./build_appimage.sh && exec ./dist/FuckExam-x86_64.AppImage
```

Либо используйте launcher из репозитория:

```bash
./update-and-run.sh
```

## Удалённый production-вариант

Текущий прототип intentionally использует простой TCP transport для локального теста. Для интернета нужен backend для комнат и токенов и WebRTC SFU; нельзя выставлять этот тестовый TCP-сервер напрямую в публичный интернет без TLS и авторизации.
