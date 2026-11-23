//! The shared host: startup, the single-instance pipe and shutdown, in process.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use netlantern::host::{Backend, Host, HostError, HostOptions, Started};
use netlantern_core::{EndReason, Metric, MonitorState, NoopNotifier, Target};
use netlantern_server::HostKind;
use reqwest::Client;
use serde_json::{json, Value};
#[cfg(windows)]
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(5);

/// A pipe name no other test (or running Net Lantern) uses.
fn unique_pipe(test: &str) -> String {
    format!(
        r"\\.\pipe\netlantern-test-host-{}-{}",
        std::process::id(),
        test
    )
}

/// The built-in home demo, sped up, on the first free default port, with an
/// `on_open` that does nothing.
fn demo_options(pipe: &str) -> HostOptions {
    HostOptions {
        backend: Backend::Demo {
            scenario: None,
            time_scale: Some(60),
        },
        port: None,
        dev_origins: Vec::new(),
        notifier: Arc::new(NoopNotifier),
        autostart: None,
        host_kind: HostKind::Exe,
        on_open: Arc::new(|_url| {}),
        pipe_name: pipe.to_string(),
    }
}

async fn start_running(opts: HostOptions) -> Host {
    match Host::start(opts).await {
        Ok(Started::Running(host)) => host,
        Ok(Started::AlreadyRunning) => panic!("expected a running host, got AlreadyRunning"),
        Err(e) => panic!("the host did not start: {e}"),
    }
}

/// A host serving in the background until `stop` is sent.
struct Serving {
    port: u16,
    stop: oneshot::Sender<()>,
    done: JoinHandle<std::io::Result<()>>,
}

impl Serving {
    fn new(host: Host) -> Self {
        let port = host.port();
        let (stop, stopped) = oneshot::channel::<()>();
        let done = tokio::spawn(host.serve_until(async {
            let _ = stopped.await;
        }));
        Self { port, stop, done }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// Signals shutdown and waits for `serve_until` to return.
    async fn shut_down(self) {
        let _ = self.stop.send(());
        timeout(WAIT, self.done)
            .await
            .expect("serve_until did not return in time")
            .expect("the serve task panicked")
            .expect("serve_until failed");
    }
}

/// A client that keeps cookies, as a browser does, and ignores any system
/// proxy so loopback requests stay on loopback.
fn browser() -> Client {
    Client::builder()
        .no_proxy()
        .cookie_store(true)
        .build()
        .unwrap()
}

fn token_of(url: &str) -> String {
    url.split("?t=")
        .nth(1)
        .expect("the login URL carries a token")
        .to_string()
}

async fn sign_in(client: &Client, serving: &Serving, url: &str) -> reqwest::StatusCode {
    client
        .post(serving.url("/api/session"))
        .json(&json!({ "token": token_of(url) }))
        .send()
        .await
        .unwrap()
        .status()
}

/// Runs a quick scan and waits for monitoring to begin.
async fn scan_and_wait_for_monitoring(client: &Client, serving: &Serving) -> Value {
    let res = client
        .post(serving.url("/api/scans"))
        .json(&json!({ "mode": "quick" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 202);
    let scan_id = res.json::<Value>().await.unwrap()["scan_id"]
        .as_u64()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let scan: Value = client
            .get(serving.url(&format!("/api/scans/{scan_id}")))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if scan["state"] == "done" {
            break;
        }
        assert!(Instant::now() < deadline, "the scan did not finish: {scan}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let status: Value = client
            .get(serving.url("/api/monitor"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if status["state"] == "monitoring" {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "monitoring did not start: {status}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[tokio::test(flavor = "multi_thread")]
async fn start_serves_the_dashboard_and_login_url_signs_in_once() {
    let host = start_running(demo_options(&unique_pipe("dashboard"))).await;
    let url = host.login_url();
    assert!(
        url.starts_with(&format!("http://127.0.0.1:{}/?t=", host.port())),
        "unexpected login URL shape"
    );
    let serving = Serving::new(host);

    let client = browser();
    let res = client.get(serving.url("/")).send().await.unwrap();
    assert_eq!(res.status(), 200);

    let res = client
        .post(serving.url("/api/session"))
        .json(&json!({ "token": token_of(&url) }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
    assert!(
        res.headers().get("set-cookie").is_some(),
        "signing in sets the session cookie"
    );

    // The token is one-time.
    assert_eq!(sign_in(&browser(), &serving, &url).await, 401);

    serving.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn monitor_is_running_after_start() {
    let host = start_running(demo_options(&unique_pipe("monitor"))).await;
    let url = host.login_url();
    let serving = Serving::new(host);
    let client = browser();
    assert_eq!(sign_in(&client, &serving, &url).await, 204);

    let status = scan_and_wait_for_monitoring(&client, &serving).await;
    assert_ne!(status["state"], "not_connected", "{status}");

    serving.shut_down().await;
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn second_start_reports_already_running_and_calls_on_open() {
    let pipe = unique_pipe("second");
    let (tx, mut opened) = mpsc::unbounded_channel::<String>();
    let mut opts = demo_options(&pipe);
    opts.on_open = Arc::new(move |url| {
        let _ = tx.send(url);
    });
    let serving = Serving::new(start_running(opts).await);

    let second = timeout(WAIT, Host::start(demo_options(&pipe)))
        .await
        .expect("the second start did not return in time");
    assert!(
        matches!(second, Ok(Started::AlreadyRunning)),
        "the second start must find the first running"
    );

    let url = timeout(WAIT, opened.recv())
        .await
        .expect("on_open was not called")
        .unwrap();
    assert!(
        timeout(Duration::from_millis(300), opened.recv())
            .await
            .is_err(),
        "on_open must be called once"
    );
    assert_eq!(sign_in(&browser(), &serving, &url).await, 204);

    serving.shut_down().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_ends_the_session_as_quit_and_frees_the_port_and_pipe() {
    // The demo keeps its data in memory; holding the store handle keeps the
    // database alive after the host has stopped.
    let pipe = unique_pipe("shutdown");
    let host = start_running(demo_options(&pipe)).await;
    let store = host.state().store.clone();
    let monitor = Arc::clone(&host.state().monitor);
    let url = host.login_url();
    let serving = Serving::new(host);
    let port = serving.port;
    let client = browser();
    assert_eq!(sign_in(&client, &serving, &url).await, 204);
    scan_and_wait_for_monitoring(&client, &serving).await;
    let status = monitor.status();
    assert_eq!(status.state, MonitorState::Monitoring);
    let network = status.network_id.expect("monitoring a network");
    let started_at = status.session_started_at.expect("a session is open");

    serving.shut_down().await;

    // The port is free again.
    drop(std::net::TcpListener::bind(("127.0.0.1", port)).expect("the port is still taken"));

    // The pipe is free again: a new host on it runs instead of deferring.
    let again = timeout(WAIT, Host::start(demo_options(&pipe)))
        .await
        .expect("the new start did not return in time");
    match again {
        Ok(Started::Running(host)) => Serving::new(host).shut_down().await,
        Ok(Started::AlreadyRunning) => panic!("the pipe is still held"),
        Err(e) => panic!("the new host did not start: {e}"),
    }

    // The monitoring session ended as a quit.
    let series = store
        .metrics(
            network,
            Target::Gateway,
            Metric::RttMs,
            started_at,
            now_ms(),
        )
        .await
        .unwrap();
    let last = series.sessions.last().expect("the session was recorded");
    assert!(last.ended_at.is_some(), "the session is still open");
    assert_eq!(last.end_reason, Some(EndReason::Quit));
}

#[tokio::test(flavor = "multi_thread")]
async fn port_in_use_error_keeps_the_existing_wording() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let mut opts = demo_options(&unique_pipe("port"));
    opts.port = Some(port);

    let err = match Host::start(opts).await {
        Err(e) => e,
        Ok(_) => panic!("the port is taken; start must fail"),
    };
    assert!(matches!(err, HostError::PortInUse(p) if p == port), "{err}");
    assert_eq!(
        err.to_string(),
        format!(
            "Port {port} is in use. Close the program using it or choose another with --port <number>."
        )
    );
}
