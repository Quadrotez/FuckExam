from __future__ import annotations

import os
import platform
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class Component:
    name: str
    command: str | None
    required: bool
    purpose: str


@dataclass(frozen=True)
class EnvironmentReport:
    distro: str
    desktop: str
    session: str
    compositor: str
    components: tuple[Component, ...]
    package_manager: str | None

    @property
    def missing(self) -> tuple[Component, ...]:
        return tuple(item for item in self.components if item.command and shutil.which(item.command) is None)

    @property
    def has_required(self) -> bool:
        return not any(item.required for item in self.missing)

    @property
    def install_packages(self) -> tuple[str, ...]:
        packages: list[str] = []
        for item in self.missing:
            if item.command == "ffmpeg":
                packages.append("ffmpeg")
            elif item.command == "VBoxManage":
                packages.append("virtualbox")
            elif item.command == "grim":
                packages.append("grim")
            elif item.command == "xdg-desktop-portal":
                packages.append("xdg-desktop-portal")
            elif item.command == "pipewire":
                packages.append("pipewire")
            elif item.command == "wireplumber":
                packages.append("wireplumber")
            elif item.command == "xdotool":
                packages.append("xdotool")
            elif item.command == "hyprctl":
                continue
            elif item.command == "xdg-desktop-portal-hyprland":
                packages.append("xdg-desktop-portal-hyprland")
            elif item.command == "xdg-desktop-portal-kde":
                packages.append("xdg-desktop-portal-kde")
            elif item.command == "xdg-desktop-portal-gnome":
                packages.append("xdg-desktop-portal-gnome")
            elif item.command == "xdg-desktop-portal-wlr":
                packages.append("xdg-desktop-portal-wlr")
        return tuple(dict.fromkeys(packages))

    def install_command(self) -> list[str] | None:
        if not self.package_manager or not self.install_packages:
            return None
        if self.package_manager == "pacman":
            return ["pacman", "-S", "--needed", *self.install_packages]
        if self.package_manager == "apt-get":
            package_map = {"virtualbox": "virtualbox", "grim": "grim", "xdotool": "xdotool"}
            packages = [package_map.get(item, item) for item in self.install_packages]
            return ["apt-get", "install", "-y", *packages]
        return None


def _read_distro() -> str:
    values: dict[str, str] = {}
    try:
        for line in Path("/etc/os-release").read_text(encoding="utf-8").splitlines():
            if "=" in line:
                key, value = line.split("=", 1)
                values[key] = value.strip().strip('"')
    except OSError:
        pass
    return values.get("PRETTY_NAME", platform.system())


def detect_environment() -> EnvironmentReport:
    session = os.environ.get("XDG_SESSION_TYPE", "unknown").lower()
    desktop = os.environ.get("XDG_CURRENT_DESKTOP", os.environ.get("DESKTOP_SESSION", "unknown"))
    if os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") or shutil.which("hyprctl"):
        compositor = "Hyprland"
    elif "KDE" in desktop.upper() or os.environ.get("KDE_FULL_SESSION"):
        compositor = "KDE Plasma"
    elif "GNOME" in desktop.upper():
        compositor = "GNOME"
    elif session == "wayland":
        compositor = "Wayland compositor (unknown)"
    elif session == "x11":
        compositor = "X11"
    else:
        compositor = "unknown"

    components = [
        Component("FFmpeg", "ffmpeg", True, "local MP4 recording and encoding"),
        Component("VirtualBox", "VBoxManage", True, "VM discovery and selection"),
    ]
    if session == "wayland":
        components.extend([
            Component("XDG Desktop Portal", "xdg-desktop-portal", True, "native Wayland permission dialog"),
            Component("PipeWire", "pipewire", True, "native Wayland video stream"),
            Component("WirePlumber", "wireplumber", True, "PipeWire session management"),
            Component("grim", "grim", False, "fallback Wayland frame capture"),
        ])
        backend_commands = {
            "Hyprland": "xdg-desktop-portal-hyprland",
            "KDE Plasma": "xdg-desktop-portal-kde",
            "GNOME": "xdg-desktop-portal-gnome",
            "Wayland compositor (unknown)": "xdg-desktop-portal-wlr",
        }
        backend = backend_commands.get(compositor)
        if backend:
            components.append(Component("Portal backend", backend, True, f"ScreenCast backend for {compositor}"))
    elif session == "x11":
        components.append(Component("xdotool", "xdotool", False, "window geometry lookup"))

    package_manager = "pacman" if shutil.which("pacman") else "apt-get" if shutil.which("apt-get") else None
    return EnvironmentReport(_read_distro(), desktop, session, compositor, tuple(components), package_manager)


def install_missing(report: EnvironmentReport) -> tuple[bool, str]:
    command = report.install_command()
    if not command:
        return False, "Для этого дистрибутива автоматическая установка не настроена."
    if shutil.which("pkexec"):
        full_command = ["pkexec", *command]
    else:
        full_command = ["sudo", *command]
    try:
        result = subprocess.run(full_command, check=False)
    except OSError as exc:
        return False, str(exc)
    if result.returncode != 0:
        return False, f"Установка завершилась с кодом {result.returncode}."
    return True, "Установка завершена. Перезапустите проверку."
