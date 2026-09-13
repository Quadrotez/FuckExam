import base64
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from FuckExam.recording import OBSRecordingError, OBSRecorder

CONFIG = tempfile.gettempdir() + "/fuckexam-obs-websocket-config-test.json"


class FakeReqError(RuntimeError):
    pass


class FakeObs:
    def __init__(self, **responses):
        self.calls = []
        self.responses = responses

    def send(self, request, data=None, raw=False):
        self.calls.append((request, data))
        response = self.responses.get(request)
        if response is None:
            if request in ("GetSourceActive",):
                response = {"requestStatus": {"result": True}, "responseData": {"videoActive": True}}
            else:
                response = {"requestStatus": {"result": True}, "responseData": {}}
        if not response["requestStatus"]["result"]:
            raise FakeReqError(response["requestStatus"].get("comment"))
        return response["responseData"]

    def disconnect(self):
        self.calls.append(("__disconnect__", None))


class OBSRecorderTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.recorder = OBSRecorder(self.root, fps=15)

    def tearDown(self):
        self.tmp.cleanup()

    @mock.patch("FuckExam.recording._config_file")
    def test_ensure_websocket_config_enables_server(self, config_file):
        path = Path(CONFIG)
        if path.exists():
            path.unlink()
        config_file.return_value = path
        self.recorder.ensure_websocket_config()
        import json

        data = json.loads(path.read_text(encoding="utf-8"))
        self.assertTrue(data["server_enabled"])
        self.assertEqual(data["server_port"], 4455)
        self.assertTrue(data["server_password"])
        path.unlink()

    @mock.patch("FuckExam.recording._wayland", return_value=True)
    def test_prepare_builds_scene_and_sources(self, _):
        fake = FakeObs(
            **{
                "GetSceneCollectionList": {"requestStatus": {"result": True}, "responseData": {"sceneCollections": []}},
                "GetSceneList": {"requestStatus": {"result": True}, "responseData": {"scenes": []}},
                "GetInputList": {"requestStatus": {"result": True}, "responseData": {"inputs": []}},
                "GetInputKindList": {
                    "requestStatus": {"result": True},
                    "responseData": {"inputKinds": ["pipewire-window-capture-source", "pipewire-screen-capture-source"]},
                },
                "CreateInput": {"requestStatus": {"result": True}, "responseData": {"sceneItemId": 7}},
                "GetSceneItemId": {"requestStatus": {"result": True}, "responseData": {"sceneItemId": 7}},
                "GetVideoSettings": {
                    "requestStatus": {"result": True},
                    "responseData": {"baseWidth": 1920, "baseHeight": 1080},
                },
                "GetSourceActive": {"requestStatus": {"result": True}, "responseData": {"videoActive": True}},
            }
        )
        self.recorder._client = fake
        picked = []
        self.recorder.prepare("Windows 11", "FuckExam", request_picker=picked.append)
        requested = [name for name, _ in fake.calls]
        self.assertIn("CreateSceneCollection", requested)
        self.assertIn("CreateScene", requested)
        self.assertEqual(requested.count("CreateInput"), 2)
        self.assertEqual(requested.count("SetSceneItemTransform"), 2)
        self.assertEqual(len(picked), 2)
        window_input = next(data for name, data in fake.calls if name == "CreateInput")
        self.assertEqual(window_input["inputKind"], "pipewire-window-capture-source")

    @mock.patch("FuckExam.recording._wayland", return_value=True)
    def test_window_kind_falls_back_to_unified_screen_source(self, _):
        fake = FakeObs(
            **{
                "GetInputKindList": {
                    "requestStatus": {"result": True},
                    "responseData": {"inputKinds": ["pipewire-screen-capture-source"]},
                }
            }
        )
        self.recorder._client = fake
        self.assertEqual(self.recorder._window_kind(), "pipewire-screen-capture-source")

    def test_layout_sends_flat_transform_keys(self):
        fake = FakeObs(
            **{
                "GetSceneItemId": {"requestStatus": {"result": True}, "responseData": {"sceneItemId": 3}},
                "SetSceneItemTransform": {"requestStatus": {"result": True}, "responseData": {}},
            }
        )
        self.recorder._client = fake
        self.recorder._layout("VM", True, (1920, 1080))
        name, data = fake.calls[-1]
        self.assertEqual(name, "SetSceneItemTransform")
        tr = data["sceneItemTransform"]
        self.assertEqual(tr["boundsType"], "OBS_BOUNDS_STRETCH")
        self.assertIn("positionX", tr)
        self.assertIn("positionY", tr)
        self.assertEqual(tr["boundsWidth"], 1152)
        self.assertEqual(tr["boundsHeight"], 928)
        self.assertNotIn("position", data["sceneItemTransform"])
        self.assertNotIn("bounds", data["sceneItemTransform"])

    def test_request_raises_on_obs_error(self):
        fake = FakeObs()
        fake.responses["GetVersion"] = {"requestStatus": {"result": False, "comment": "nope"}, "responseData": {}}
        self.recorder._client = fake
        with self.assertRaises(OBSRecordingError):
            self.recorder._request("GetVersion")

    def test_stop_reports_empty_output(self):
        output = self.root / "recordings" / "empty.mkv"
        output.write_bytes(b"")
        fake = FakeObs(
            **{
                "StopRecord": {
                    "requestStatus": {"result": True},
                    "responseData": {"outputPath": str(output)},
                },
                "GetRecordStatus": {
                    "requestStatus": {"result": True},
                    "responseData": {"recordingActive": False, "recordingPaused": False, "recordingTimecode": 0},
                },
            }
        )
        self.recorder._client = fake
        path, error = self.recorder.stop()
        self.assertEqual(path, output)
        self.assertIsNotNone(error)
        self.assertFalse(output.exists())

    def test_stop_reports_ok(self):
        output = self.root / "recordings" / "fine.mkv"
        output.write_bytes(b"\x00" * 4096)
        fake = FakeObs(
            **{
                "StopRecord": {
                    "requestStatus": {"result": True},
                    "responseData": {"outputPath": str(output)},
                },
                "GetRecordStatus": {
                    "requestStatus": {"result": True},
                    "responseData": {"recordingActive": False, "recordingPaused": False, "recordingTimecode": 0},
                },
            }
        )
        self.recorder._client = fake
        path, error = self.recorder.stop()
        self.assertEqual(path, output)
        self.assertIsNone(error)

    def test_record_active_accepts_output_active_key(self):
        self.assertTrue(self.recorder._record_active({"outputActive": True, "outputBytes": 123}))
        self.assertTrue(self.recorder._record_active({"recordingActive": True}))
        self.assertFalse(self.recorder._record_active({}))

    @mock.patch("FuckExam.recording.find_obs", return_value=None)
    def test_start_without_obs_raises(self, _):
        with self.assertRaises(OBSRecordingError):
            self.recorder.start("Windows 11", "FuckExam")

    def test_scene_preview_returns_decoded_png(self):
        png = base64.b64encode(b"\x89PNG\r\n\x1a\n" + b"0" * 200).decode("ascii")
        fake = FakeObs(
            **{
                "GetSourceScreenshot": {
                    "requestStatus": {"result": True},
                    "responseData": {"imageData": f"data:image/png;base64,{png}"},
                }
            }
        )
        self.recorder._client = fake
        data = self.recorder.scene_preview()
        self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
        request, payload = fake.calls[0]
        self.assertEqual(request, "GetSourceScreenshot")
        self.assertEqual(payload["sourceName"], "FuckExam")
        self.assertEqual(payload["imageWidth"], 640)

    def test_scene_preview_is_none_when_disconnected(self):
        self.recorder._client = None
        self.assertIsNone(self.recorder.scene_preview())

    def test_scene_preview_is_none_on_empty_image(self):
        fake = FakeObs(
            **{
                "GetSourceScreenshot": {
                    "requestStatus": {"result": True},
                    "responseData": {"imageData": ""},
                }
            }
        )
        self.recorder._client = fake
        self.assertIsNone(self.recorder.scene_preview())


if __name__ == "__main__":
    unittest.main()