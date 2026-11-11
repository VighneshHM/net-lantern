//! Desktop notifications, behind a trait so tests and headless runs can
//! leave them out.

/// Shows a desktop notification. Must not block: an implementation hands the
/// work off to a thread.
pub trait Notifier: Send + Sync {
    fn notify(&self, title: &str, body: &str);
}

/// Shows nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify(&self, _title: &str, _body: &str) {}
}
