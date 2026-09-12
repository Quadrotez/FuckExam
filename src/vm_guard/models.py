from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


@dataclass(frozen=True)
class VMInfo:
    identifier: str
    name: str
    uuid: str
    state: str


@dataclass(frozen=True)
class WindowInfo:
    title: str
    handle: str | None = None
    process_id: int | None = None

    def matches(self, expected: str) -> bool:
        return expected.casefold() in self.title.casefold()


class MonitorEvent(str, Enum):
    FOCUS_GAINED = "focus_gained"
    FOCUS_LOST = "focus_lost"


@dataclass(frozen=True)
class FocusEvent:
    kind: MonitorEvent
    previous: WindowInfo | None
    current: WindowInfo


@dataclass(frozen=True)
class MonitorConfig:
    poll_interval: float = 0.25
    cooldown: float = 1.0
    allow_input: bool = False
    nudge_pixels: int = 20
