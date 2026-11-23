//! Desktop notifications through the notification plugin.
//!
//! Every monitor title carries the app name. A failure is logged once per
//! distinct error, never with the title or body: they hold device names and
//! addresses.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

use netlantern_core::Notifier;
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

/// `Net Lantern: {title}`, so the notification says whose it is.
pub fn format_title(title: &str) -> String {
    format!("Net Lantern: {title}")
}

/// Remembers which errors were logged already.
#[derive(Default)]
pub struct OnceLog(Mutex<HashSet<String>>);

impl OnceLog {
    /// Whether `error` is new (and so should be logged now).
    pub fn first(&self, error: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(error.to_string())
    }
}

/// The monitor's notifier. The plugin shows notifications on its own async
/// runtime, so `notify` does not block the monitor.
pub struct TauriNotifier {
    handle: AppHandle,
    errors: OnceLog,
}

impl TauriNotifier {
    pub fn new(handle: AppHandle) -> Self {
        Self {
            handle,
            errors: OnceLog::default(),
        }
    }

    /// Shows `title` and `body` as they are.
    pub fn show(&self, title: &str, body: &str) {
        let shown = self
            .handle
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show();
        if let Err(e) = shown {
            let e = e.to_string();
            if self.errors.first(&e) {
                tracing::warn!("cannot show a desktop notification: {e}");
            }
        }
    }
}

impl Notifier for TauriNotifier {
    fn notify(&self, title: &str, body: &str) {
        self.show(&format_title(title), body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifier_formats_title() {
        assert_eq!(
            format_title("Office printer is offline"),
            "Net Lantern: Office printer is offline"
        );
    }

    #[test]
    fn each_distinct_error_is_logged_once() {
        let log = OnceLog::default();
        assert!(log.first("no desktop"));
        assert!(!log.first("no desktop"));
        assert!(log.first("other"));
        assert!(!log.first("other"));
    }
}
