import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import main


class MainTests(unittest.TestCase):
    def test_load_config_resolves_relative_output_dir(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.ini"
            config.write_text("[screenshot]\nhotkey = Ctrl + Shift + G\noutput_dir = captures\n", encoding="utf-8")
            hotkey, output_dir = main.load_config(config)

        self.assertEqual(hotkey, ["ctrl", "shift", "g"])
        self.assertEqual(output_dir, config.parent / "captures")

    def test_normalize_key_aliases(self):
        self.assertEqual(main.normalize_name("Control"), "ctrl")
        self.assertEqual(main.normalize_name("Key.shift_l"), "key.shift_l")
        self.assertEqual(main.normalize_name("  g "), "g")

    @patch("main.take_screenshot", return_value=(Path("screen.png"), "test"))
    def test_hotkey_triggers_once_until_release(self, take_screenshot):
        with tempfile.TemporaryDirectory() as directory:
            app = main.HotkeyScreenshotApp(["ctrl", "g"], Path(directory))
            ctrl = main.keyboard.Key.ctrl_l
            g = main.keyboard.KeyCode.from_char("g")

            app.on_press(ctrl)
            app.on_press(g)
            app.on_press(g)
            self.assertEqual(take_screenshot.call_count, 1)

            app.on_release(g)
            app.on_press(g)
            self.assertEqual(take_screenshot.call_count, 2)

    @patch("main.shutil.which", return_value=None)
    def test_grim_error_is_clear(self, _which):
        with self.assertRaisesRegex(RuntimeError, "Не найден grim"):
            main.screenshot_with_grim(Path("screen.png"))

    @staticmethod
    def available_capture_tools(name):
        return f"/usr/bin/{name}" if name in {"spectacle", "grim"} else None

    @patch.dict(main.os.environ, {"XDG_CURRENT_DESKTOP": "KDE"}, clear=False)
    @patch("main.shutil.which", side_effect=available_capture_tools.__func__)
    @patch("main.screenshot_with_spectacle")
    def test_wayland_prefers_spectacle_on_kde(self, spectacle, _which):
        spectacle.return_value = None
        with tempfile.TemporaryDirectory() as directory:
            backend = main.screenshot_wayland(Path(directory) / "screen.png")
        self.assertEqual(backend, "spectacle")
        spectacle.assert_called_once()

    @patch.dict(main.os.environ, {"XDG_CURRENT_DESKTOP": "KDE"}, clear=False)
    @patch("main.shutil.which", side_effect=available_capture_tools.__func__)
    @patch("main.screenshot_with_spectacle", side_effect=RuntimeError("unsupported"))
    @patch("main.screenshot_with_grim")
    def test_wayland_falls_back_to_grim(self, grim, _spectacle, _which):
        grim.return_value = None
        with tempfile.TemporaryDirectory() as directory:
            backend = main.screenshot_wayland(Path(directory) / "screen.png")
        self.assertEqual(backend, "grim")
        grim.assert_called_once()


if __name__ == "__main__":
    unittest.main()
