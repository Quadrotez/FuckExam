from __future__ import annotations

import io
import json
import os
import queue
import socket
import shutil
import struct
import subprocess
import sys
import threading
import time
import tempfile
from datetime import datetime
from pathlib import Path
import tkinter as tk
from tkinter import messagebox, ttk

try:
    from PIL import Image, ImageDraw, ImageGrab, ImageTk
except ImportError as exc:  # pragma: no cover
    raise RuntimeError("FuckExam requires Pillow: pip install Pillow") from exc

from .storage import AppStorage
from .virtualbox import VBoxManageClient, VirtualBoxError

BG = "#090909"
PANEL = "#141414"
PANEL_2 = "#1d1d1d"
RED = "#ef3030"
ORANGE = "#ff8a1f"
TEXT = "#f5f5f5"
MUTED = "#9a9a9a"
GREEN = "#54d68b"
HEADER = b"FEX1"


def runtime_root() -> Path:
    """Return a writable portable-data location next to the launched binary."""
    explicit_root = os.environ.get("FUCKEXAM_PORTABLE_ROOT")
    if explicit_root:
        return Path(explicit_root).resolve()
    appimage = os.environ.get("APPIMAGE")
    if appimage:
        return Path(appimage).resolve().parent
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path.cwd()


def pack_message(payload: dict) -> bytes:
    raw = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    return HEADER + struct.pack("!I", len(raw)) + raw


def recv_exact(sock: socket.socket, size: int) -> bytes:
    data = bytearray()
    while len(data) < size:
        chunk = sock.recv(size - len(data))
        if not chunk:
            raise ConnectionError("connection closed")
        data.extend(chunk)
    return bytes(data)


def recv_message(sock: socket.socket) -> dict:
    if recv_exact(sock, 4) != HEADER:
        raise ConnectionError("invalid protocol header")
    size = struct.unpack("!I", recv_exact(sock, 4))[0]
    if size > 16 * 1024 * 1024:
        raise ConnectionError("message too large")
    return json.loads(recv_exact(sock, size).decode("utf-8"))


def jpeg_bytes(image: Image.Image, quality: int = 70) -> bytes:
    buf = io.BytesIO()
    image.convert("RGB").save(buf, "JPEG", quality=quality, optimize=True)
    return buf.getvalue()


def wayland_capture(box: tuple[int, int, int, int]) -> Image.Image | None:
    x1, y1, x2, y2 = box
    width, height = x2 - x1, y2 - y1
    geometry = f"{x1},{y1} {width}x{height}"
    if shutil.which("grim"):
        result = subprocess.run(["grim", "-g", geometry, "-"], capture_output=True, check=False)
        if result.returncode == 0 and result.stdout:
            return Image.open(io.BytesIO(result.stdout)).convert("RGB")
    for command in ("gnome-screenshot", "spectacle"):
        if not shutil.which(command):
            continue
        with tempfile.NamedTemporaryFile(suffix=".png") as temp:
            if command == "gnome-screenshot":
                args = [command, "-f", temp.name]
            else:
                args = [command, "-b", "-n", "-o", temp.name]
            result = subprocess.run(args, capture_output=True, check=False)
            if result.returncode == 0 and Path(temp.name).exists():
                image = Image.open(temp.name).convert("RGB")
                return image.crop((x1, y1, min(x2, image.width), min(y2, image.height)))
    return None


def _window_geometry_xdotool(title: str) -> tuple[int, int, int, int] | None:
    if not shutil.which("xdotool"):
        return None
    result = subprocess.run(["xdotool", "search", "--onlyvisible", "--name", title], capture_output=True, text=True, check=False)
    window_ids = result.stdout.splitlines()
    if not window_ids:
        return None
    geometry = subprocess.run(["xdotool", "getwindowgeometry", "--shell", window_ids[-1]], capture_output=True, text=True, check=False)
    values = {}
    for line in geometry.stdout.splitlines():
        if "=" in line:
            key, value = line.split("=", 1)
            values[key] = int(value)
    if all(key in values for key in ("X", "Y", "WIDTH", "HEIGHT")):
        return values["X"], values["Y"], values["X"] + values["WIDTH"], values["Y"] + values["HEIGHT"]
    return None


def _window_geometry_hyprland(title: str) -> tuple[int, int, int, int] | None:
    if not shutil.which("hyprctl"):
        return None
    result = subprocess.run(["hyprctl", "clients", "-j"], capture_output=True, text=True, check=False)
    try:
        clients = json.loads(result.stdout)
    except json.JSONDecodeError:
        return None
    for client in clients:
        if title.casefold() in str(client.get("title", "")).casefold() or title.casefold() in str(client.get("class", "")).casefold():
            x, y = client.get("at", [0, 0])
            width, height = client.get("size", [0, 0])
            return int(x), int(y), int(x + width), int(y + height)
    return None


def resolve_window_box(title: str, fallback: tuple[int, int, int, int]) -> tuple[int, int, int, int]:
    """Resolve a live window rectangle; fallback remains available for unsupported compositors."""
    if not title.strip():
        return fallback
    return (_window_geometry_xdotool(title) or _window_geometry_hyprland(title) or fallback)


def capture_area(box: tuple[int, int, int, int], label: str, size: tuple[int, int]) -> Image.Image:
    try:
        if os.environ.get("WAYLAND_DISPLAY"):
            image = wayland_capture(box)
            if image is None:
                raise RuntimeError("Wayland capture unavailable: install grim or a desktop screenshot tool")
        else:
            image = ImageGrab.grab(bbox=box, all_screens=True)
        image.thumbnail(size, Image.Resampling.LANCZOS)
        canvas = Image.new("RGB", size, "#111111")
        canvas.paste(image, ((size[0] - image.width) // 2, (size[1] - image.height) // 2))
        return canvas
    except Exception:
        canvas = Image.new("RGB", size, "#181818")
        draw = ImageDraw.Draw(canvas)
        draw.text((24, size[1] // 2 - 12), f"{label} capture unavailable", fill=ORANGE)
        return canvas


def compose_frame(vm_box: tuple[int, int, int, int], app_box: tuple[int, int, int, int], vm_title: str = "", app_title: str = "") -> Image.Image:
    width, height = 960, 540
    vm_box = resolve_window_box(vm_title, vm_box)
    app_box = resolve_window_box(app_title, app_box)
    left = capture_area(vm_box, "VM", (620, 480))
    right = capture_area(app_box, "APP", (300, 480))
    frame = Image.new("RGB", (width, height), "#080808")
    frame.paste(left, (16, 44))
    frame.paste(right, (644, 44))
    draw = ImageDraw.Draw(frame)
    draw.rectangle((0, 0, width, 35), fill="#151515")
    draw.text((16, 10), "FUCKEXAM  /  LIVE SESSION", fill=TEXT)
    draw.text((644, 10), "APPLICATION", fill=ORANGE)
    draw.rectangle((0, 0, width - 1, height - 1), outline=RED, width=2)
    draw.line((628, 40, 628, height - 16), fill=ORANGE, width=2)
    return frame


class HostServer:
    def __init__(self, port: int, on_chat, on_status):
        self.port = port
        self.on_chat = on_chat
        self.on_status = on_status
        self.stop_event = threading.Event()
        self.client: socket.socket | None = None
        self.lock = threading.Lock()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.server: socket.socket | None = None

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.stop_event.set()
        for sock in (self.client, self.server):
            if sock:
                try:
                    sock.close()
                except OSError:
                    pass

    def send_frame(self, data: bytes) -> None:
        with self.lock:
            if not self.client:
                return
            try:
                self.client.sendall(pack_message({"type": "frame", "data": data.hex()}))
            except OSError:
                self.client = None
                self.on_status("viewer disconnected")

    def send_chat(self, text: str) -> None:
        with self.lock:
            if self.client:
                try:
                    self.client.sendall(pack_message({"type": "chat", "text": text, "from": "host"}))
                except OSError:
                    pass

    def _run(self) -> None:
        try:
            self.server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            self.server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            self.server.bind(("0.0.0.0", self.port))
            self.server.listen(1)
            self.server.settimeout(0.5)
            self.on_status(f"listening on port {self.port}")
            while not self.stop_event.is_set():
                try:
                    client, addr = self.server.accept()
                except socket.timeout:
                    continue
                with self.lock:
                    self.client = client
                self.on_status(f"viewer connected: {addr[0]}")
                try:
                    client.settimeout(0.5)
                    while not self.stop_event.is_set():
                        try:
                            message = recv_message(client)
                        except socket.timeout:
                            continue
                        if message.get("type") == "chat":
                            self.on_chat(message.get("text", ""), "viewer")
                except (OSError, ConnectionError):
                    pass
                finally:
                    with self.lock:
                        self.client = None
                    self.on_status("viewer disconnected")
        except OSError as exc:
            self.on_status(f"server error: {exc}")


class ViewerClient:
    def __init__(self, host: str, port: int, on_frame, on_chat, on_status):
        self.host, self.port = host, port
        self.on_frame, self.on_chat, self.on_status = on_frame, on_chat, on_status
        self.sock: socket.socket | None = None
        self.stop_event = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self.thread.start()

    def stop(self) -> None:
        self.stop_event.set()
        if self.sock:
            try:
                self.sock.close()
            except OSError:
                pass

    def send_chat(self, text: str) -> None:
        if self.sock:
            try:
                self.sock.sendall(pack_message({"type": "chat", "text": text, "from": "viewer"}))
            except OSError:
                self.on_status("send failed")

    def _run(self) -> None:
        try:
            self.sock = socket.create_connection((self.host, self.port), timeout=5)
            self.sock.settimeout(1)
            self.on_status("connected as viewer")
            while not self.stop_event.is_set():
                try:
                    message = recv_message(self.sock)
                except socket.timeout:
                    continue
                if message.get("type") == "frame":
                    self.on_frame(bytes.fromhex(message["data"]))
                elif message.get("type") == "chat":
                    self.on_chat(message.get("text", ""), "host")
        except (OSError, ConnectionError) as exc:
            self.on_status(f"viewer connection stopped: {exc}")


class CaptureRecorder:
    def __init__(self, root: Path, fps: int = 15):
        self.root = root
        self.fps = fps
        self.proc: subprocess.Popen | None = None
        self.path: Path | None = None
        self.frames: queue.Queue[bytes | None] = queue.Queue(maxsize=3)
        self.worker: threading.Thread | None = None
        self.error: str | None = None

    def start(self, size=(960, 540)) -> Path:
        if not shutil.which("ffmpeg"):
            raise RuntimeError("FFmpeg не найден в PATH. Установите пакет ffmpeg и перезапустите приложение.")
        folder = self.root / "recordings"
        folder.mkdir(parents=True, exist_ok=True)
        self.path = folder / f"session-{datetime.now():%Y%m%d-%H%M%S}.mp4"
        self.proc = subprocess.Popen([
            "ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24",
            "-s", f"{size[0]}x{size[1]}", "-r", str(self.fps), "-i", "-", "-an", "-c:v", "libx264",
            "-preset", "ultrafast", "-pix_fmt", "yuv420p", str(self.path)
        ], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.worker = threading.Thread(target=self._run, name="ffmpeg-writer", daemon=True)
        self.worker.start()
        return self.path

    def write(self, image: Image.Image) -> None:
        if self.proc and self.proc.poll() is None:
            try:
                self.frames.put_nowait(image.convert("RGB").tobytes())
            except queue.Full:
                pass

    def _run(self) -> None:
        try:
            while True:
                frame = self.frames.get()
                if frame is None:
                    break
                if self.proc and self.proc.stdin:
                    self.proc.stdin.write(frame)
                    self.proc.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            self.error = f"FFmpeg остановился: {exc}"

    def stop(self) -> None:
        if self.proc:
            self.frames.put(None)
            if self.worker:
                self.worker.join(timeout=5)
            if self.proc.stdin and not self.proc.stdin.closed:
                self.proc.stdin.close()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
            if self.proc.returncode and self.proc.stderr:
                details = self.proc.stderr.read().decode("utf-8", "replace").strip()
                self.error = details or f"FFmpeg завершился с кодом {self.proc.returncode}"
            self.proc = None
            self.worker = None


class FuckExamApp(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("FuckExam — Remote VM Lab")
        self.geometry("1180x760")
        self.minsize(980, 650)
        self.configure(bg=BG)
        self.storage = AppStorage(runtime_root() / "FuckExamData")
        self.server: HostServer | None = None
        self.viewer: ViewerClient | None = None
        self.recorder: CaptureRecorder | None = None
        self.running = False
        self.capture_thread: threading.Thread | None = None
        self.capture_stop = threading.Event()
        self.frame_queue: queue.Queue[Image.Image] = queue.Queue(maxsize=2)
        self.photo = None
        self._build_ui()
        self.protocol("WM_DELETE_WINDOW", self.close)
        self.after(60, self._render_loop)

    def _build_ui(self):
        style = ttk.Style(self)
        style.theme_use("clam")
        style.configure("TLabel", background=BG, foreground=TEXT)
        style.configure("TButton", background=PANEL_2, foreground=TEXT, padding=8)
        style.map("TButton", background=[("active", RED)])
        style.configure("TEntry", fieldbackground=PANEL_2, foreground=TEXT)
        style.configure("TNotebook", background=BG, borderwidth=0)
        style.configure("TNotebook.Tab", background=PANEL_2, foreground=TEXT, padding=(16, 8))

        header = tk.Frame(self, bg=BG)
        header.pack(fill="x", padx=24, pady=(20, 8))
        tk.Label(header, text="FUCKEXAM", bg=BG, fg=RED, font=("Arial", 23, "bold")).pack(side="left")
        tk.Label(header, text="  /  REMOTE VM OBSERVER", bg=BG, fg=ORANGE, font=("Arial", 11, "bold")).pack(side="left", pady=8)
        self.status = tk.Label(header, text="LOCAL PROTOTYPE", bg=BG, fg=MUTED, font=("Arial", 10))
        self.status.pack(side="right", pady=8)

        self.tabs = ttk.Notebook(self)
        self.tabs.pack(fill="both", expand=True, padx=20, pady=10)
        self.host_tab = tk.Frame(self.tabs, bg=BG)
        self.viewer_tab = tk.Frame(self.tabs, bg=BG)
        self.tabs.add(self.host_tab, text="  HOST / VM  ")
        self.tabs.add(self.viewer_tab, text="  VIEWER  ")
        self._build_host()
        self._build_viewer()

    def _panel(self, parent):
        return tk.Frame(parent, bg=PANEL, highlightbackground="#252525", highlightthickness=1)

    def _build_host(self):
        controls = self._panel(self.host_tab)
        controls.pack(side="left", fill="y", padx=(0, 14), pady=4)
        tk.Label(controls, text="BROADCAST CONTROL", bg=PANEL, fg=ORANGE, font=("Arial", 11, "bold")).pack(anchor="w", padx=18, pady=(18, 12))
        tk.Label(controls, text="VirtualBox VM", bg=PANEL, fg=MUTED).pack(anchor="w", padx=18, pady=(8, 3))
        vm_row = tk.Frame(controls, bg=PANEL)
        vm_row.pack(fill="x", padx=18)
        self.vm_name = ttk.Combobox(vm_row, state="readonly", width=22)
        self.vm_name.pack(side="left", ipady=5)
        tk.Button(vm_row, text="↻", command=self.refresh_vms, bg=ORANGE, fg="black", relief="flat", width=3).pack(side="left", padx=(6, 0), ipady=4)
        self.vm_hint = tk.Label(controls, text="Loading VM list…", bg=PANEL, fg=MUTED, wraplength=230, justify="left")
        self.vm_hint.pack(anchor="w", padx=18, pady=(4, 4))
        self.host_port = self._field(controls, "Port", "8765")
        self.host_fps = self._field(controls, "FPS", "15")
        self.vm_box = self._field(controls, "VM area x,y,width,height", "0,0,1280,720")
        self.app_box = self._field(controls, "App area x,y,width,height", "0,0,980,650")
        self.app_title = self._field(controls, "App window title", "FuckExam")
        self.record_var = tk.BooleanVar(value=True)
        tk.Checkbutton(controls, text="Record session locally", variable=self.record_var, bg=PANEL, fg=TEXT, selectcolor=PANEL_2, activebackground=PANEL, activeforeground=TEXT).pack(anchor="w", padx=18, pady=12)
        self.record_status = tk.Label(controls, text="Recording: ready", bg=PANEL, fg=MUTED, wraplength=230, justify="left")
        self.record_status.pack(anchor="w", padx=18, pady=(0, 8))
        self.host_button = tk.Button(controls, text="START BROADCAST", command=self.start_host, bg=RED, fg="white", activebackground=ORANGE, relief="flat", padx=12, pady=10)
        self.host_button.pack(fill="x", padx=18, pady=(8, 18))
        self.capture_status = tk.Label(controls, text="Capture: automatic backend", bg=PANEL, fg=MUTED, wraplength=230, justify="left")
        self.capture_status.pack(anchor="w", padx=18, pady=(0, 8))
        tk.Label(controls, text="Viewer connects to this computer\nusing its IP and port.", bg=PANEL, fg=MUTED, justify="left").pack(anchor="w", padx=18, pady=(0, 18))

        right = tk.Frame(self.host_tab, bg=BG)
        right.pack(side="left", fill="both", expand=True)
        self.host_preview = tk.Label(right, bg="#050505", text="Preview will appear here", fg=MUTED)
        self.host_preview.pack(fill="both", expand=True)
        self.host_chat = self._chat_panel(right, host=True)
        self.after(100, self.refresh_vms)

    def refresh_vms(self):
        self.vm_hint.configure(text="Loading VM list…", fg=MUTED)
        def load():
            try:
                vms = VBoxManageClient().list_vms()
                names = [vm.name for vm in vms]
                self.after(0, self._set_vms, names)
            except VirtualBoxError as exc:
                self.after(0, self._set_vms_error, str(exc))
        threading.Thread(target=load, name="vm-list", daemon=True).start()

    def _set_vms(self, names):
        self.vm_name["values"] = names
        if names:
            self.vm_name.current(0)
            self.vm_hint.configure(text=f"Found {len(names)} VM(s). Select one to start.", fg=GREEN)
        else:
            self.vm_hint.configure(text="No registered VMs found.", fg=ORANGE)

    def _set_vms_error(self, text):
        self.vm_name["values"] = []
        self.vm_hint.configure(text=text, fg=RED)

    def _build_viewer(self):
        controls = self._panel(self.viewer_tab)
        controls.pack(side="left", fill="y", padx=(0, 14), pady=4)
        tk.Label(controls, text="VIEW SESSION", bg=PANEL, fg=ORANGE, font=("Arial", 11, "bold")).pack(anchor="w", padx=18, pady=(18, 12))
        self.viewer_host = self._field(controls, "Host IP / hostname", "127.0.0.1")
        self.viewer_port = self._field(controls, "Port", "8765")
        self.viewer_button = tk.Button(controls, text="CONNECT AS VIEWER", command=self.start_viewer, bg=ORANGE, fg="black", activebackground=RED, relief="flat", padx=12, pady=10)
        self.viewer_button.pack(fill="x", padx=18, pady=(20, 18))
        tk.Label(controls, text="Viewer can send messages.\nHost cannot reply by design.", bg=PANEL, fg=MUTED, justify="left").pack(anchor="w", padx=18, pady=(0, 18))
        right = tk.Frame(self.viewer_tab, bg=BG)
        right.pack(side="left", fill="both", expand=True)
        self.viewer_preview = tk.Label(right, bg="#050505", text="Not connected", fg=MUTED)
        self.viewer_preview.pack(fill="both", expand=True)
        self.viewer_chat = self._chat_panel(right, host=False)

    def _field(self, parent, label, default):
        tk.Label(parent, text=label, bg=PANEL, fg=MUTED).pack(anchor="w", padx=18, pady=(8, 3))
        entry = tk.Entry(parent, bg=PANEL_2, fg=TEXT, insertbackground=ORANGE, relief="flat", width=25)
        entry.insert(0, default)
        entry.pack(anchor="w", padx=18, ipady=6)
        return entry

    def _chat_panel(self, parent, host: bool):
        panel = tk.Frame(parent, bg=PANEL)
        panel.pack(fill="x", pady=(10, 0))
        title = "INCOMING CHAT (VIEWER → HOST)" if host else "CHAT TO HOST"
        tk.Label(panel, text=title, bg=PANEL, fg=ORANGE, font=("Arial", 10, "bold")).pack(anchor="w", padx=12, pady=(8, 4))
        chat = tk.Text(panel, height=5, bg="#0c0c0c", fg=TEXT, insertbackground=TEXT, relief="flat", state="disabled")
        chat.pack(fill="x", padx=12)
        row = tk.Frame(panel, bg=PANEL)
        row.pack(fill="x", padx=12, pady=8)
        entry = tk.Entry(row, bg=PANEL_2, fg=TEXT, insertbackground=ORANGE, relief="flat")
        entry.pack(side="left", fill="x", expand=True, ipady=6)
        if host:
            entry.configure(state="disabled")
        else:
            tk.Button(row, text="SEND", command=lambda: self.send_viewer_chat(entry), bg=RED, fg="white", relief="flat").pack(side="right", padx=(8, 0), ipadx=8)
        return chat

    def _append_chat(self, widget, text, sender):
        widget.configure(state="normal")
        widget.insert("end", f"[{sender}] {text}\n")
        widget.see("end")
        widget.configure(state="disabled")

    def _parse_box(self, entry):
        values = [int(x.strip()) for x in entry.get().split(",")]
        if len(values) != 4:
            raise ValueError("capture box must be x,y,width,height")
        return values[0], values[1], values[0] + values[2], values[1] + values[3]

    def start_host(self):
        if self.running:
            self.stop_all()
            return
        try:
            port = int(self.host_port.get())
            fps = max(1, min(30, int(self.host_fps.get())))
            self.host_vm_box = self._parse_box(self.vm_box)
            self.host_app_box = self._parse_box(self.app_box)
            selected_vm = self.vm_name.get().strip()
            if not selected_vm:
                raise ValueError("Выберите VM из списка VirtualBox.")
        except ValueError as exc:
            messagebox.showerror("Invalid settings", str(exc))
            return
        self.server = HostServer(port, lambda text, sender: self.after(0, self._host_chat, text, sender), lambda text: self.after(0, self._set_status, text))
        self.server.start()
        self.recorder = CaptureRecorder(self.storage.root, fps=fps) if self.record_var.get() else None
        if self.recorder:
            try:
                path = self.recorder.start()
            except RuntimeError as exc:
                self.server.stop()
                self.server = None
                self.recorder = None
                messagebox.showerror("Recording unavailable", str(exc))
                return
            self.record_status.configure(text=f"Recording to:\n{path}", fg=GREEN)
        else:
            self.record_status.configure(text="Recording disabled", fg=MUTED)
        backend = "Wayland: grim/screenshot tool" if os.environ.get("WAYLAND_DISPLAY") else "X11: Pillow ImageGrab"
        self.capture_status.configure(text=f"Capture backend:\n{backend}", fg=ORANGE)
        self.running = True
        self.capture_stop.clear()
        self.capture_thread = threading.Thread(target=self._capture_loop, args=(selected_vm, self.app_title.get().strip(), fps), name="screen-capture", daemon=True)
        self.capture_thread.start()
        self.host_button.configure(text="STOP BROADCAST", bg="#7d1f1f")
        self._set_status(f"broadcasting {selected_vm} at {fps} FPS" + (f" / recording {path.name}" if self.recorder else ""))

    def _capture_loop(self, vm_title: str, app_title: str, fps: int):
        interval = 1.0 / fps
        last_geometry_at = 0.0
        vm_box, app_box = self.host_vm_box, self.host_app_box
        while self.running and not self.capture_stop.is_set():
            started = time.monotonic()
            if started - last_geometry_at >= 1.0:
                vm_box = resolve_window_box(vm_title, self.host_vm_box)
                app_box = resolve_window_box(app_title, self.host_app_box)
                last_geometry_at = started
            frame = compose_frame(vm_box, app_box)
            if self.recorder:
                self.recorder.write(frame)
            try:
                self.frame_queue.put_nowait(frame)
            except queue.Full:
                pass
            if self.server:
                self.server.send_frame(jpeg_bytes(frame))
            self.capture_stop.wait(max(0.001, interval - (time.monotonic() - started)))

    def start_viewer(self):
        if self.viewer:
            self.viewer.stop()
        try:
            port = int(self.viewer_port.get())
        except ValueError:
            messagebox.showerror("Invalid port", "Port must be a number")
            return
        self.viewer = ViewerClient(self.viewer_host.get().strip(), port, lambda data: self.after(0, self._viewer_frame, data), lambda text, sender: self.after(0, self._viewer_chat, text, sender), lambda text: self.after(0, self._set_status, text))
        self.viewer.start()

    def _viewer_frame(self, data):
        try:
            image = Image.open(io.BytesIO(data)).copy()
            self._show_image(self.viewer_preview, image)
        except Exception:
            pass

    def _render_loop(self):
        try:
            frame = self.frame_queue.get_nowait()
            self._show_image(self.host_preview, frame)
        except queue.Empty:
            pass
        self.after(60, self._render_loop)

    def _show_image(self, widget, image):
        width = max(widget.winfo_width(), 300)
        height = max(widget.winfo_height(), 220)
        image = image.copy()
        image.thumbnail((width - 8, height - 8), Image.Resampling.LANCZOS)
        self.photo = ImageTk.PhotoImage(image)
        widget.configure(image=self.photo, text="")

    def _host_chat(self, text, sender):
        self._append_chat(self.host_chat, text, sender)
        self.storage.save_message(sender, text)

    def _viewer_chat(self, text, sender):
        self._append_chat(self.viewer_chat, text, sender)

    def send_viewer_chat(self, entry):
        text = entry.get().strip()
        if text and self.viewer:
            self.viewer.send_chat(text)
            self._append_chat(self.viewer_chat, text, "you")
            entry.delete(0, "end")

    def _set_status(self, text):
        self.status.configure(text=text.upper(), fg=GREEN if "error" not in text.lower() else RED)

    def stop_all(self):
        self.running = False
        self.capture_stop.set()
        if self.capture_thread and self.capture_thread is not threading.current_thread():
            self.capture_thread.join(timeout=3)
        self.capture_thread = None
        if self.server:
            self.server.stop()
            self.server = None
        if self.viewer:
            self.viewer.stop()
            self.viewer = None
        record_error = None
        if self.recorder:
            self.recorder.stop()
            if self.recorder.error:
                record_error = self.recorder.error
                self._set_status(self.recorder.error)
                self.record_status.configure(text=f"Recording error:\n{self.recorder.error}", fg=RED)
            else:
                self.record_status.configure(text="Recording: saved", fg=GREEN)
            self.recorder = None
        self.host_button.configure(text="START BROADCAST", bg=RED)
        if not record_error:
            self._set_status("stopped")

    def close(self):
        self.stop_all()
        self.destroy()


def main() -> int:
    app = FuckExamApp()
    app.mainloop()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
