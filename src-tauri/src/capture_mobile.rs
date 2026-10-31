//! Заглушка записи для платформ без поддержки (Android и прочие мобильные).
//!
//! Android-версия — зритель + чат. Запись экрана телефона сюда не входит,
//! поэтому все команды записи честно возвращают ошибку, а UI их прячет.

use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::streaming::StreamHub;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct WindowInfo {
    pub node_id: u32,
    #[serde(rename = "width")]
    pub width: i32,
    #[serde(rename = "height")]
    pub height: i32,
    pub output: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSnapshot {
    pub running: bool,
    pub windows: Vec<WindowInfo>,
    pub error: Option<String>,
}

pub struct Recorder {
    pub inner: Mutex<Option<()>>,
    #[allow(dead_code)]
    pub hub: Arc<StreamHub>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            hub: Arc::new(StreamHub::default()),
        }
    }
}

pub async fn start_recording(
    _state: &Recorder,
    _app: Option<&tauri::AppHandle>,
) -> Result<String, String> {
    Err("запись не поддерживается на этой платформе".into())
}

pub fn stop_recording(_state: &Recorder) -> Result<String, String> {
    Err("запись не поддерживается на этой платформе".into())
}

pub fn status(_state: &Recorder) -> RecordingSnapshot {
    RecordingSnapshot {
        running: false,
        windows: Vec::new(),
        error: Some("запись не поддерживается на этой платформе".into()),
    }
}

pub fn run_cli_record() {
    eprintln!("запись не поддерживается на этой платформе");
}
