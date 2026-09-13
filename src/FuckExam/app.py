from __future__ import annotations

import io
import json
import logging
import os
import queue
import socket
import struct
import sys
import threading
import time
import traceback
from pathlib import Path
import tkinter as tk
from tkinter import messagebox, ttk

from PIL import Image, ImageTk

from .recording import OBSRecorder, OBSRecordingError
from .storage import AppStorage
from .environment import component_present, detect_environment, install_missing

BG = "#090909"
PANEL = "#141414"
PANEL_2 = "#1d1d1d"
RED = "#ef3030"
ORANGE = "#ff8a1f"
TEXT = "#f5f5f5"
MUTED = "#9a9a9a"
GREEN = "#54d68b"
HEADER = b"FEX1"
PREVIEW_FPS = 4
APP_LOGGER: logging.Logger | None = None


def log_event(message: str, *args) -> None:
    if APP_LOGGER:
        APP_LOGGER.info(message, *args)


def runtime_root() -> Path:
    """Return a writable portable-data location next to the launched binary."""
    appimage = os.environ.get("APPIMAGE")
    if appimage:
        return Path(appimage).resolve().parent
    explicit_root = os.environ.get("FUCKEXAM_PORTABLE_ROOT")
    if explicit_root:
        return Path(explicit_root).resolve()
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


class FuckExamApp(tk.Tk):
    def __init__(self):
        super().__init__()
        self.title("FuckExam — Remote VM Lab")
        self.geometry("1180x760")
        self.minsize(980, 650)
        self.configure(bg=BG)
        self.runtime_dir = runtime_root()
        self.storage = AppStorage(self.runtime_dir / "FuckExamData")
        global APP_LOGGER
        APP_LOGGER = logging.getLogger("fuckexam")
        APP_LOGGER.setLevel(logging.INFO)
        APP_LOGGER.handlers.clear()
        self.storage.root.mkdir(parents=True, exist_ok=True)
        handler = logging.FileHandler(self.storage.root / "fuckexam-runtime.log", encoding="utf-8")
        handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
        APP_LOGGER.addHandler(handler)
        log_event("startup pid=%s cwd=%s runtime_dir=%s wayland=%s display=%s", os.getpid(), Path.cwd(), self.runtime_dir, bool(os.environ.get("WAYLAND_DISPLAY")), os.environ.get("WAYLAND_DISPLAY"))
        self.server: HostServer | None = None
        self.viewer: ViewerClient | None = None
        self.recorder: OBSRecorder | None = None
        self.running = False
        self.preview_thread: threading.Thread | None = None
        self.preview_stop = threading.Event()
        self.frame_queue: queue.Queue[Image.Image] = queue.Queue(maxsize=2)
        self._picker_queue: queue.Queue[tuple[str, threading.Event]] = queue.Queue()
        self.photo = None
        self._build_ui()
        self.protocol("WM_DELETE_WINDOW", self.close)
        self.after(60, self._render_loop)
        self.after(350, self.show_environment_check)

    def show_environment_check(self):
        report = detect_environment()
        self.environment_report = report
        dialog = tk.Toplevel(self)
        dialog.title("FuckExam — first launch check")
        dialog.geometry("700x560")
        dialog.configure(bg=BG)
        dialog.transient(self)
        dialog.grab_set()
        tk.Label(dialog, text="FIRST LAUNCH CHECK", bg=BG, fg=ORANGE, font=("Arial", 16, "bold")).pack(anchor="w", padx=22, pady=(20, 5))
        tk.Label(dialog, text=f"{report.distro}  /  {report.desktop}  /  {report.session}  /  {report.compositor}", bg=BG, fg=MUTED, wraplength=650, justify="left").pack(anchor="w", padx=22, pady=(0, 15))
        table = tk.Frame(dialog, bg=PANEL)
        table.pack(fill="both", expand=True, padx=22, pady=4)
        for component in report.components:
            present = component_present(component)
            color = GREEN if present else RED if component.required else ORANGE
            state = "OK" if present else "MISSING"
            row = tk.Frame(table, bg=PANEL)
            row.pack(fill="x", padx=14, pady=7)
            tk.Label(row, text=state, bg=color, fg="black", width=9, font=("Arial", 9, "bold")).pack(side="left")
            tk.Label(row, text=f"{component.name} — {component.purpose}", bg=PANEL, fg=TEXT, anchor="w").pack(side="left", padx=12)
        buttons = tk.Frame(dialog, bg=BG)
        buttons.pack(fill="x", padx=22, pady=18)
        if report.missing and report.install_command():
            tk.Button(buttons, text="INSTALL MISSING COMPONENTS", command=lambda: self.install_environment(dialog), bg=RED, fg="white", activebackground=ORANGE, relief="flat", padx=12, pady=9).pack(side="left")
        else:
            tk.Label(buttons, text="All required components are available.", bg=BG, fg=GREEN).pack(side="left")
        tk.Button(buttons, text="CLOSE", command=dialog.destroy, bg=PANEL_2, fg=TEXT, relief="flat", padx=16, pady=9).pack(side="right")

    def install_environment(self, dialog):
        report = getattr(self, "environment_report", detect_environment())
        self._set_status("installing system components")
        def run_install():
            ok, message = install_missing(report)
            self.after(0, lambda: (dialog.destroy(), messagebox.showinfo("Environment setup", message), self.show_environment_check()) if ok else messagebox.showerror("Environment setup", message))
        threading.Thread(target=run_install, name="environment-install", daemon=True).start()

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
        self.host_button = tk.Button(controls, text="START SESSION", command=self.start_host, bg=RED, fg="white", activebackground=ORANGE, relief="flat", padx=12, pady=9)
        self.host_button.pack(fill="x", padx=18, pady=(8, 3))
        self.host_port = self._field(controls, "Port", "8765")
        self.host_fps = self._field(controls, "FPS", "15")
        self.vm_title = self._field(controls, "VM window title (X11/Windows target)", "")
        self.app_title = self._field(controls, "App window title (X11/Windows target)", "FuckExam")
        self.record_status = tk.Label(controls, text="Recording: ready", bg=PANEL, fg=MUTED, wraplength=230, justify="left")
        self.record_status.pack(anchor="w", padx=18, pady=(0, 8))
        self.capture_status = tk.Label(controls, text="Capture: OBS Studio (websocket)", bg=PANEL, fg=MUTED, wraplength=230, justify="left")
        self.capture_status.pack(anchor="w", padx=18, pady=(0, 8))
        tk.Label(controls, text="Viewer connects to this computer\nusing its IP and port.", bg=PANEL, fg=MUTED, justify="left").pack(anchor="w", padx=18, pady=(0, 18))

        right = tk.Frame(self.host_tab, bg=BG)
        right.pack(side="left", fill="both", expand=True)
        self.host_preview = tk.Label(right, bg="#050505", text="Preview will appear here", fg=MUTED)
        self.host_preview.pack(fill="both", expand=True)
        self.host_chat = self._chat_panel(right, host=True)

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

    def start_host(self):
        if self.running:
            self.stop_all()
            return
        try:
            port = int(self.host_port.get())
            fps = max(1, min(30, int(self.host_fps.get())))
            if not OBSRecorder.available():
                raise RuntimeError("OBS Studio не найден. Установите OBS через FIRST LAUNCH CHECK и перезапустите приложение.")
        except (ValueError, RuntimeError) as exc:
            messagebox.showerror("Invalid settings", str(exc))
            return
        if not self.recorder or not self.recorder.recording:
            self.recorder = OBSRecorder(self.storage.root, fps=fps)
        vm_title = self.vm_title.get().strip()
        app_title = self.app_title.get().strip()
        backend = "OBS Studio (websocket)"
        log_event("start_session port=%s fps=%s vm_title=%r app_title=%r recording=obs %s", port, fps, vm_title, app_title, "wayland-portal" if os.environ.get("WAYLAND_DISPLAY") else "native-window-capture")
        self.capture_status.configure(text=f"Capture backend:\n{backend}", fg=ORANGE)
        self.running = True
        self.preview_stop.clear()
        if os.environ.get("WAYLAND_DISPLAY"):
            self.capture_status.configure(text="Capture backend:\nOBS + Wayland portal — native windows", fg=GREEN)
            log_event("preview via OBS scene snapshots (Wayland portal)")
        else:
            log_event("preview via OBS scene snapshots (window capture)")
        self.host_preview.configure(image="", text="Starting live preview…", fg=GREEN)
        self.preview_thread = threading.Thread(target=self._obs_preview_loop, name="obs-preview", daemon=True)
        self.preview_thread.start()
        self.server = HostServer(port, lambda text, sender: self.after(0, self._host_chat, text, sender), lambda text: self.after(0, self._set_status, text))
        self.server.start()
        self.host_button.configure(text="STOP SESSION", bg="#7d1f1f")
        self.record_status.configure(text="Recording: starting OBS session…", fg=ORANGE)
        self._set_status("starting session (OBS recording)")
        self.after(250, self.start_recording)

    def toggle_recording(self):
        if self.recorder and self.recorder.recording:
            self.stop_recording()
        else:
            self.start_recording()

    def _picker_dialog(self, text: str):
        done = threading.Event()
        self._picker_queue.put((text, done))
        while self.running and not done.is_set():
            done.wait(0.2)
        return done

    def _process_picker_queue(self):
        try:
            text, done = self._picker_queue.get_nowait()
        except queue.Empty:
            return
        messagebox.showinfo("FuckExam — window picker", text)
        done.set()

    def start_recording(self):
        if not self.running:
            messagebox.showinfo("Start broadcast first", "Сначала запустите трансляцию, затем запись.")
            return
        if self.recorder and self.recorder.recording:
            return
        if not self.recorder:
            try:
                fps = max(1, min(30, int(self.host_fps.get())))
            except ValueError:
                fps = 15
            self.recorder = OBSRecorder(self.storage.root, fps=fps)
        self.record_status.configure(text="Recording: preparing OBS…", fg=ORANGE)
        threading.Thread(target=self._recording_worker_start, name="recording-start", daemon=True).start()

    def _recording_worker_start(self):
        recorder = self.recorder
        if not self.running:
            return
        try:
            path = recorder.start(
                self.vm_title.get().strip(),
                self.app_title.get().strip(),
                request_picker=self._picker_dialog,
            )
        except OBSRecordingError as exc:
            message = str(exc)
            log_event("recording start failed: %s", exc)
            self.after(0, lambda msg=message: self._recording_failed(msg))
            return
        except Exception:
            log_event("recording start crashed:\n%s", traceback.format_exc())
            self.after(0, lambda: self._recording_failed("Непредвиденная ошибка записи.\nПодробности в FuckExamData/fuckexam-runtime.log"))
            return
        if not self.running:
            recorder.stop()
            recorder.close()
            return
        self.after(0, lambda folder=path: self._recording_started(folder))

    def _recording_failed(self, message: str):
        self.record_status.configure(text=f"Recording unavailable:\n{message}", fg=RED)
        self._set_status("recording unavailable")

    def _recording_started(self, folder: Path):
        self.record_status.configure(text=f"Recording:\nOBS → {folder}", fg=GREEN)
        self._set_status("recording via OBS")

    def stop_recording(self):
        recorder = self.recorder
        if not recorder:
            return
        self.record_status.configure(text="Recording: stopping…", fg=ORANGE)
        threading.Thread(target=self._recording_worker_stop, args=(recorder,), name="recording-stop", daemon=True).start()

    def _recording_worker_stop(self, recorder):
        try:
            output_path, error = recorder.stop()
        except OBSRecordingError as exc:
            output_path, error = None, str(exc)
        self.after(0, lambda: self._recording_stopped(output_path, error))

    def _recording_stopped(self, output_path, error):
        if output_path and output_path.exists():
            self.record_status.configure(text=f"Recording saved:\n{output_path}", fg=GREEN)
            self._set_status("recording saved")
        else:
            if not error:
                error = "Файл записи не найден."
            self.record_status.configure(text=f"Recording error:\n{error}", fg=RED)
            self._set_status("recording error")

    def _obs_preview_loop(self):
        interval = 1.0 / PREVIEW_FPS
        while self.running and not self.preview_stop.is_set():
            started = time.monotonic()
            data = self.recorder.scene_preview(640, 360) if self.recorder else None
            if data:
                try:
                    image = Image.open(io.BytesIO(data)).copy()
                    try:
                        self.frame_queue.put_nowait(image)
                    except queue.Full:
                        pass
                    if self.server:
                        self.server.send_frame(data)
                except Exception:
                    pass
            self.preview_stop.wait(max(0.001, interval - (time.monotonic() - started)))

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
        self._process_picker_queue()
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
        self.preview_stop.set()
        if self.preview_thread and self.preview_thread is not threading.current_thread():
            self.preview_thread.join(timeout=3)
        self.preview_thread = None
        if self.server:
            self.server.stop()
            self.server = None
        if self.viewer:
            self.viewer.stop()
            self.viewer = None
        recorder = self.recorder
        if recorder:
            if recorder.recording:
                output_path, error = recorder.stop()
                self._recording_stopped(output_path, error)
            recorder.close()
            self.recorder = None
        self.host_button.configure(text="START SESSION", bg=RED)
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
