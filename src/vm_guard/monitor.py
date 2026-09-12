from __future__ import annotations

import logging
import time
from collections.abc import Callable

from .models import FocusEvent, MonitorConfig, MonitorEvent, WindowInfo

LOGGER = logging.getLogger(__name__)


class FocusMonitor:
    def __init__(self, expected_title: str, window_provider, action: Callable[[], None], config: MonitorConfig | None = None):
        self.expected_title = expected_title
        self.window_provider = window_provider
        self.action = action
        self.config = config or MonitorConfig()
        self._was_active: bool | None = None
        self._last_action_at = 0.0

    def step(self) -> FocusEvent | None:
        current = self.window_provider.current()
        is_active = current.matches(self.expected_title)
        event: FocusEvent | None = None
        if self._was_active is True and not is_active:
            event = FocusEvent(MonitorEvent.FOCUS_LOST, None, current)
            now = time.monotonic()
            if now - self._last_action_at >= self.config.cooldown:
                LOGGER.info("focus_lost: active VM window -> %s", current.title or "<без заголовка>")
                self.action()
                self._last_action_at = now
            else:
                LOGGER.info("focus_lost пропущен из-за cooldown")
        elif self._was_active is False and is_active:
            event = FocusEvent(MonitorEvent.FOCUS_GAINED, None, current)
            LOGGER.info("focus_gained: %s", current.title or "<без заголовка>")
        self._was_active = is_active
        return event

    def run(self, stop: Callable[[], bool] | None = None, on_event: Callable[[FocusEvent], None] | None = None) -> None:
        LOGGER.info("monitor запущен; ожидаемый заголовок: %s", self.expected_title)
        while not (stop and stop()):
            event = self.step()
            if event and on_event:
                on_event(event)
            time.sleep(self.config.poll_interval)
