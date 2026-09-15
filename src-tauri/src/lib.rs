pub mod capture;
pub mod streaming;
pub mod window_capture;

use std::sync::Arc;

use tauri::{AppHandle, State};

#[tauri::command]
async fn start_recording(state: State<'_, capture::Recorder>) -> Result<String, String> {
    capture::start_recording(&state).await
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
            start_recording,
            stop_recording,
            recording_status,
            stream_start,
            relay_connect,
            stream_status,
            stream_stop
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}