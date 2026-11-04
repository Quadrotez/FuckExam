use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Отступ по умолчанию, если настройки почему-то недоступны.
#[cfg(desktop)]
const DEFAULT_HIDE_MARGIN: f64 = 10.0;
/// Насколько дальше должна отойти мышь, чтобы окно вернулось: разрыв между
/// порогами не даёт окну мигать на границе.
#[cfg(desktop)]
const SHOW_GAP: f64 = 22.0;

/// Пороги «Хайди» в пикселях: (прятать, показывать).
#[cfg(desktop)]
fn margins(app: &AppHandle) -> (f64, f64) {
    let hide = app
        .try_state::<std::sync::Arc<crate::settings::SettingsStore>>()
        .map(|store| store.get().hidey_margin as f64)
        .unwrap_or(DEFAULT_HIDE_MARGIN);
    (hide, hide + SHOW_GAP)
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Снимает слежение при выходе: KWin-скрипт не должен переживать приложение.
#[cfg(desktop)]
pub fn shutdown() {
    #[cfg(target_os = "linux")]
    crate::hidey_kwin::stop_blocking();
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    #[cfg(all(desktop, target_os = "linux"))]
    if enabled {
        crate::hidey_kwin::start();
    } else {
        crate::hidey_kwin::stop();
    }
    if !enabled {
        if let Some(window) = app.get_webview_window("chat") {
            let _ = window.show();
        }
    }
}

/// Позиция курсора в пикселях окна.
///
/// На KDE Wayland сначала спрашиваем KWin: GDK через XWayland видит курсор
/// только пока он над нашим же окном.
#[cfg(desktop)]
fn cursor(app: &AppHandle) -> Option<(f64, f64)> {
    #[cfg(target_os = "linux")]
    if let Some(position) = crate::hidey_kwin::cursor() {
        return Some(position);
    }
    app.cursor_position()
        .ok()
        .map(|position| (position.x, position.y))
}

#[cfg(desktop)]
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move { watch(app).await });
}

#[cfg(desktop)]
async fn watch(app: AppHandle) {
    let mut hidden = false;
    let mut rect: Option<(f64, f64, f64, f64)> = None;
    let debug = std::env::var_os("FEX_DEBUG").is_some();
    let mut tick: u64 = 0;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        tick += 1;

        let Some(window) = app.get_webview_window("chat") else {
            hidden = false;
            rect = None;
            continue;
        };

        if !enabled() {
            if hidden {
                let _ = window.show();
                hidden = false;
            }
            continue;
        }

        let Some((cx, cy)) = cursor(&app) else {
            if debug && tick % 10 == 0 {
                crate::diag::log("hidey: курсор недоступен");
            }
            continue;
        };

        if debug && tick % 10 == 0 {
            crate::diag::log(&format!(
                "hidey: cursor=({cx:.0},{cy:.0}) rect={rect:?} hidden={hidden} visible={:?}",
                window.is_visible().ok()
            ));
        }

        let (hide_margin, show_margin) = margins(&app);
        if debug && tick % 10 == 0 {
            crate::diag::log(&format!("hidey: margins={hide_margin}/{show_margin}"));
        }

        if hidden {
            match rect {
                Some((x, y, w, h)) if !inside(cx, cy, x, y, w, h, show_margin) => {
                    if debug {
                        crate::diag::log("hidey: show");
                    }
                    let _ = window.show();
                    hidden = false;
                }
                None => {
                    if debug {
                        crate::diag::log("hidey: show (no rect)");
                    }
                    let _ = window.show();
                    hidden = false;
                }
                _ => {}
            }
            continue;
        }

        let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
            continue;
        };
        let (x, y, w, h) = (
            position.x as f64,
            position.y as f64,
            size.width as f64,
            size.height as f64,
        );
        rect = Some((x, y, w, h));
        if inside(cx, cy, x, y, w, h, hide_margin) && window.is_visible().unwrap_or(true) {
            if debug {
                crate::diag::log(&format!(
                    "hidey: hide at cursor=({cx:.0},{cy:.0}) rect=({x:.0},{y:.0},{w:.0},{h:.0})"
                ));
            }
            let _ = window.hide();
            hidden = true;
        }
    }
}

#[cfg(desktop)]
fn inside(cx: f64, cy: f64, x: f64, y: f64, w: f64, h: f64, margin: f64) -> bool {
    cx >= x - margin && cx <= x + w + margin && cy >= y - margin && cy <= y + h + margin
}
