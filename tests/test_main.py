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
            "provider": "groq", "model": "meta-llama/llama-4-scout-17b-16e-instruct",
            "api_key": "test-key", "vision": "auto", "vision_models": "",
            "temperature": "0.2", "max_tokens": "100", "system_prompt": "Ответь кратко.",
        },
        "telegram": {"bot_token": "123:token", "chat_id": "42", "parse_mode": "", "include_ocr_text": "true"},
        "network": {"proxy_url": "socks5://127.0.0.1:1080", "timeout": "10"},
    })
    return config


class MainTests(unittest.TestCase):
    def test_config_requires_local_secret_file(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "config.ini.example"):
                main.load_config(Path(directory) / "config.ini")

    def test_config_defaults_to_new_hotkey(self):
        self.assertEqual(main.config_hotkey(make_config()), ["ctrl", "alt", "printscreen"])

    def test_normalize_key_aliases(self):
        self.assertEqual(main.normalize_name("Control"), "ctrl")
        self.assertEqual(main.normalize_name("PrintScreen"), "printscreen")
        self.assertEqual(main.normalize_key(main.keyboard.Key.alt_l), "alt")

    def test_proxy_supports_http_and_socks5(self):
        config = make_config()
        self.assertEqual(main.proxy_from_config(config)["https"], "socks5://127.0.0.1:1080")
        config["network"]["proxy_url"] = "http://127.0.0.1:8080"
        self.assertEqual(main.proxy_from_config(config)["https"], "http://127.0.0.1:8080")

    def test_model_supports_vision(self):
        self.assertTrue(main.model_supports_vision(make_config()))
        config = make_config()
        config["llm"]["model"] = "vendor/custom-model"
        config["llm"]["vision_models"] = "custom-model"
        self.assertTrue(main.model_supports_vision(config))

    @patch("main.requests.post")
    @patch("main.image_data_url", return_value="data:image/png;base64,abc")
    def test_vision_request_sends_image_url(self, _image_data_url, post):
        response = Mock(ok=True)
        response.json.return_value = {"choices": [{"message": {"content": "Ответ по картинке"}}]}
        post.return_value = response
        answer, used_vision = main.llm_answer(Path("screen.png"), "", make_config())
        self.assertEqual(answer, "Ответ по картинке")
        self.assertTrue(used_vision)
        content = post.call_args.kwargs["json"]["messages"][1]["content"]
        self.assertEqual(content[1]["type"], "image_url")
        self.assertEqual(content[1]["image_url"]["url"], "data:image/png;base64,abc")

    @patch("main.transcribe_image", return_value="локальный OCR")
    @patch("main.llm_request", return_value="Ответ по OCR")
    def test_never_uses_ocr(self, llm_request, transcribe):
        config = make_config()
        config["llm"]["vision"] = "never"
        answer, used_vision = main.llm_answer(Path("screen.png"), "", config)
        self.assertEqual((answer, used_vision), ("Ответ по OCR", False))
        transcribe.assert_called_once()
        llm_request.assert_called_once()

    @patch("main.requests.post")
    def test_telegram_sends_answer_and_ocr(self, post):
        post.return_value = Mock(ok=True)
        config = make_config()
        main.send_telegram(main.format_telegram_message("Ответ", "текст со скриншота", Path("screen.png"), config, False), config)
        payload = post.call_args.kwargs["json"]
        self.assertIn("Ответ", payload["text"])
        self.assertIn("текст со скриншота", payload["text"])

    def test_markdown_is_converted_to_telegram_html(self):
        converted = main.markdown_to_telegram_html("## Решение\n**Ответ: 42**\n```python\nprint(42)\n```")
        self.assertNotIn("##", converted)
        self.assertIn("<b>Ответ: 42</b>", converted)
        self.assertIn("<pre><code>print(42)</code></pre>", converted)

    def test_extract_message_text_supports_string_and_blocks(self):
        self.assertEqual(main.extract_message_text({"content": "  ответ  "}), "ответ")
        self.assertEqual(main.extract_message_text({"content": [{"type": "text", "text": "а"}, {"type": "text", "text": "б"}]}), "аб")
        self.assertEqual(main.extract_message_text({"content": None}), "")

    @patch("main.requests.post")
    def test_empty_content_has_diagnostic(self, post):
        response = Mock(ok=True, text='{"choices":[{"message":{"content":null},"finish_reason":"stop"}]}')
        response.json.return_value = {"choices": [{"message": {"content": None}, "finish_reason": "stop"}]}
        post.return_value = response
        with self.assertRaisesRegex(RuntimeError, "пустой content.*finish_reason=stop"):
            main.llm_request(None, "текст", make_config())

    @patch("main.shutil.which", return_value=None)
    def test_grim_error_is_clear(self, _which):
        with self.assertRaisesRegex(RuntimeError, "Не найден grim"):
            main.screenshot_with_grim(Path("screen.png"))


if __name__ == "__main__":
    unittest.main()
