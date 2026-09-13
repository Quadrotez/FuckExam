from __future__ import annotations

import logging
import threading
import time
from dataclasses import dataclass

LOGGER = logging.getLogger(__name__)


class GuestMouseError(RuntimeError):
    """VirtualBox guest mouse is unavailable or cannot accept an event."""


@dataclass
class GuestMouseOscillator:
    identifier: str
    pixels: int = 100
    allow_input: bool = False
    interval_seconds: float = 0.25

    def __post_init__(self) -> None:
        if self.pixels <= 0:
            raise ValueError("Амплитуда движения должна быть положительной")
        if self.interval_seconds <= 0:
            raise ValueError("Интервал движения должен быть положительным")
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self._session = None

    def start(self) -> None:
        if not self.allow_input:
            LOGGER.info(
                "dry-run: виртуальная мышь VM двигалась бы влево-вправо, пока окно VM не получит фокус"
            )
            return
        if self._thread and self._thread.is_alive():
            return
        self._stop_event.clear()
        self._thread = threading.Thread(target=self._run, name="guest-mouse-oscillator", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop_event.set()
        if self._thread and self._thread is not threading.current_thread():
            self._thread.join(timeout=max(1.0, self.interval_seconds * 4))
        self._thread = None

    def _run(self) -> None:
        session = None
        try:
            session, mouse = self._open_guest_mouse()
            if not getattr(mouse, "relative_supported", True):
                raise GuestMouseError(
                    "Гостевая VM не поддерживает relative mouse events; "
                    "проверьте Pointing Device и Guest Additions"
                )
            direction = -self.pixels
            LOGGER.info(
                "guest mouse oscillator запущен: amplitude=%d interval=%.2fs; host cursor не изменяется",
                self.pixels,
                self.interval_seconds,
            )
            while not self._stop_event.is_set():
                mouse.put_mouse_event(direction, 0, 0, 0, 0)
                direction = -direction
                self._stop_event.wait(self.interval_seconds)
        except Exception as exc:
            LOGGER.error("guest mouse oscillator остановлен с ошибкой: %s", exc)
        finally:
            if session is not None:
                try:
                    session.unlock_machine()
                except Exception as exc:
                    LOGGER.warning("не удалось освободить VirtualBox session: %s", exc)
            LOGGER.info("guest mouse oscillator остановлен")

    def _open_guest_mouse(self):
        try:
            import virtualbox
            from virtualbox.library import LockType
        except ImportError as exc:
            raise GuestMouseError(
                "Не найден Python VirtualBox SDK. Выполните: "
                "`sudo pacman -S virtualbox-sdk` и `python -m pip install virtualbox`. "
                "Проверьте импорт командами `python -c 'import vboxapi'` и "
                "`python -c 'import virtualbox'`."
            ) from exc
        try:
            vbox = virtualbox.VirtualBox()
            machine = vbox.find_machine(self.identifier)
            session = virtualbox.Session()
            machine.lock_machine(session, LockType.shared)
            return session, session.console.mouse
        except Exception as exc:
            raise GuestMouseError(
                f"Не удалось открыть виртуальную мышь VM {self.identifier!r}: {exc}"
            ) from exc
