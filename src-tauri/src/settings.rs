use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 7335;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub port: u16,
    pub fps: u32,
    pub quality: u8,
    pub max_width: usize,
    pub relay_url: String,
    pub mode: String,
    pub web_on_launch: bool,
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            fps: 24,
            quality: 75,
            max_width: 1280,
            relay_url: String::new(),
            mode: "local".into(),
            web_on_launch: true,
            close_to_tray: true,
        }
    }
}

impl Settings {
    /// Приводит значения к безопасным границам, чтобы JSON из веба или
    /// localStorage не сломал стрим.
    pub fn normalized(mut self) -> Self {
        self.port = if self.port >= 1024 { self.port } else { DEFAULT_PORT };
        self.fps = self.fps.clamp(1, 60);
        self.quality = self.quality.clamp(1, 100);
        self.max_width = self.max_width.clamp(64, 3840);
        if self.mode != "relay" {
            self.mode = "local".into();
        }
        self.relay_url = self.relay_url.trim().to_string();
        self
    }
}

pub struct SettingsStore {
    path: Option<PathBuf>,
    inner: Mutex<Settings>,
}

impl SettingsStore {
    pub fn load() -> Arc<Self> {
        let path = dirs::config_dir().map(|dir| dir.join("FuckExam").join("settings.json"));
        let settings = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<Settings>(&raw).ok())
            .unwrap_or_default()
            .normalized();
        Arc::new(Self {
            path,
            inner: Mutex::new(settings),
        })
    }

    #[cfg(test)]
    pub fn ephemeral() -> Arc<Self> {
        Arc::new(Self {
            path: None,
            inner: Mutex::new(Settings::default()),
        })
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    pub fn set(&self, settings: Settings) -> Settings {
        let settings = settings.normalized();
        *self.inner.lock().unwrap() = settings.clone();
        self.save(&settings);
        settings
    }

    fn save(&self, settings: &Settings) {
        let Some(path) = self.path.as_deref() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(raw) = serde_json::to_string_pretty(settings) else {
            return;
        };
        if let Err(e) = std::fs::write(path, raw) {
            eprintln!("[fuckexam] settings: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_out_of_range_values() {
        let s = Settings {
            port: 80,
            fps: 0,
            quality: 255,
            max_width: 1,
            mode: "wat".into(),
            relay_url: "  ws://x  ".into(),
            ..Settings::default()
        }
        .normalized();
        assert_eq!(s.port, DEFAULT_PORT);
        assert_eq!(s.fps, 1);
        assert_eq!(s.quality, 100);
        assert_eq!(s.max_width, 64);
        assert_eq!(s.mode, "local");
        assert_eq!(s.relay_url, "ws://x");
    }

    #[test]
    fn store_roundtrip_keeps_values() {
        let store = SettingsStore::ephemeral();
        let saved = store.set(Settings {
            port: 7455,
            fps: 30,
            ..Settings::default()
        });
        assert_eq!(saved.port, 7455);
        assert_eq!(store.get().fps, 30);
    }
}
