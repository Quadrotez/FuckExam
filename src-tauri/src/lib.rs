#[cfg(target_os = "linux")]
#[path = "capture_linux.rs"]
pub mod capture;
#[cfg(target_os = "windows")]
#[path = "capture_windows.rs"]
pub mod capture;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
#[path = "capture_mobile.rs"]
pub mod capture;

pub mod chat_hide;
pub mod diag;
#[cfg(all(desktop, target_os = "linux"))]
pub mod hidey_kwin;
pub mod settings;

#[cfg(target_os = "linux")]
#[path = "window_capture_linux.rs"]
pub mod window_capture;

pub mod streaming;
pub mod tray;
pub mod ui;
pub mod web;

use serde::{Deserialize, Serialize};
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
    pub web: bool,
    pub desktop: bool,
}

#[tauri::command]
fn platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS.to_string(),
        recorder: cfg!(any(target_os = "linux", target_os = "windows")),
        relay: true,
        web: cfg!(desktop),
        desktop: cfg!(desktop),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebInfo {
    pub running: bool,
    pub port: u16,
    pub addresses: Vec<String>,
    pub close_to_tray: bool,
}

#[tauri::command]
fn web_info(web: State<'_, Arc<web::WebServer>>) -> WebInfo {
    let settings = web.store.get();
    let port = web.port().unwrap_or(settings.port);
    #[cfg(desktop)]
    let mut addresses = Vec::new();
    #[cfg(not(desktop))]
    let addresses: Vec<String> = Vec::new();
    #[cfg(desktop)]
    {
        let mut ips = streaming::local_addresses().unwrap_or_default();
        ips.sort();
        addresses.extend(ips.iter().map(|ip| format!("http://{ip}:{port}/")));
        addresses.push(format!("http://127.0.0.1:{port}/admin"));
    }
    WebInfo {
        running: web.port().is_some(),
        port,
        addresses,
        close_to_tray: settings.close_to_tray,
    }
}

#[tauri::command]
fn get_settings(store: State<'_, Arc<settings::SettingsStore>>) -> settings::Settings {
    store.get()
}

#[tauri::command]
fn save_settings(
    store: State<'_, Arc<settings::SettingsStore>>,
    web: State<'_, Arc<web::WebServer>>,
    settings: settings::Settings,
) -> settings::Settings {
    let previous = store.get();
    let saved = store.set(settings);
    if previous.port != saved.port && saved.web_on_launch && web.port().is_some() {
        web.restart_later(saved.port);
    }
    saved
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamOptions {
    fps: u32,
    quality: u8,
    max_width: usize,
}

#[tauri::command]
async fn stream_start(
    app: AppHandle,
    state: State<'_, streaming::Streaming>,
    web: State<'_, Arc<web::WebServer>>,
    options: StreamOptions,
) -> Result<streaming::StreamStatus, String> {
    let settings = web.store.get();
    let config = streaming::StreamConfig {
        fps: options.fps,
        quality: options.quality,
        max_width: options.max_width,
    };
    web.ensure(settings.port).await?;
    state
        .0
        .arm_local(app, settings.port, config)
        .await
}

#[tauri::command]
async fn relay_connect(
    app: AppHandle,
    state: State<'_, streaming::Streaming>,
    url: String,
    options: StreamOptions,
) -> Result<streaming::StreamStatus, String> {
    let config = streaming::StreamConfig {
        fps: options.fps,
        quality: options.quality,
        max_width: options.max_width,
    };
    state.0.arm_relay(app, url, config).await
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
fn window_show(app: AppHandle) {
    ui::show(&app);
}

#[tauri::command]
fn window_to_tray(app: AppHandle) {
    ui::hide_to_tray(&app);
}

#[tauri::command]
fn window_hide_all(app: AppHandle) {
    ui::hide_completely(&app);
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
    open_chat_window_impl(&app).await
}

#[cfg(desktop)]
pub(crate) async fn open_chat_window_impl(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("chat") {
        window.show().map_err(|e| format!("show chat window: {e}"))?;
        window
            .set_focus()
            .map_err(|e| format!("focus chat window: {e}"))?;
        return Ok(());
    }

    WebviewWindowBuilder::new(app, "chat", WebviewUrl::App("index.html".into()))
        .title("FuckExam — Чат")
        .inner_size(420.0, 620.0)
        .min_inner_size(200.0, 160.0)
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
    let store = settings::SettingsStore::load();
    let web = web::WebServer::new(hub.clone(), store.clone());

    let setup_web = web.clone();
    let setup_store = store.clone();
    #[cfg(desktop)]
    let close_store = store.clone();
    let builder = tauri::Builder::default()
        .manage(capture::Recorder {
            inner: std::sync::Mutex::new(None),
            hub: hub.clone(),
        })
        .manage(streaming::Streaming(hub))
        .manage(web)
        .manage(store)
        .setup(move |app| {
            #[cfg(desktop)]
            {
                // Окно создаётся из tauri.conf.json (так же, как на Android).
                // Иконку ставим здесь: на Linux без неё окно в панели задач безымянное.
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_icon(APP_ICON.clone());
                }
                if let Err(e) = tray::setup(app.handle()) {
                    diag::log(&format!("tray: {e}"));
                }
                chat_hide::spawn(app.handle().clone());
            }
            setup_web.set_app(app.handle().clone());

            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                let store = close_store.clone();
                if let Some(window) = app.get_webview_window("main") {
                    window.on_window_event(move |event| {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            if store.get().close_to_tray {
                                api.prevent_close();
                                ui::hide_to_tray(&handle);
                            }
                        }
                    });
                }
            }

            if setup_store.get().web_on_launch {
                let web = setup_web.clone();
                let port = setup_store.get().port;
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = web.ensure(port).await {
                        diag::log(&format!("web: {e}"));
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform_info,
            web_info,
            get_settings,
            save_settings,
            start_recording,
            stop_recording,
            recording_status,
            stream_start,
            relay_connect,
            stream_status,
            stream_stop,
            relay_ping,
            window_show,
            window_to_tray,
            window_hide_all,
            chat_send,
            chat_history,
            open_chat_window
        ]);

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(e) => {
            #[cfg(not(target_os = "android"))]
            let text = format!(
                "не удалось запустить приложение: {e}\nЛог: {}",
                diag::log_path().display()
            );
            #[cfg(target_os = "android")]
            let text = format!("не удалось запустить приложение: {e}");
            diag::report_fatal(&text);
            return;
        }
    };

    app.run(|_app_handle, _event| {
        #[cfg(desktop)]
        if let tauri::RunEvent::Exit = _event {
            chat_hide::shutdown();
        }
        #[cfg(not(desktop))]
        if let tauri::RunEvent::Resumed = _event {
            use tauri::Emitter as _;
            let _ = _app_handle.emit("app-resumed", ());
        }
    });
    diag::log("exit: clean");
}
