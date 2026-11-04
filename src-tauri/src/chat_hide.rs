use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Курсор ближе этого расстояния к окну чата — прячем окно.
#[cfg(desktop)]
const HIDE_MARGIN: f64 = 10.0;
/// Курсор дальше этого расстояния — возвращаем окно. Небольшой разрыв между
/// порогами не даёт окну мигать на границе, но и не растягивает «зону тишины».
#[cfg(desktop)]
const SHOW_MARGIN: f64 = 32.0;

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if !enabled {
        if let Some(window) = app.get_webview_window("chat") {
            let _ = window.show();
        }
    }
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

        let Ok(cursor) = app.cursor_position() else {
            if debug && tick % 10 == 0 {
                crate::diag::log("hidey: cursor_position error");
            }
            continue;
        };

        if debug && tick % 10 == 0 {
            crate::diag::log(&format!(
                "hidey: cursor=({:.0},{:.0}) rect={rect:?} hidden={hidden} visible={:?}",
                cursor.x,
                cursor.y,
                window.is_visible().ok()
            ));
        }

        if hidden {
            match rect {
                Some((x, y, w, h)) if !inside(cursor, x, y, w, h, SHOW_MARGIN) => {
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
        if inside(cursor, x, y, w, h, HIDE_MARGIN) && window.is_visible().unwrap_or(true) {
            if debug {
                crate::diag::log(&format!(
                    "hidey: hide at cursor=({:.0},{:.0}) rect=({x:.0},{y:.0},{w:.0},{h:.0})",
                    cursor.x, cursor.y
                ));
            }
            let _ = window.hide();
            hidden = true;
        }
    }
}

#[cfg(desktop)]
fn inside(
    cursor: tauri::PhysicalPosition<f64>,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    margin: f64,
) -> bool {
    cursor.x >= x - margin
        && cursor.x <= x + w + margin
        && cursor.y >= y - margin
        && cursor.y <= y + h + margin
}
