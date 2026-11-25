//! Start at login through the autostart plugin. The plugin is initialised
//! with `--hidden`, so a start at login stays in the tray.

use netlantern_server::Autostart;
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;

/// The arguments a start at login runs with.
pub const ARGS: &[&str] = &["--hidden"];

/// The host's start-at-login switch. Errors are the OS's free text: the
/// server keeps them out of its answers.
pub struct TauriAutostart {
    handle: AppHandle,
}

impl TauriAutostart {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle }
    }
}

impl Autostart for TauriAutostart {
    fn is_enabled(&self) -> Result<bool, String> {
        self.handle
            .autolaunch()
            .is_enabled()
            .map_err(|e| e.to_string())
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        let manager = self.handle.autolaunch();
        let result = if on {
            manager.enable()
        } else {
            manager.disable()
        };
        result.map_err(|e| e.to_string())
    }
}
