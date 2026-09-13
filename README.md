# FuckExam

Тестовая desktop-версия наблюдателя VM с GUI в чёрно-красно-оранжевой палитре.

## Что есть в прототипе

- режим **HOST / VM** для запуска локальной трансляции;
- захват области VM и области окна приложения;
- live-превью с компоновкой «VM + APP»;
- локальная запись через FFmpeg в `FuckExamData/recordings`;
- режим **VIEWER** для подключения по IP и TCP-порту;
- односторонний чат `viewer → host`;
- SQLite база в `FuckExamData/fuckexam.sqlite3`;
- все данные создаются в текущей директории запуска.

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

Приложение намеренно не использует домашнюю директорию пользователя для runtime-данных. Папка `FuckExamData` создаётся рядом с местом запуска процесса. Поэтому для portable-режима нужно запускать exe/AppImage из папки, доступной для записи.

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

## Удалённый production-вариант

Текущий прототип intentionally использует простой TCP transport для локального теста. Для интернета нужен backend для комнат и токенов и WebRTC SFU; нельзя выставлять этот тестовый TCP-сервер напрямую в публичный интернет без TLS и авторизации.
