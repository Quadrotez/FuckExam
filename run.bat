@echo off
setlocal EnableExtensions
cd /d "%~dp0"

echo === FuckExam: запуск ===
if not exist ".venv\Scripts\python.exe" (
    echo [ERROR] Виртуальное окружение не найдено. Сначала запусти install.bat.
    exit /b 1
)
if not exist "config.ini" (
    echo [ERROR] Не найден config.ini. Сначала запусти install.bat и заполни конфигурацию.
    exit /b 1
)

if exist "%ProgramFiles%\Tesseract-OCR" set "PATH=%ProgramFiles%\Tesseract-OCR;%PATH%"
if exist "%LocalAppData%\Tesseract-OCR" set "PATH=%LocalAppData%\Tesseract-OCR;%PATH%"

call ".venv\Scripts\activate.bat"
python main.py
set "EXIT_CODE=%ERRORLEVEL%"
call ".venv\Scripts\deactivate.bat" >nul 2>nul
exit /b %EXIT_CODE%
