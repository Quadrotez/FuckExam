# FuckExam

Тестовая desktop-версия наблюдателя VM с GUI в чёрно-красно-оранжевой палитре.

## Что есть в прототипе

- режим **HOST / VM** для запуска локальной трансляции;
- локальная запись через **OBS Studio** (`obs-websocket`, библиотека `obsws-python`) в `FuckExamData/recordings`;
- захват двух окон нативно: на Wayland — через `xdg-desktop-portal` + PipeWire (по portal restore token, без повторных диалогов), на X11/Windows — по заголовку окна;
- **live-превью в приложении** (~4 fps) с компоновкой «VM + APP» — это снапшот OBS-сцены, ровно то, что пишется в файл;
- трансляция тех же кадров соединённым зрителям (HostServer шлёт PNG);
- режим **VIEWER** для подключения по IP и TCP-порту;
- односторонний чат `viewer → host`;
- SQLite база в `FuckExamData/fuckexam.sqlite3`;
- все runtime-данные создаются в portable-каталоге рядом с исполняемым файлом.

При первом запуске открывается проверка окружения: дистрибутив, desktop session, Wayland/X11, compositor, VirtualBox, OBS, PipeWire, WirePlumber и XDG Desktop Portal. Для Arch Linux недостающие пакеты можно установить кнопкой через `pkexec`/`sudo`. После установки проверку нужно повторить.

Для записи нужен установленный `obs-studio` с плагином `obs-websocket` (входит в официальные сборки OBS 30+/31+ и в пакеты дистрибутивов), доступный в `PATH`. Приложение поднимает локальную запись через obs-websocket и при необходимости само включает websocket-сервер в конфиге OBS и запускает OBS в свёрнутом в трей режиме.

```bash
sudo pacman -S obs-studio
```

VirtualBox нужен, чтобы запускать саму экзаменационную ВМ: её окно — это и есть источник записи. Приложение не обращается к VirtualBox API и не требует `VBoxManage` для своей работы.

OBS захватывает два окна (VM и приложение) и обрабатывает оба через один видео-стрим. На Wayland источник OBS использует `xdg-desktop-portal` + PipeWire: системный диалог выбора источника показывается один раз, выбор запоминается (portal restore token) и переиспользуется в следующих сессиях. Если выбор окна отменён, приложение сообщает об ошибке записи.

Диагностика запуска и захвата сохраняется в `FuckExamData/fuckexam-runtime.log`; путь записанного MP4 отображается в интерфейсе.

В host-интерфейсе одна кнопка `START SESSION` запускает и трансляцию (с превью в приложении), и локальную запись; `STOP SESSION` останавливает всё вместе. Поля `VM window title` / `App window title` используются только на X11/Windows как заголовок окна для нативного захвата OBS; на Wayland достаточно выбрать окна в портале.

Это тестовый локальный транспорт для проверки UX. Он рассчитан на одного зрителя и не заменяет production WebRTC/SFU. Для дальнейшего релиза TCP-слой можно заменить на LiveKit Cloud, Cloudflare Realtime или self-hosted LiveKit.

## Запуск из исходников

```bash
python -m venv .venv
# Windows: .venv\\Scripts\\activate
# Linux/macOS: source .venv/bin/activate
python -m pip install -e .
python -m FuckExam
```

На машине ведущего нажмите **START SESSION**. На машине зрителя укажите IP ведущего и тот же порт, затем нажмите **CONNECT AS VIEWER**. Порт должен быть доступен в firewall ведущего.

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