from __future__ import annotations

import configparser
import logging
import os
import platform
import shutil
import subprocess
import sys
import threading
from datetime import datetime
from pathlib import Path
from typing import Iterable

from pynput import keyboard


ROOT = Path(__file__).resolve().parent
CONFIG_FILE = ROOT / "config.ini"
LOGGER = logging.getLogger("fuckexam-screenshot")

KEY_ALIASES = {
    "control": "ctrl",
    "ctrl_l": "ctrl",
    "ctrl_r": "ctrl",
    "shift_l": "shift",
    "shift_r": "shift",
    "alt_l": "alt",
    "alt_r": "alt",
    "cmd": "cmd",
    "cmd_l": "cmd",
    "cmd_r": "cmd",
    "win": "cmd",
    "super": "cmd",
    "super_l": "cmd",
    "super_r": "cmd",
}


def load_config(path: Path = CONFIG_FILE) -> tuple[list[str], Path]:
    """Load the hotkey and output directory from config.ini."""
    parser = configparser.ConfigParser()
    parser.read(path, encoding="utf-8")

    hotkey = parser.get("screenshot", "hotkey", fallback="ctrl+shift+s")
    output_dir = parser.get("screenshot", "output_dir", fallback="screenshots")
    keys = [normalize_name(part) for part in hotkey.split("+") if part.strip()]

    if not keys:
        raise ValueError("В config.ini не задана ни одна клавиша в hotkey")
    if len(set(keys)) != len(keys):
        raise ValueError("В hotkey не должно быть повторяющихся клавиш")

    output_path = Path(output_dir).expanduser()
    if not output_path.is_absolute():
        output_path = path.resolve().parent / output_path
    return keys, output_path


def normalize_name(value: str) -> str:
    """Normalize config and pynput names to a common representation."""
    name = value.strip().lower().replace(" ", "_")
    return KEY_ALIASES.get(name, name)


def normalize_key(key: keyboard.Key | keyboard.KeyCode) -> str | None:
    if isinstance(key, keyboard.KeyCode):
        return normalize_name(key.char) if key.char else None
    return normalize_name(str(key).removeprefix("Key."))


def is_wayland() -> bool:
    return platform.system() == "Linux" and os.environ.get("XDG_SESSION_TYPE", "").lower() == "wayland"


def make_filename(output_dir: Path) -> Path:
    timestamp = datetime.now().astimezone().strftime("%Y-%m-%d_%H-%M-%S_%f")[:-3]
    return output_dir / f"screenshot_{timestamp}.png"


def screenshot_with_grim(destination: Path) -> None:
    if shutil.which("grim") is None:
        raise RuntimeError(
            "В Wayland-сессии не найден grim. Установи его пакетным менеджером "
            "своего дистрибутива и повтори запуск."
        )
    result = subprocess.run(
        ["grim", str(destination)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        details = result.stderr.strip() or result.stdout.strip() or "нет подробностей"
        raise RuntimeError(f"grim завершился с кодом {result.returncode}: {details}")


def screenshot_with_mss(destination: Path) -> None:
    try:
        from mss import mss
        from mss.tools import to_png
    except ImportError as error:
        raise RuntimeError("Не установлен пакет mss. Выполни pip install -r requirements.txt") from error

    with mss() as capture:
        monitor = capture.monitors[0]
        image = capture.grab(monitor)
        to_png(image.rgb, image.size, output=str(destination))


def take_screenshot(output_dir: Path) -> Path:
    output_dir.mkdir(parents=True, exist_ok=True)
    destination = make_filename(output_dir)
    if is_wayland():
        screenshot_with_grim(destination)
    else:
        screenshot_with_mss(destination)
    return destination


class HotkeyScreenshotApp:
    def __init__(self, hotkey: Iterable[str], output_dir: Path) -> None:
        self.hotkey = set(hotkey)
        self.output_dir = output_dir
        self.pressed: set[str] = set()
        self.triggered = False
        self._screenshot_lock = threading.Lock()

    def on_press(self, key: keyboard.Key | keyboard.KeyCode) -> None:
        normalized = normalize_key(key)
        if normalized is None:
            return
        self.pressed.add(normalized)
        if not self.triggered and self.hotkey.issubset(self.pressed):
            self.triggered = True
            self._run_screenshot()

    def on_release(self, key: keyboard.Key | keyboard.KeyCode) -> None:
        normalized = normalize_key(key)
        if normalized is not None:
            self.pressed.discard(normalized)
        if not self.hotkey.issubset(self.pressed):
            self.triggered = False

    def _run_screenshot(self) -> None:
        if not self._screenshot_lock.acquire(blocking=False):
            LOGGER.warning("Скриншот уже создаётся, повторное срабатывание пропущено")
            return
        try:
            LOGGER.info("Создаю скриншот...")
            destination = take_screenshot(self.output_dir)
            LOGGER.info("Скриншот сохранён: %s", destination)
        except Exception as error:  # noqa: BLE001 - ошибка должна быть видна пользователю
            LOGGER.error("Не удалось создать скриншот: %s", error)
        finally:
            self._screenshot_lock.release()

    def run(self) -> None:
        listener = keyboard.Listener(on_press=self.on_press, on_release=self.on_release)
        listener.start()
        LOGGER.info("Горячая клавиша: %s", "+".join(sorted(self.hotkey)))
        LOGGER.info("Backend: %s", "grim (Wayland)" if is_wayland() else "mss")
        LOGGER.info("Скриншоты будут сохраняться в: %s", self.output_dir)
        LOGGER.info("Ожидание нажатия. Для выхода нажми Ctrl+C.")
        try:
            listener.join()
        except KeyboardInterrupt:
            LOGGER.info("Завершение работы")
            listener.stop()


def configure_logging() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
        datefmt="%H:%M:%S",
    )


def main() -> int:
    configure_logging()
    try:
        hotkey, output_dir = load_config()
        HotkeyScreenshotApp(hotkey, output_dir).run()
    except (OSError, ValueError, RuntimeError) as error:
        LOGGER.error("Ошибка запуска: %s", error)
        return 1
    except Exception as error:  # noqa: BLE001
        LOGGER.error("Непредвиденная ошибка: %s", error)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
