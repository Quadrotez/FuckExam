#[cfg(target_os = "linux")]
#[path = "capture_linux.rs"]
pub mod capture;
#[cfg(target_os = "windows")]
#[path = "capture_windows.rs"]
pub mod capture;

#[cfg(target_os = "linux")]
#[path = "window_capture_linux.rs"]
pub mod window_capture;

pub mod streaming;

use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

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
) -> Result<streaming::StreamStatus, String> {
    state.arm_local(app, port).await
}
#[tauri::command]
async fn relay_connect(
    app: AppHandle,
    state: State<'_, streaming::Streaming>,
    url: String,
    room: Option<String>,
) -> Result<streaming::StreamStatus, String> {
    state.arm_relay(app, url, room).await
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
        .build()
        .map(|_| ())
        .map_err(|e| format!("open chat window: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hub = Arc::new(streaming::StreamHub::default());
    tauri::Builder::default()
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
