use tauri::AppHandle;

#[cfg(desktop)]
use tauri::Manager;

/// Показывает главное окно и возвращает иконку в трей.
pub fn show(app: &AppHandle) {
    #[cfg(desktop)]
    {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        crate::tray::set_visible(app, true);
    }
    #[cfg(not(desktop))]
    let _ = app;
}

/// Окно спрятано, иконка в трее остаётся.
pub fn hide_to_tray(app: &AppHandle) {
    #[cfg(desktop)]
    {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
        crate::tray::set_visible(app, true);
    }
    #[cfg(not(desktop))]
    let _ = app;
}

/// Окно спрятано и иконка трея убрана: вернуть приложение можно только
/// через веб-панель на этом же компьютере.
pub fn hide_completely(app: &AppHandle) {
    #[cfg(desktop)]
    {
        crate::web::ensure_running(app);
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
        crate::tray::set_visible(app, false);
    }
    #[cfg(not(desktop))]
    let _ = app;
}
