//! The tray icon: a left click opens the window; the menu opens it or quits.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::App;

const OPEN: &str = "open";
const QUIT: &str = "quit";

/// Adds the tray icon. Tauri keeps it alive for the life of the app.
pub fn build(app: &App) -> tauri::Result<TrayIcon> {
    let open = MenuItem::with_id(app, OPEN, "Open Net Lantern", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Net Lantern")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN => crate::open_window(app.clone(), None),
            QUIT => crate::quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::open_window(tray.app_handle().clone(), None);
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)
}
