//! The refused-host log line, in a binary of its own: `tracing` caches per
//! callsite whether any subscriber wants it, and another test's subscriber in
//! the same process could have cached the server's callsites as "never".

mod common;

use std::sync::{Arc, Mutex};

use common::{browser, send, Server};

/// Log output captured from the `tracing` macros.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

// A current-thread runtime, so the server's tasks log on this thread, into the
// subscriber set here.
#[tokio::test(flavor = "current_thread")]
async fn a_foreign_host_is_logged_once() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _default = tracing::subscriber::set_default(subscriber);
    // Forget any interest cached before this subscriber existed.
    tracing::callsite::rebuild_interest_cache();

    let server = Server::start("home").await;
    let c = browser();
    for _ in 0..3 {
        let reply = send(
            c.get(server.url("/api/network"))
                .header("host", "attacker.example:47800"),
        )
        .await;
        assert_eq!(reply.status, 403, "body: {}", reply.body);
    }
    let logs = captured.text();
    assert_eq!(logs.matches("refused request").count(), 1, "logs: {logs}");
    assert!(logs.contains("attacker.example:47800"), "logs: {logs}");
}
