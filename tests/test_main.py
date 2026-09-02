import configparser
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

import main


def make_config() -> configparser.ConfigParser:
    config = configparser.ConfigParser()
    config.read_dict({
        "screenshot": {"hotkey": "ctrl+alt+printscreen", "output_dir": "screenshots"},
        "ocr": {"command": "tesseract", "language": "rus+eng", "psm": "6"},
        "llm": {
            "provider": "groq",
            "model": "llama-3.1-8b-instant",
            "api_key": "test-key",
            "temperature": "0.2",
            "max_tokens": "100",
            "system_prompt": "Ответь кратко.",
        },
        "telegram": {"bot_token": "123:token", "chat_id": "42", "parse_mode": "HTML"},
        "network": {"proxy_url": "socks5://127.0.0.1:1080", "timeout": "10"},
    })
    return config


class MainTests(unittest.TestCase):
    def test_config_requires_local_secret_file(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "config.ini.example"):
                main.load_config(Path(directory) / "config.ini")

    def test_config_defaults_to_new_hotkey(self):
        config = make_config()
        self.assertEqual(main.config_hotkey(config), ["ctrl", "alt", "printscreen"])

    def test_normalize_key_aliases(self):
        self.assertEqual(main.normalize_name("Control"), "ctrl")
        self.assertEqual(main.normalize_name("PrintScreen"), "printscreen")
        self.assertEqual(main.normalize_key(main.keyboard.Key.alt_l), "alt")

    def test_proxy_supports_http_and_socks5(self):
        config = make_config()
        self.assertEqual(main.proxy_from_config(config), {"http": "socks5://127.0.0.1:1080", "https": "socks5://127.0.0.1:1080"})
        config["network"]["proxy_url"] = "http://127.0.0.1:8080"
        self.assertEqual(main.proxy_from_config(config)["https"], "http://127.0.0.1:8080")

    @patch("main.requests.post")
    def test_llm_uses_groq_openai_compatible_api(self, post):
        response = Mock(ok=True)
        response.json.return_value = {"choices": [{"message": {"content": "Ответ"}}]}
        post.return_value = response
        answer = main.llm_answer("текст", make_config())
        self.assertEqual(answer, "Ответ")
        self.assertEqual(post.call_args.args[0], "https://api.groq.com/openai/v1/chat/completions")
        self.assertEqual(post.call_args.kwargs["proxies"]["https"], "socks5://127.0.0.1:1080")

    @patch("main.requests.post")
    def test_telegram_sends_formatted_answer_and_ocr(self, post):
        post.return_value = Mock(ok=True)
        config = make_config()
        main.send_telegram(main.format_telegram_message("Ответ", "текст со скриншота", Path("screen.png"), config), config)
        payload = post.call_args.kwargs["json"]
        self.assertIn("Ответ", payload["text"])
        self.assertIn("текст со скриншота", payload["text"])
        self.assertEqual(payload["parse_mode"], "HTML")

    @patch("main.take_screenshot", return_value=(Path("screen.png"), "test"))
    def test_hotkey_triggers_once_until_release(self, take_screenshot):
        config = make_config()
        with tempfile.TemporaryDirectory() as directory:
            app = main.HotkeyScreenshotApp(["ctrl", "g"], Path(directory), config)
            app._run_pipeline = lambda: None
            app.on_press(main.keyboard.Key.ctrl_l)
            app.on_press(main.keyboard.KeyCode.from_char("g"))
            app.on_press(main.keyboard.KeyCode.from_char("g"))
            self.assertTrue(app.triggered)
            app.on_release(main.keyboard.KeyCode.from_char("g"))
            self.assertFalse(app.triggered)

    @patch("main.shutil.which", return_value=None)
    def test_grim_error_is_clear(self, _which):
        with self.assertRaisesRegex(RuntimeError, "Не найден grim"):
            main.screenshot_with_grim(Path("screen.png"))


if __name__ == "__main__":
    unittest.main()
