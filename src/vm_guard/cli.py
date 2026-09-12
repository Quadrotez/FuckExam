from __future__ import annotations

import argparse
import logging

from .actions import MouseNudgeAction
from .models import MonitorConfig, MonitorEvent
from .monitor import FocusMonitor
from .virtualbox import VBoxManageClient, VirtualBoxError
from .windows import ActiveWindowProvider, WindowDetectionError


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="VM Guard — monitor VirtualBox window focus")
    parser.add_argument("--vm", dest="identifier", help="имя или UUID виртуальной машины")
    parser.add_argument("--window-title", help="подстрока заголовка окна; по умолчанию имя VM")
    parser.add_argument("--poll-interval", type=float, default=0.25)
    parser.add_argument("--nudge-pixels", type=int, default=20)
    parser.add_argument("--cooldown", type=float, default=1.0)
    parser.add_argument("--allow-input", action="store_true", help="разрешить реальное движение мыши")
    parser.add_argument("--once", action="store_true", help="завершиться после первого focus_lost")
    parser.add_argument("--log-level", default="INFO", choices=["DEBUG", "INFO", "WARNING", "ERROR"])
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    logging.basicConfig(level=getattr(logging, args.log_level), format="%(asctime)s %(levelname)s %(name)s %(message)s")
    identifier = args.identifier or input("Имя или UUID VirtualBox VM: ").strip()
    if not identifier:
        logging.error("Идентификатор VM не задан")
        return 2
    if args.poll_interval <= 0 or args.cooldown < 0:
        logging.error("poll interval должен быть > 0, cooldown не может быть отрицательным")
        return 2
    try:
        vm = VBoxManageClient().show_vm(identifier)
        logging.info("VM найдена: name=%s uuid=%s state=%s", vm.name, vm.uuid, vm.state)
        expected_title = args.window_title or vm.name
        provider = ActiveWindowProvider()
        action = MouseNudgeAction(args.nudge_pixels, args.allow_input)
        monitor = FocusMonitor(expected_title, provider, action.run, MonitorConfig(args.poll_interval, args.cooldown, args.allow_input, args.nudge_pixels))
        if args.once:
            stopped = False

            def stop_after_focus_lost(event) -> None:
                nonlocal stopped
                if event.kind is MonitorEvent.FOCUS_LOST:
                    stopped = True

            monitor.run(lambda: stopped, stop_after_focus_lost)
        else:
            monitor.run()
    except (VirtualBoxError, WindowDetectionError, RuntimeError, KeyboardInterrupt) as exc:
        if isinstance(exc, KeyboardInterrupt):
            logging.info("остановлено пользователем")
            return 0
        logging.error("запуск невозможен: %s", exc)
        return 1
    return 0
