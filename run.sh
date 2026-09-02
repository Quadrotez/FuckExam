#!/usr/bin/env bash
set -Eeuo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"

echo "=== FuckExam: запуск ==="
if [[ ! -x .venv/bin/python ]]; then
  echo "[ERROR] Виртуальное окружение не найдено. Сначала запусти ./install.sh." >&2
  exit 1
fi
if [[ ! -f config.ini ]]; then
  echo "[ERROR] Не найден config.ini. Сначала запусти ./install.sh и заполни конфигурацию." >&2
  exit 1
fi

source .venv/bin/activate
exec python main.py
