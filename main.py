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
from typing import Callable, Iterable

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


def run_capture_command(command: list[str], destination: Path, backend: str) -> None:
    result = subprocess.run(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        details = result.stderr.strip() or result.stdout.strip() or "нет подробностей"
        raise RuntimeError(f"{backend} завершился с кодом {result.returncode}: {details}")
    if not destination.is_file() or destination.stat().st_size == 0:
        raise RuntimeError(f"{backend} завершился успешно, но файл скриншота не создан")


def screenshot_with_spectacle(destination: Path) -> None:
    """Capture through KDE Spectacle, which does not require wlr-screencopy."""
    if shutil.which("spectacle") is None:
        raise RuntimeError("Не найден KDE Spectacle")
    run_capture_command(
        ["spectacle", "--background", "--nonotify", "--fullscreen", "--output", str(destination)],
        destination,
        "Spectacle",
    )


def screenshot_with_gnome_screenshot(destination: Path) -> None:
    """Capture through GNOME Screenshot when available."""
    if shutil.which("gnome-screenshot") is None:
        raise RuntimeError("Не найден gnome-screenshot")
    run_capture_command(
        ["gnome-screenshot", "--file", str(destination)],
        destination,
        "gnome-screenshot",
    )


def screenshot_with_grim(destination: Path) -> None:
    """Capture through grim for compositors implementing wlr-screencopy."""
    if shutil.which("grim") is None:
        raise RuntimeError("Не найден grim")
    run_capture_command(["grim", str(destination)], destination, "grim")


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


def screenshot_wayland(destination: Path) -> str:
    """Select the best available compositor-native capture backend."""
    desktop = os.environ.get("XDG_CURRENT_DESKTOP", "").lower()
    candidates: list[tuple[str, Callable[[Path], None]]] = []

    if "kde" in desktop or os.environ.get("KDE_FULL_SESSION"):
        candidates.append(("spectacle", screenshot_with_spectacle))
    if "gnome" in desktop:
        candidates.append(("gnome-screenshot", screenshot_with_gnome_screenshot))
    candidates.extend([
        ("spectacle", screenshot_with_spectacle),
        ("gnome-screenshot", screenshot_with_gnome_screenshot),
        ("grim", screenshot_with_grim),
    ])

    attempted: set[str] = set()
    errors: list[str] = []
    for name, capture in candidates:
        if name in attempted or shutil.which(name) is None:
            continue
        attempted.add(name)
        try:
            capture(destination)
            return name
        except RuntimeError as error:
            errors.append(str(error))
            destination.unlink(missing_ok=True)

    if errors:
        raise RuntimeError("; ".join(errors))
    raise RuntimeError(
        "Не найден backend скриншотов для Wayland. Установи Spectacle, "
        "gnome-screenshot или grim."
    )


def take_screenshot(output_dir: Path) -> tuple[Path, str]:
    output_dir.mkdir(parents=True, exist_ok=True)
    destination = make_filename(output_dir)
    if is_wayland():
        backend = screenshot_wayland(destination)
    else:
        screenshot_with_mss(destination)
        backend = "mss"
    return destination, backend


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
            destination, backend = take_screenshot(self.output_dir)
            LOGGER.info("Скриншот сохранён: %s (backend: %s)", destination, backend)
        except Exception as error:  # noqa: BLE001 - ошибка должна быть видна пользователю
            LOGGER.error("Не удалось создать скриншот: %s", error)
        finally:
            self._screenshot_lock.release()

    def run(self) -> None:
        listener = keyboard.Listener(on_press=self.on_press, on_release=self.on_release)
        listener.start()
        LOGGER.info("Горячая клавиша: %s", "+".join(sorted(self.hotkey)))
        if is_wayland():
            LOGGER.info("Backend: compositor-native Wayland capture")
        else:
            LOGGER.info("Backend: mss")
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
