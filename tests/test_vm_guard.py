import unittest
from unittest.mock import patch

from vm_guard.actions import GuestMouseNudgeAction
from vm_guard.models import MonitorConfig, WindowInfo
from vm_guard.monitor import FocusMonitor
from vm_guard.virtualbox import VBoxManageClient


class FakeWindows:
    def __init__(self, *titles):
        self.titles = iter(titles)

    def current(self):
        return WindowInfo(next(self.titles))


class FocusMonitorTests(unittest.TestCase):
    def test_action_only_on_transition_out_and_not_every_poll(self):
        calls = []
        provider = FakeWindows("Windows 11 - VirtualBox", "Windows 11 - VirtualBox", "Browser", "Browser")
        monitor = FocusMonitor("Windows 11", provider, lambda: calls.append("nudge"), MonitorConfig(cooldown=0))
        monitor.step()
        monitor.step()
        event = monitor.step()
        monitor.step()
        self.assertEqual(calls, ["nudge"])
        self.assertEqual(event.kind.value, "focus_lost")

    def test_title_matching_is_case_insensitive_substring(self):
        self.assertTrue(WindowInfo("Oracle VM VirtualBox Manager").matches("virtualbox"))


class GuestMouseActionTests(unittest.TestCase):
    def test_dry_run_does_not_open_sdk(self):
        with patch.object(GuestMouseNudgeAction, "_open_guest_mouse") as open_mouse:
            GuestMouseNudgeAction("demo", allow_input=False).run()
        open_mouse.assert_not_called()


class VBoxParsingTests(unittest.TestCase):
    def test_show_vm_parses_machine_readable_output(self):
        client = object.__new__(VBoxManageClient)
        client._run = lambda *args: 'name="Demo VM"\nUUID="1234"\nVMState="running"\n'
        vm = client.show_vm("Demo VM")
        self.assertEqual(vm.name, "Demo VM")
        self.assertEqual(vm.uuid, "1234")
        self.assertEqual(vm.state, "running")


if __name__ == "__main__":
    unittest.main()
