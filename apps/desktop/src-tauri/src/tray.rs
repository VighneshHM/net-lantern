//! The tray icon: a left click opens the window; the menu opens it or quits.

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::App;

/// The tray icon's id, for removing it on a forced exit.
pub const ID: &str = "main";
const OPEN: &str = "open";
const QUIT: &str = "quit";

/// Adds the tray icon. Tauri keeps it alive for the life of the app.
pub fn build(app: &App) -> tauri::Result<TrayIcon> {
    let open = MenuItem::with_id(app, OPEN, "Open Net Lantern", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    // Its own 32 px image: on Windows the window icon would come from the .ico at
    // SM_CXICON (40-48 px at 125-150% scaling), not the small tray design.
    let icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
    TrayIconBuilder::with_id(ID)
        .icon(icon)
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
        })
        .build(app)
}
