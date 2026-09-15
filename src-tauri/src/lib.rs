pub mod capture;
pub mod window_capture;

use tauri::State;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(capture::Recorder::default())
        .invoke_handler(tauri::generate_handler![
            start_recording,
            stop_recording,
            recording_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}