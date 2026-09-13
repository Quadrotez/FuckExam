import unittest

from FuckExam.models import MonitorConfig, WindowInfo
from FuckExam.monitor import FocusMonitor


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


if __name__ == "__main__":
    unittest.main()
