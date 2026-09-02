#!/usr/bin/env bash
set -Eeuo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"

echo "=== FuckExam: установка для Linux ==="

if ! command -v python3 >/dev/null 2>&1; then
  echo "[ERROR] Не найден python3. Установи Python 3.10+ средствами своей ОС." >&2
  exit 1
fi

if [[ ! -x .venv/bin/python ]]; then
  echo "Создаю виртуальное окружение..."
  python3 -m venv .venv
fi

.venv/bin/python -m pip install --upgrade pip
.venv/bin/python -m pip install -r requirements.txt

if [[ ! -f config.ini ]]; then
  cp config.ini.example config.ini
  echo "Создан config.ini из config.ini.example. Заполни API-ключи и Telegram-параметры."
fi

if command -v tesseract >/dev/null 2>&1; then
  echo "Tesseract уже установлен: $(tesseract --version 2>&1 | head -n 1)"
elif command -v pacman >/dev/null 2>&1; then
  echo "Устанавливаю Tesseract и языки rus/eng через pacman..."
  sudo pacman -S --needed --noconfirm tesseract tesseract-data-rus tesseract-data-eng
elif command -v apt-get >/dev/null 2>&1; then
  echo "Устанавливаю Tesseract и языки rus/eng через apt..."
  sudo apt-get update
  sudo apt-get install -y tesseract-ocr tesseract-ocr-rus tesseract-ocr-eng
elif command -v dnf >/dev/null 2>&1; then
  echo "Устанавливаю Tesseract и языки rus/eng через dnf..."
  sudo dnf install -y tesseract tesseract-langpack-rus tesseract-langpack-eng
else
  echo "[WARN] Пакетный менеджер не распознан. Установи Tesseract и rus/eng вручную."
fi

if command -v tesseract >/dev/null 2>&1; then
  echo "Tesseract готов: $(tesseract --version 2>&1 | head -n 1)"
else
  echo "[WARN] tesseract не найден в PATH. OCR не заработает до его установки."
fi

echo
echo "Установка завершена. Перед запуском отредактируй config.ini."
