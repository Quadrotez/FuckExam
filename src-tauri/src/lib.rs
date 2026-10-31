#[cfg(target_os = "linux")]
#[path = "capture_linux.rs"]
pub mod capture;
#[cfg(target_os = "windows")]
#[path = "capture_windows.rs"]
pub mod capture;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[path = "capture_mobile.rs"]
pub mod capture;

pub mod diag;

#[cfg(target_os = "linux")]
#[path = "window_capture_linux.rs"]
pub mod window_capture;

pub mod streaming;

use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[cfg(desktop)]
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

#[cfg(desktop)]
const APP_ICON: tauri::image::Image<'static> = tauri::include_image!("./icons/icon.png");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub os: String,
    pub recorder: bool,
    pub relay: bool,
}

#[tauri::command]
fn platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS.to_string(),
        recorder: cfg!(any(target_os = "linux", target_os = "windows")),
        relay: true,
    }
}

#[tauri::command]
async fn start_recording(
    app: AppHandle,
    state: State<'_, capture::Recorder>,
) -> Result<String, String> {
    capture::start_recording(&state, Some(&app)).await
}
#[tauri::command]
fn stop_recording(state: State<'_, capture::Recorder>) -> Result<String, String> {
    capture::stop_recording(&state)
}
#[tauri::command]
fn recording_status(state: State<'_, capture::Recorder>) -> capture::RecordingSnapshot {
    capture::status(&state)
}
#[tauri::command]
async fn stream_start(
    app: AppHandle,
    state: State<'_, streaming::Streaming>,
    port: Option<u16>,
    fps: Option<u32>,
    quality: Option<u8>,
    max_width: Option<usize>,
) -> Result<streaming::StreamStatus, String> {
    let config = streaming::StreamConfig {
        fps: fps.unwrap_or(0),
        quality: quality.unwrap_or(0),
        max_width: max_width.unwrap_or(0),
    };
    state.arm_local(app, port, config).await
}
#[tauri::command]
async fn relay_connect(
    app: AppHandle,
    state: State<'_, streaming::Streaming>,
    url: String,
    fps: Option<u32>,
    quality: Option<u8>,
    max_width: Option<usize>,
) -> Result<streaming::StreamStatus, String> {
    let config = streaming::StreamConfig {
        fps: fps.unwrap_or(0),
        quality: quality.unwrap_or(0),
        max_width: max_width.unwrap_or(0),
    };
    state.arm_relay(app, url, config).await
}
#[tauri::command]
fn stream_status(state: State<'_, streaming::Streaming>) -> streaming::StreamStatus {
    state.0.status()
}
#[tauri::command]
fn stream_stop(state: State<'_, streaming::Streaming>) {
    state.0.deactivate();
}
#[tauri::command]
async fn relay_ping(url: String) -> Result<String, String> {
    streaming::ping_relay(url).await
}

#[tauri::command]
async fn chat_send(
    state: State<'_, streaming::Streaming>,
    text: String,
    markdown: bool,
    image: Option<streaming::ChatImage>,
) -> Result<(), String> {
    state.0.send_chat(text, markdown, image).await
}

#[tauri::command]
fn chat_history(state: State<'_, streaming::Streaming>) -> Vec<streaming::ChatMsg> {
    state.0.chat_history()
}

#[cfg(desktop)]
#[tauri::command]
async fn open_chat_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("chat") {
        window.show().map_err(|e| format!("show chat window: {e}"))?;
        window
            .set_focus()
            .map_err(|e| format!("focus chat window: {e}"))?;
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, "chat", WebviewUrl::App("index.html".into()))
        .title("FuckExam — Чат")
        .inner_size(420.0, 620.0)
        .min_inner_size(320.0, 320.0)
        .resizable(true)
        .center()
        .icon(APP_ICON.clone())
        .map_err(|e| format!("set chat icon: {e}"))?
        .build()
        .map(|_| ())
        .map_err(|e| format!("open chat window: {e}"))
}

/// На Android отдельное окно открыть нельзя — там чат живёт на том же экране.
#[cfg(not(desktop))]
#[tauri::command]
async fn open_chat_window() -> Result<(), String> {
    Err("отдельное окно чата недоступно на этой платформе".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    diag::install_panic_hook();
    diag::log_environment();

    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        // KDE/Wayland normally provides XWayland. Using it here gives the
        // window a real native taskbar icon while keeping multiple processes
        // possible (GTK application IDs are deliberately disabled).
        std::env::set_var("GDK_BACKEND", "x11");
    }

    let hub = Arc::new(streaming::StreamHub::default());
    let result = tauri::Builder::default()
        .setup(|app| {
            #[cfg(desktop)]
            {
                // Окно создаётся из tauri.conf.json (так же, как на Android).
                // Иконку ставим здесь: на Linux без неё окно в панели задач безымянное.
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_icon(APP_ICON.clone());
                }
            }
            #[cfg(not(desktop))]
            {
                let _ = app;
            }
            Ok(())
        })
        .manage(capture::Recorder {
            inner: std::sync::Mutex::new(None),
            hub: hub.clone(),
        })
        .manage(streaming::Streaming(hub))
        .invoke_handler(tauri::generate_handler![
            platform_info,
            start_recording,
            stop_recording,
            recording_status,
            stream_start,
            relay_connect,
            stream_status,
            stream_stop,
            relay_ping,
            chat_send,
            chat_history,
            open_chat_window
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => diag::log("exit: clean"),
        Err(e) => {
            #[cfg(not(target_os = "android"))]
            let text = format!(
                "не удалось запустить приложение: {e}\nЛог: {}",
                diag::log_path().display()
            );
            #[cfg(target_os = "android")]
            let text = format!("не удалось запустить приложение: {e}");
            diag::report_fatal(&text);
        }
    }
}
