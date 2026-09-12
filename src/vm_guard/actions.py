from __future__ import annotations

import logging
import time
from dataclasses import dataclass

LOGGER = logging.getLogger(__name__)


@dataclass
class MouseNudgeAction:
    pixels: int = 20
    allow_input: bool = False
    sleep_seconds: float = 0.05

    def run(self) -> None:
        if self.pixels <= 0:
            raise ValueError("Амплитуда движения должна быть положительной")
        if not self.allow_input:
            LOGGER.info("dry-run: движение мыши влево-вправо на %d px пропущено", self.pixels)
            return
        try:
            from pynput.mouse import Controller
        except ImportError as exc:
            raise RuntimeError("Для --allow-input установите pynput") from exc
        mouse = Controller()
        x, y = mouse.position
        mouse.position = (x - self.pixels, y)
        time.sleep(self.sleep_seconds)
        mouse.position = (x + self.pixels, y)
        time.sleep(self.sleep_seconds)
        mouse.position = (x, y)
        LOGGER.info("mouse nudge выполнен")
