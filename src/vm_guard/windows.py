from __future__ import annotations

import platform
import shutil
import subprocess

from .models import WindowInfo


class WindowDetectionError(RuntimeError):
    pass


class ActiveWindowProvider:
    def current(self) -> WindowInfo:
        system = platform.system()
        if system == "Windows":
            return self._windows()
        if system == "Linux":
            return self._linux()
        raise WindowDetectionError(f"Платформа {system} пока не поддержана для определения фокуса окна.")

    @staticmethod
    def _windows() -> WindowInfo:
        import ctypes
        import ctypes.wintypes

        user32 = ctypes.windll.user32
        handle = user32.GetForegroundWindow()
        length = user32.GetWindowTextLengthW(handle)
        buffer = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(handle, buffer, length + 1)
        return WindowInfo(title=buffer.value, handle=str(handle))

    @staticmethod
    def _linux() -> WindowInfo:
        if shutil.which("xdotool"):
            window_id = subprocess.check_output(["xdotool", "getactivewindow"], text=True).strip()
            title = subprocess.check_output(["xdotool", "getwindowname", window_id], text=True).strip()
            return WindowInfo(title=title, handle=window_id)
        if shutil.which("xprop"):
            root = subprocess.check_output(["xprop", "-root", "_NET_ACTIVE_WINDOW"], text=True).strip()
            window_id = root.rsplit(" ", 1)[-1]
            title = subprocess.check_output(["xprop", "-id", window_id, "_NET_WM_NAME"], text=True, errors="replace").strip()
            return WindowInfo(title=title, handle=window_id)
        raise WindowDetectionError("Для Linux нужен xdotool или xprop; Wayland backend будет добавлен отдельно.")
