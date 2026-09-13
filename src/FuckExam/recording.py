from __future__ import annotations

import base64
import json
import logging
import os
import secrets
import shutil
import socket
import string
import subprocess
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path

try:
    from obsws_python import ReqClient
except ImportError as exc:  # pragma: no cover
    raise RuntimeError("FuckExam requires obsws-python: pip install obsws-python websocket-client") from exc

LOGGER = logging.getLogger("fuckexam")

COLLECTION_NAME = "FuckExam"
SCENE_NAME = "FuckExam"
VM_SOURCE = "FuckExam-VM"
APP_SOURCE = "FuckExam-App"
WS_PORT = 4455
LAUNCH_TIMEOUT = 40.0
PORTAL_WAIT = 150.0

WS_SCREEN_WINDOW_KIND = "pipewire-window-capture-source"
WS_SCREEN_KIND = "pipewire-screen-capture-source"
WIN_WINDOW_KIND = "windows_capture"
X11_WINDOW_KIND = "xcomposite_input"


class OBSRecordingError(RuntimeError):
    pass


def _wayland() -> bool:
    return bool(os.environ.get("WAYLAND_DISPLAY"))


def _port_open(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5):
            return True
    except OSError:
        return False


def _obs_running() -> bool:
    try:
        if sys.platform == "win32":
            result = subprocess.run(
                ["tasklist", "/FI", "IMAGENAME eq obs64.exe"], capture_output=True, text=True, check=False
            )
            return "obs64.exe" in result.stdout
        result = subprocess.run(["pgrep", "-x", "obs"], capture_output=True, check=False)
        return result.returncode == 0
    except OSError:
        return False


def find_obs() -> str | None:
    for name in ("obs", "obs64"):
        path = shutil.which(name)
        if path:
            return path
    if sys.platform == "win32":
        for candidate in (
            Path(r"C:\Program Files\obs-studio\bin\64bit\obs64.exe"),
            Path(r"C:\Program Files (x86)\obs-studio\bin\32bit\obs32.exe"),
        ):
            if candidate.exists():
                return str(candidate)
    elif sys.platform == "darwin":
        candidate = Path("/Applications/OBS.app/Contents/MacOS/obs")
        if candidate.exists():
            return str(candidate)
    return None


def _config_file() -> Path:
    if sys.platform == "win32":
        base = os.environ.get("APPDATA", str(Path.home() / "AppData" / "Roaming"))
    elif sys.platform == "darwin":
        base = str(Path.home() / "Library" / "Application Support")
    else:
        base = os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))
    return Path(base) / "obs-studio" / "plugin_config" / "obs-websocket" / "config.json"


class OBSRecorder:
    """Record the VM and app windows through OBS Studio + obs-websocket.

    OBS is the only cross-platform capture/encode engine. Window sources that
    require the XDG portal are remembered between sessions by the portal
    restore token, so the picker is shown only on first setup.
    """

    def __init__(self, root: Path, fps: int = 15, port: int = WS_PORT):
        self.root = root
        self.fps = max(1, int(fps))
        self.port = port
        self.password = ""
        self.recordings = root / "recordings"
        self.recordings.mkdir(parents=True, exist_ok=True)
        self.process: subprocess.Popen | None = None
        self._client: ReqClient | None = None
        self._launched_by_us = False
        self.error: str | None = None
        self.last_output: Path | None = None
        self.recording = False
        self._sources_ready = False
        self._kinds: list[str] | None = None
        self._request_lock = threading.Lock()

    @staticmethod
    def available() -> bool:
        return find_obs() is not None

    # ------------------------------------------------------------------ #
    # lifecycle                                                           #
    # ------------------------------------------------------------------ #

    def ensure_websocket_config(self) -> None:
        path = _config_file()
        data: dict = {}
        if path.exists():
            try:
                data = json.loads(path.read_text(encoding="utf-8"))
            except json.JSONDecodeError as exc:
                raise OBSRecordingError(f"Повреждён файл настроек OBS websocket: {path}") from exc
        else:
            LOGGER.info("websocket config absent, generating at %s", path)
        password = data.get("server_password")
        if not password:
            password = "".join(secrets.choice(string.ascii_letters + string.digits) for _ in range(24))
        data.update(
            {
                "server_enabled": True,
                "server_port": self.port,
                "server_password": password,
                "auth_required": data.get("auth_required", True),
                "alerts_enabled": data.get("alerts_enabled", False),
                "first_load": data.get("first_load", False),
            }
        )
        self.password = str(password)
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(data, indent=2), encoding="utf-8")
        tmp.replace(path)
        LOGGER.info("websocket config ensured at %s port=%s", path, self.port)

    def connect(self, timeout: float = LAUNCH_TIMEOUT) -> None:
        if self._client is not None:
            return
        obs = find_obs()
        if not obs:
            raise OBSRecordingError("OBS не найден. Установите OBS Studio и перезапустите приложение.")
        self.ensure_websocket_config()
        if not _port_open(self.port):
            if _obs_running():
                raise OBSRecordingError(
                    "OBS уже запущен без websocket-сервера. Закройте OBS полностью и повторите запись."
                )
            self._launched_by_us = True
            self.process = subprocess.Popen(
                [obs, "--minimize-to-tray", "--disable-missing-files-check"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            LOGGER.info("launched obs pid=%s waiting for websocket", self.process.pid)
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if _port_open(self.port):
                    break
                if self.process.poll() is not None:
                    raise OBSRecordingError(f"OBS завершился при запуске (код {self.process.returncode}).")
                time.sleep(0.4)
            if not _port_open(self.port):
                raise OBSRecordingError("OBS запустился, но websocket-сервер не отвечает.")
        try:
            self._client = ReqClient(host="127.0.0.1", port=self.port, password=self.password, timeout=5)
            self._request("GetVersion")
        except Exception as exc:
            self._client = None
            raise OBSRecordingError(f"Не удалось подключиться к OBS websocket: {exc}") from exc
        LOGGER.info("connected to OBS websocket on port %s", self.port)

    def _request(self, request: str, data: dict | None = None) -> dict:
        with self._request_lock:
            if self._client is None:
                raise OBSRecordingError("OBS websocket не подключён.")
            try:
                return self._client.send(request, data or {}, raw=True)
            except Exception as exc:
                raise OBSRecordingError(f"OBS не выполнил {request}: {exc}") from exc

    # ------------------------------------------------------------------ #
    # scene / sources                                                     #
    # ------------------------------------------------------------------ #

    def _prepare_scene(self) -> tuple[int, int]:
        collections = self._request("GetSceneCollectionList").get("sceneCollections", [])
        if COLLECTION_NAME not in collections:
            self._request("CreateSceneCollection", {"sceneCollectionName": COLLECTION_NAME})
        scenes = self._request("GetSceneList").get("scenes", [])
        if not any(item.get("sceneName") == SCENE_NAME for item in scenes):
            self._request("CreateScene", {"sceneName": SCENE_NAME})
        self._request("SetCurrentProgramScene", {"sceneName": SCENE_NAME})
        video = self._request("GetVideoSettings")
        return int(video.get("baseWidth", 1920)), int(video.get("baseHeight", 1080))

    @staticmethod
    def _input_kind() -> str:
        if _wayland():
            return WS_SCREEN_WINDOW_KIND
        if sys.platform == "win32":
            return WIN_WINDOW_KIND
        return X11_WINDOW_KIND

    def _available_kinds(self) -> list[str]:
        if self._kinds is None:
            self._kinds = self._request("GetInputKindList", {}).get("inputKinds", [])
        return self._kinds

    def _window_kind(self) -> str:
        if not _wayland():
            return self._input_kind()
        kinds = self._available_kinds()
        for kind in (WS_SCREEN_WINDOW_KIND, WS_SCREEN_KIND):
            if kind in kinds:
                return kind
        raise OBSRecordingError("В этом OBS нет PipeWire источника для захвата окон.")

    def _source_settings(self, title: str) -> dict:
        if _wayland():
            return {"RestoreToken": ""}
        if sys.platform == "win32":
            return {"capture_mode": 0, "capture_cursor": True, "window": title}
        return {"capture_window": title}

    def _existing_input(self, name: str) -> dict | None:
        for item in self._request("GetInputList", {}).get("inputs", []):
            if item.get("inputName") == name:
                return item
        return None

    def _scene_item_id(self, source_name: str) -> int:
        data = self._request("GetSceneItemId", {"sceneName": SCENE_NAME, "sourceName": source_name})
        return int(data.get("sceneItemId", -1))

    def _create_input(self, name: str, title: str) -> int:
        kind = self._window_kind()
        settings = self._source_settings(title)
        resp = self._request(
            "CreateInput",
            {
                "sceneName": SCENE_NAME,
                "inputName": name,
                "inputKind": kind,
                "inputSettings": settings,
                "sceneItemEnabled": True,
            },
        )
        return int(resp.get("sceneItemId", -1))

    def _has_portal_token(self, name: str) -> bool:
        if not _wayland():
            return False
        try:
            settings = self._request("GetInputSettings", {"inputName": name}).get("inputSettings", {})
        except OBSRecordingError:
            return False
        return bool(settings.get("RestoreToken"))

    def _wait_active(self, name: str, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                data = self._request("GetSourceActive", {"sourceName": name})
            except OBSRecordingError:
                data = {}
            if data.get("videoActive"):
                LOGGER.info("source %s is active", name)
                return
            time.sleep(0.5)
        raise OBSRecordingError(
            f"Источник {name} не запустился. Окно не выбрано в диалоге OBS или отменено."
        )

    def _ensure_source(self, name: str, title: str, request_picker: Callable[[str], None] | None) -> int:
        existing = self._existing_input(name)
        if existing is not None:
            needs_pick = _wayland() and not self._has_portal_token(name)
            if needs_pick:
                if request_picker:
                    request_picker(
                        f"Выберите окно «{title}» в системном диалоге OBS.\n\n"
                        "Этот выбор сохранится для следующих сессий."
                    )
                self._request("RemoveInput", {"inputName": name})
                return self._create_input(name, title)
            if not _wayland():
                self._request(
                    "SetInputSettings",
                    {"inputName": name, "inputSettings": self._source_settings(title), "overlay": True},
                )
            return self._scene_item_id(name)
        if _wayland() and request_picker:
            request_picker(
                f"Выберите окно «{title}» в системном диалоге OBS.\n\n"
                "Этот выбор сохранится для следующих сессий."
            )
        elif not _wayland() and request_picker:
            request_picker(f"Настраиваю захват окна «{title}» через OBS.")
        return self._create_input(name, title)

    def _layout(self, source_name: str, left: bool, canvas: tuple[int, int]) -> None:
        width, height = canvas
        side = int(width * 0.03)
        top = int(height * 0.06)
        body = int(height * 0.86)
        if left:
            bounds_width = int(width * 0.60)
            x, y = side, top
        else:
            bounds_width = int(width * 0.28)
            x, y = width - side - bounds_width, top
        item_id = self._scene_item_id(source_name)
        if item_id < 0:
            raise OBSRecordingError(f"Не найдено поле сцены для {source_name}.")
        transform = {
            "boundsType": "OBS_BOUNDS_STRETCH",
            "boundsAlignment": 5,
            "positionX": x,
            "positionY": y,
            "boundsWidth": bounds_width,
            "boundsHeight": body,
            "rotation": 0.0,
            "scaleX": 1.0,
            "scaleY": 1.0,
        }
        self._request(
            "SetSceneItemTransform",
            {
                "sceneName": SCENE_NAME,
                "sceneItemId": item_id,
                "sceneItemTransform": transform,
            },
        )

    def prepare(
        self,
        vm_title: str,
        app_title: str,
        request_picker: Callable[[str], None] | None = None,
    ) -> None:
        if not self._sources_ready:
            canvas = self._prepare_scene()
            vm_id = self._ensure_source(VM_SOURCE, vm_title, request_picker)
            if vm_id >= 0:
                self._wait_active(VM_SOURCE, PORTAL_WAIT if _wayland() else 30.0)
                self._layout(VM_SOURCE, True, canvas)
            app_id = self._ensure_source(APP_SOURCE, app_title, request_picker)
            if app_id >= 0:
                self._wait_active(APP_SOURCE, PORTAL_WAIT if _wayland() else 30.0)
                self._layout(APP_SOURCE, False, canvas)
            self._sources_ready = True

    # ------------------------------------------------------------------ #
    # record                                                              #
    # ------------------------------------------------------------------ #

    @staticmethod
    def _record_active(status: dict) -> bool:
        return bool(status.get("recordingActive") or status.get("outputActive"))

    def scene_preview(self, width: int = 640, height: int = 360) -> bytes | None:
        """Live PNG snapshot of the composed FuckExam scene, or None if the
        scene/websocket are not ready yet."""
        try:
            data = self._request(
                "GetSourceScreenshot",
                {
                    "sourceName": SCENE_NAME,
                    "imageFormat": "png",
                    "imageWidth": width,
                    "imageHeight": height,
                },
            )
            image = data.get("imageData", "")
            if not image:
                return None
            return base64.b64decode(image.split(",", 1)[1])
        except (OBSRecordingError, ValueError, IndexError):
            return None

    def start(self, vm_title: str, app_title: str, request_picker: Callable[[str], None] | None = None) -> Path:
        self.error = None
        self.last_output = None
        self.connect()
        self.prepare(vm_title, app_title, request_picker)
        self._request("SetRecordDirectory", {"recordDirectory": str(self.recordings)})
        self._request("StartRecord")
        self.recording = True
        deadline = time.monotonic() + 10.0
        while time.monotonic() < deadline:
            status = self._request("GetRecordStatus")
            if self._record_active(status):
                LOGGER.info("OBS recording started into %s", self.recordings)
                return self.recordings
            time.sleep(0.25)
        self._request("StopRecord")
        self.recording = False
        raise OBSRecordingError("OBS не смог стартовать локальную запись.")

    def stop(self) -> tuple[Path | None, str | None]:
        self.error = None
        output_path: Path | None = None
        if self._client is not None:
            try:
                data = self._request("StopRecord")
                raw = data.get("outputPath")
                if raw:
                    output_path = Path(raw)
            except OBSRecordingError as exc:
                self.error = str(exc)
            try:
                deadline = time.monotonic() + 8.0
                while time.monotonic() < deadline:
                    status = self._request("GetRecordStatus")
                    if not self._record_active(status):
                        break
                    time.sleep(0.25)
            except OBSRecordingError:
                pass
        self.recording = False
        if output_path is None:
            output_path = self._newest_recording()
        self.last_output = output_path
        if output_path is None or not output_path.exists():
            if not self.error:
                self.error = "Файл записи не создан."
        elif output_path.stat().st_size == 0:
            if not self.error:
                self.error = "Файл записи пуст (запись не получила кадров)."
            output_path.unlink(missing_ok=True)
            return output_path, self.error
        return output_path, self.error

    def _newest_recording(self) -> Path | None:
        try:
            files = [item for item in self.recordings.iterdir() if item.is_file()]
        except OSError:
            return None
        if not files:
            return None
        return max(files, key=lambda item: item.stat().st_mtime)

    def close(self) -> None:
        try:
            if self.recording:
                self.stop()
        except Exception:
            LOGGER.exception("error while stopping recording on close")
        if self._client is not None:
            try:
                self._client.disconnect()
            except Exception:
                LOGGER.exception("websocket disconnect failed")
            self._client = None
        if self._launched_by_us and self.process is not None:
            LOGGER.info("closing OBS launched by the app")
            try:
                self.process.terminate()
                self.process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
            except Exception:
                LOGGER.exception("failed to stop OBS")
            self.process = None
            self._launched_by_us = False