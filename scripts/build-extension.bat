@echo off
setlocal
cd /d "%~dp0.."
python scripts\build_extension.py
endlocal
