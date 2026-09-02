from __future__ import annotations

import configparser
import html
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

import requests
from pynput import keyboard


ROOT = Path(__file__).resolve().parent
CONFIG_FILE = ROOT / "config.ini"
LOGGER = logging.getLogger("fuckexam-screenshot")

KEY_ALIASES = {
    "control": "ctrl", "ctrl_l": "ctrl", "ctrl_r": "ctrl",
    "shift_l": "shift", "shift_r": "shift",
    "alt_l": "alt", "alt_r": "alt",
    "cmd": "cmd", "cmd_l": "cmd", "cmd_r": "cmd",
    "win": "cmd", "super": "cmd", "super_l": "cmd", "super_r": "cmd",
    "print": "printscreen", "print_screen": "printscreen",
}
PROVIDER_URLS = {
    "openrouter": "https://openrouter.ai/api/v1/chat/completions",
    "groq": "https://api.groq.com/openai/v1/chat/completions",
}


def normalize_name(value: str) -> str:
    name = value.strip().lower().replace(" ", "_")
    return KEY_ALIASES.get(name, name)


def normalize_key(key: keyboard.Key | keyboard.KeyCode) -> str | None:
    if isinstance(key, keyboard.KeyCode):
        return normalize_name(key.char) if key.char else None
    return normalize_name(str(key).removeprefix("Key."))


def load_config(path: Path = CONFIG_FILE) -> configparser.ConfigParser:
    if not path.is_file():
        raise RuntimeError(f"Не найден {path}. Скопируй config.ini.example в config.ini и заполни секреты.")
    config = configparser.ConfigParser()
    config.read(path, encoding="utf-8")
    return config


def config_hotkey(config: configparser.ConfigParser) -> list[str]:
    value = config.get("screenshot", "hotkey", fallback="ctrl+alt+printscreen")
    keys = [normalize_name(part) for part in value.split("+") if part.strip()]
    if not keys or len(set(keys)) != len(keys):
        raise ValueError("Некорректный hotkey в config.ini")
    return keys


def output_dir_from_config(config: configparser.ConfigParser) -> Path:
    value = Path(config.get("screenshot", "output_dir", fallback="screenshots")).expanduser()
    return value if value.is_absolute() else ROOT / value


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
    if shutil.which("spectacle") is None:
        raise RuntimeError("Не найден KDE Spectacle")
    run_capture_command(["spectacle", "--background", "--nonotify", "--fullscreen", "--output", str(destination)], destination, "Spectacle")


def screenshot_with_gnome_screenshot(destination: Path) -> None:
    if shutil.which("gnome-screenshot") is None:
        raise RuntimeError("Не найден gnome-screenshot")
    run_capture_command(["gnome-screenshot", "--file", str(destination)], destination, "gnome-screenshot")


def screenshot_with_grim(destination: Path) -> None:
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
        image = capture.grab(capture.monitors[0])
        to_png(image.rgb, image.size, output=str(destination))


def screenshot_wayland(destination: Path) -> str:
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
        except (RuntimeError, OSError) as error:
            errors.append(str(error))
            destination.unlink(missing_ok=True)
    if errors:
        raise RuntimeError("; ".join(errors))
    raise RuntimeError("Не найден backend скриншотов для Wayland. Установи Spectacle, gnome-screenshot или grim.")


def take_screenshot(output_dir: Path) -> tuple[Path, str]:
    output_dir.mkdir(parents=True, exist_ok=True)
    destination = make_filename(output_dir)
    if is_wayland():
        backend = screenshot_wayland(destination)
    else:
        screenshot_with_mss(destination)
        backend = "mss"
    return destination, backend


def proxy_from_config(config: configparser.ConfigParser) -> dict[str, str] | None:
    proxy = config.get("network", "proxy_url", fallback="").strip()
    if not proxy:
        return None
    if not proxy.startswith(("http://", "https://", "socks5://", "socks5h://")):
        raise ValueError("proxy_url должен начинаться с http://, https://, socks5:// или socks5h://")
    return {"http": proxy, "https": proxy}


def request_kwargs(config: configparser.ConfigParser) -> dict:
    kwargs = {"timeout": config.getint("network", "timeout", fallback=60)}
    proxies = proxy_from_config(config)
    if proxies:
        kwargs["proxies"] = proxies
    return kwargs


def transcribe_image(image_path: Path, config: configparser.ConfigParser) -> str:
    command = config.get("ocr", "command", fallback="tesseract")
    language = config.get("ocr", "language", fallback="rus+eng")
    psm = config.get("ocr", "psm", fallback="6")
    if shutil.which(command) is None:
        raise RuntimeError(f"Не найден OCR-инструмент {command}. Установи пакет tesseract и языковые данные.")
    result = subprocess.run([command, str(image_path), "stdout", "-l", language, "--psm", psm], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        details = result.stderr.strip() or "нет подробностей"
        raise RuntimeError(f"OCR завершился с кодом {result.returncode}: {details}")
    text = result.stdout.strip()
    if not text:
        raise RuntimeError("OCR не распознал текст на скриншоте")
    return text


def llm_answer(text: str, config: configparser.ConfigParser) -> str:
    provider = config.get("llm", "provider", fallback="openrouter").strip().lower()
    if provider not in PROVIDER_URLS:
        raise ValueError(f"Неизвестный provider: {provider}. Используй openrouter или groq.")
    api_key = config.get("llm", "api_key", fallback="").strip()
    if not api_key or api_key.startswith("ВСТАВЬ"):
        raise RuntimeError("В config.ini не задан llm.api_key")
    url = config.get("llm", "base_url", fallback="").strip() or PROVIDER_URLS[provider]
    payload = {
        "model": config.get("llm", "model"),
        "messages": [
            {"role": "system", "content": config.get("llm", "system_prompt", fallback="Ответь по распознанному тексту.")},
            {"role": "user", "content": f"Текст со скриншота:\n\n{text}"},
        ],
        "temperature": config.getfloat("llm", "temperature", fallback=0.2),
        "max_tokens": config.getint("llm", "max_tokens", fallback=2000),
    }
    response = requests.post(url, headers={"Authorization": f"Bearer {api_key}", "Content-Type": "application/json"}, json=payload, **request_kwargs(config))
    if not response.ok:
        raise RuntimeError(f"{provider} HTTP {response.status_code}: {response.text[:500]}")
    data = response.json()
    try:
        answer = data["choices"][0]["message"]["content"].strip()
    except (KeyError, IndexError, TypeError) as error:
        raise RuntimeError(f"Некорректный ответ от {provider}") from error
    if not answer:
        raise RuntimeError(f"{provider} вернул пустой ответ")
    return answer


def send_telegram(text: str, config: configparser.ConfigParser) -> None:
    token = config.get("telegram", "bot_token", fallback="").strip()
    chat_id = config.get("telegram", "chat_id", fallback="").strip()
    if not token or token.startswith("ВСТАВЬ") or not chat_id or chat_id.startswith("ВСТАВЬ"):
        raise RuntimeError("В config.ini не заданы telegram.bot_token и telegram.chat_id")
    url = f"https://api.telegram.org/bot{token}/sendMessage"
    parse_mode = config.get("telegram", "parse_mode", fallback="").strip()
    for offset in range(0, len(text), 3900):
        chunk = text[offset:offset + 3900]
        payload = {"chat_id": chat_id, "text": chunk}
        if parse_mode:
            payload["parse_mode"] = parse_mode
        response = requests.post(url, json=payload, **request_kwargs(config))
        if not response.ok:
            raise RuntimeError(f"Telegram HTTP {response.status_code}: {response.text[:500]}")


def format_telegram_message(answer: str, ocr_text: str, image_path: Path, config: configparser.ConfigParser) -> str:
    include_ocr = config.getboolean("telegram", "include_ocr_text", fallback=True)
    if config.get("telegram", "parse_mode", fallback="").strip().upper() == "HTML":
        result = f"<b>Ответ нейронки</b>\n{html.escape(answer)}"
        if include_ocr:
            result += f"\n\n<b>Распознанный текст</b>\n<pre>{html.escape(ocr_text)}</pre>"
        result += f"\n\n<i>Скриншот: {html.escape(image_path.name)}</i>"
        return result
    result = f"Ответ нейронки:\n{answer}"
    if include_ocr:
        result += f"\n\nРаспознанный текст:\n{ocr_text}"
    return result + f"\n\nСкриншот: {image_path.name}"


def process_screenshot(image_path: Path, config: configparser.ConfigParser) -> None:
    LOGGER.info("Распознаю текст: %s", image_path)
    ocr_text = transcribe_image(image_path, config)
    LOGGER.info("OCR завершён: %d символов", len(ocr_text))
    answer = llm_answer(ocr_text, config)
    send_telegram(format_telegram_message(answer, ocr_text, image_path, config), config)
    LOGGER.info("Ответ отправлен в Telegram")


class HotkeyScreenshotApp:
    def __init__(self, hotkey: Iterable[str], output_dir: Path, config: configparser.ConfigParser) -> None:
        self.hotkey = set(hotkey)
        self.output_dir = output_dir
        self.config = config
        self.pressed: set[str] = set()
        self.triggered = False
        self._pipeline_lock = threading.Lock()

    def on_press(self, key: keyboard.Key | keyboard.KeyCode) -> None:
        normalized = normalize_key(key)
        if normalized is None:
            return
        self.pressed.add(normalized)
        if not self.triggered and self.hotkey.issubset(self.pressed):
            self.triggered = True
            threading.Thread(target=self._run_pipeline, daemon=True).start()

    def on_release(self, key: keyboard.Key | keyboard.KeyCode) -> None:
        normalized = normalize_key(key)
        if normalized is not None:
            self.pressed.discard(normalized)
        if not self.hotkey.issubset(self.pressed):
            self.triggered = False

    def _run_pipeline(self) -> None:
        if not self._pipeline_lock.acquire(blocking=False):
            LOGGER.warning("Предыдущий pipeline ещё выполняется, нажатие пропущено")
            return
        try:
            LOGGER.info("Создаю скриншот...")
            image_path, backend = take_screenshot(self.output_dir)
            LOGGER.info("Скриншот сохранён: %s (backend: %s)", image_path, backend)
            process_screenshot(image_path, self.config)
        except Exception as error:  # noqa: BLE001
            LOGGER.error("Не удалось обработать скриншот: %s", error)
        finally:
            self._pipeline_lock.release()

    def run(self) -> None:
        listener = keyboard.Listener(on_press=self.on_press, on_release=self.on_release)
        listener.start()
        LOGGER.info("Горячая клавиша: %s", "+".join(sorted(self.hotkey)))
        LOGGER.info("Скриншоты будут сохраняться в: %s", self.output_dir)
        LOGGER.info("Ожидание нажатия. Для выхода нажми Ctrl+C.")
        try:
            listener.join()
        except KeyboardInterrupt:
            LOGGER.info("Завершение работы")
            listener.stop()


def main() -> int:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s", datefmt="%H:%M:%S")
    try:
        config = load_config()
        HotkeyScreenshotApp(config_hotkey(config), output_dir_from_config(config), config).run()
    except (OSError, ValueError, RuntimeError) as error:
        LOGGER.error("Ошибка запуска: %s", error)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
