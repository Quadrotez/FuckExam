from __future__ import annotations

import logging
import time
from dataclasses import dataclass

LOGGER = logging.getLogger(__name__)


class GuestMouseError(RuntimeError):
    """VirtualBox guest mouse is unavailable or cannot accept an event."""


@dataclass
class GuestMouseNudgeAction:
    identifier: str
    pixels: int = 20
    allow_input: bool = False
    sleep_seconds: float = 0.05

    def run(self) -> None:
        if self.pixels <= 0:
            raise ValueError("Амплитуда движения должна быть положительной")
        if not self.allow_input:
            LOGGER.info("dry-run: движение виртуальной мыши VM влево-вправо на %d px пропущено", self.pixels)
            return
        session, mouse = self._open_guest_mouse()
        try:
            if not getattr(mouse, "relative_supported", True):
                raise GuestMouseError("Гостевая VM не поддерживает относительное движение мыши")
            # IMouse.putMouseEvent changes only the VirtualBox guest device.
            mouse.put_mouse_event(-self.pixels, 0, 0, 0, 0)
            time.sleep(self.sleep_seconds)
            mouse.put_mouse_event(self.pixels, 0, 0, 0, 0)
            LOGGER.info("guest mouse nudge выполнен; host cursor не изменялся")
        except Exception as exc:
            if isinstance(exc, GuestMouseError):
                raise
            raise GuestMouseError(f"VirtualBox не принял событие виртуальной мыши: {exc}") from exc
        finally:
            try:
                session.unlock_machine()
            except Exception as exc:
                LOGGER.warning("не удалось освободить VirtualBox session: %s", exc)

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
            # Shared lock lets us access the console of an already running VM
            # without taking control of the GUI process or host cursor.
            machine.lock_machine(session, LockType.shared)
            return session, session.console.mouse
        except Exception as exc:
            raise GuestMouseError(
                f"Не удалось открыть виртуальную мышь VM {self.identifier!r}: {exc}"
            ) from exc
