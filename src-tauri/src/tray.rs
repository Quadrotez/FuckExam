use tauri::AppHandle;

#[cfg(desktop)]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub const TRAY_ID: &str = "main";

#[cfg(desktop)]
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Показать окно", true, None::<&str>)?;
    let to_tray = MenuItem::with_id(app, "hide_tray", "Скрыть в трей", true, None::<&str>)?;
    let hide_all = MenuItem::with_id(app, "hide_all", "Скрыть полностью", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &to_tray,
            &hide_all,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("FuckExam")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => crate::ui::show(app),
            "hide_tray" => crate::ui::hide_to_tray(app),
            "hide_all" => crate::ui::hide_completely(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::ui::show(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg(not(desktop))]
pub fn setup(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

pub fn set_visible(app: &AppHandle, visible: bool) {
    #[cfg(desktop)]
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(visible);
    }
    #[cfg(not(desktop))]
    let _ = (app, visible);
}
