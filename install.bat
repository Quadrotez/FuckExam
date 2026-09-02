@echo off
setlocal EnableExtensions
cd /d "%~dp0"

echo === FuckExam: установка для Windows ===
where py >nul 2>nul
if errorlevel 1 (
    echo [ERROR] Не найден Python Launcher ^(py^). Установи Python 3.10+ с https://www.python.org/downloads/windows/
    exit /b 1
)

if not exist ".venv\Scripts\python.exe" (
    echo Создаю виртуальное окружение...
    py -3 -m venv .venv
    if errorlevel 1 exit /b 1
)

call ".venv\Scripts\python.exe" -m pip install --upgrade pip
call ".venv\Scripts\python.exe" -m pip install -r requirements.txt
if errorlevel 1 (
    echo [ERROR] Не удалось установить Python-зависимости.
    exit /b 1
)

if not exist "config.ini" (
    copy /Y "config.ini.example" "config.ini" >nul
    echo Создан config.ini из config.ini.example. Заполни API-ключи и Telegram-параметры.
)

where winget >nul 2>nul
if errorlevel 1 (
    echo [WARN] winget не найден. Установи Tesseract вручную:
    echo        https://github.com/UB-Mannheim/tesseract/wiki
) else (
    echo Устанавливаю Tesseract OCR через winget...
    winget install --id UB-Mannheim.TesseractOCR -e --accept-source-agreements --accept-package-agreements
    if errorlevel 1 echo [WARN] winget не смог установить Tesseract. Проверь установку вручную.
)

if exist "%ProgramFiles%\Tesseract-OCR\tesseract.exe" (
    echo Tesseract найден в стандартном каталоге.
) else if exist "%LocalAppData%\Tesseract-OCR\tesseract.exe" (
    echo Tesseract найден в пользовательском каталоге.
) else (
    echo [WARN] Проверь, что tesseract.exe доступен в PATH после установки.
)

echo.
echo Установка завершена. Перед запуском отредактируй config.ini.
exit /b 0
