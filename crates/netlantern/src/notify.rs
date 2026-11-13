//! Desktop notifications from the executable.
//!
//! On Windows a toast goes out under the PowerShell app id: Windows shows
//! nothing for an app id that is not registered, and Net Lantern does not write
//! to the registry or change a Windows setting. So every title carries the app
//! name instead. Toasts are shown, in order, by one dedicated thread, so
//! `notify` never blocks the monitor.

use std::collections::HashSet;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use netlantern_core::Notifier;

/// Shows one toast; the error is a short description of why it failed.
type Show = Box<dyn Fn(&str, &str) -> Result<(), String> + Send>;

/// The notifier for this platform: Windows toasts on Windows, nothing elsewhere.
pub fn desktop_notifier() -> Arc<dyn Notifier> {
    #[cfg(windows)]
    {
        Arc::new(WindowsToast::with_show(Box::new(show_windows_toast)))
    }
    #[cfg(not(windows))]
    {
        Arc::new(netlantern_core::NoopNotifier)
    }
}

/// `Net Lantern: {title}`, so the toast says whose it is.
fn format_title(title: &str) -> String {
    format!("Net Lantern: {title}")
}

#[cfg(windows)]
fn show_windows_toast(title: &str, body: &str) -> Result<(), String> {
    use tauri_winrt_notification::Toast;
    Toast::new(Toast::POWERSHELL_APP_ID)
        .title(title)
        .text1(body)
        .show()
        .map_err(|e| e.to_string())
}

/// Hands each notification to a thread that shows them one at a time.
struct WindowsToast {
    tx: Option<Sender<(String, String)>>,
    /// Kept only so a test can wait for the queue to drain.
    #[cfg(test)]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WindowsToast {
    fn with_show(show: Show) -> Self {
        let (tx, rx) = channel::<(String, String)>();
        let _thread = std::thread::Builder::new()
            .name("toast".into())
            .spawn(move || {
                // Logged once per distinct error. Never the title or body:
                // they hold device names and addresses.
                let mut logged = HashSet::new();
                while let Ok((title, body)) = rx.recv() {
                    if let Err(e) = show(&title, &body) {
                        if logged.insert(e.clone()) {
                            tracing::warn!("cannot show a desktop notification: {e}");
                        }
                    }
                }
            })
            .expect("start the toast thread");
        Self {
            tx: Some(tx),
            #[cfg(test)]
            thread: Some(_thread),
        }
    }

    /// Stops accepting notifications and waits until every queued one was tried.
    #[cfg(test)]
    fn finish(mut self) {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            thread.join().expect("the toast thread did not panic");
        }
    }
}

impl Notifier for WindowsToast {
    fn notify(&self, title: &str, body: &str) {
        if let Some(tx) = &self.tx {
            // A closed channel means the thread is gone; nothing to show to.
            let _ = tx.send((format_title(title), body.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn titles_get_the_app_name_prefix() {
        assert_eq!(
            format_title("Office printer is offline"),
            "Net Lantern: Office printer is offline"
        );
    }

    #[test]
    fn the_toast_thread_survives_a_failing_show() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let toast = WindowsToast::with_show(Box::new(move |title, body| {
            log.lock()
                .unwrap()
                .push((title.to_string(), body.to_string()));
            Err("no desktop".to_string())
        }));
        toast.notify("One", "first");
        toast.notify("Two", "second");
        toast.finish();
        assert_eq!(
            *seen.lock().unwrap(),
            [
                ("Net Lantern: One".to_string(), "first".to_string()),
                ("Net Lantern: Two".to_string(), "second".to_string())
            ]
        );
    }

    #[test]
    fn the_desktop_notifier_can_be_built_and_dropped_unused() {
        // Nothing is shown until notify is called.
        let _ = desktop_notifier();
    }
}
