//! Port binding, the `run` entry point, and the embedded UI's SPA fallback,
//! end to end against a real server on `127.0.0.1:0`.
//!
//! Nothing here depends on `ui/dist` existing: without it the fallback serves
//! a placeholder page, with it the built `index.html`, and both are
//! `text/html`.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use netbot_core::ScanConfig;
use netbot_platform::SimNetwork;
use netbot_server::{bind, run, AccessPolicy, AppState, ServeError, DEFAULT_PORTS};
use reqwest::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE};
use reqwest::{Client, Response};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

/// The longest any single wait may take before the test fails.
const WAIT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- helpers

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {WAIT:?} waiting for {what}"))
}

/// A server on the `home` scenario, started through the public `run`, and
/// its state (for launch tokens).
struct Server {
    port: u16,
    state: AppState,
}

fn app_state(port: u16) -> AppState {
    let sim = SimNetwork::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scenarios/home.json"
    ))
    .unwrap();
    AppState::new(
        Arc::new(sim),
        ScanConfig::default(),
        AccessPolicy::for_port(port, &[]),
    )
}

impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = app_state(port);
        tokio::spawn(run(listener, state.clone(), std::future::pending()));
        Self { port, state }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// A browser that has redeemed a fresh launch token.
    async fn login(&self) -> Client {
        let client = browser();
        let token = self.state.auth.issue_launch_token();
        let res = within(
            "the session",
            client
                .post(self.url("/api/session"))
                .json(&json!({ "token": token }))
                .send(),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), 204);
        client
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

async fn get(client: &Client, url: String) -> Response {
    within("an HTTP response", client.get(url).send())
        .await
        .unwrap()
}

fn header(res: &Response, name: reqwest::header::HeaderName) -> String {
    res.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}

/// Asserts the response is a structured JSON error and returns its body.
async fn json_error(res: Response, status: u16, code: &str) -> Value {
    assert_eq!(res.status(), status);
    let content_type = header(&res, CONTENT_TYPE);
    assert!(
        content_type.starts_with("application/json"),
        "expected JSON, got {content_type:?}"
    );
    let body: Value = within("a JSON body", res.json()).await.unwrap();
    assert_eq!(body["code"], code, "{body}");
    body
}

// ------------------------------------------------------------------- bind

#[tokio::test]
async fn bind_skips_a_port_that_is_taken() {
    // Windows hands out ephemeral ports in sequence, so the port after the
    // held one is often a concurrent test's client socket. Pick another pair
    // when that happens; a `bind` that wrongly gives up would fail every try.
    for _ in 0..20 {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = held.local_addr().unwrap().port();
        let Some(next) = p.checked_add(1) else {
            continue;
        };

        let Ok((listener, got)) = bind(p..=next).await else {
            continue;
        };

        assert_eq!(got, next);
        assert_eq!(listener.local_addr().unwrap().port(), next);
        assert!(
            listener.local_addr().unwrap().ip().is_loopback(),
            "must bind loopback only"
        );
        return;
    }
    panic!("never found a free port next to a held one");
}

#[tokio::test]
async fn bind_fails_when_every_port_is_taken() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let p = held.local_addr().unwrap().port();

    let err = bind(p..=p).await.unwrap_err();

    assert!(matches!(err, ServeError::NoFreePort { first, last } if first == p && last == p));
    assert!(err.to_string().contains("are all in use"));
    assert_eq!(
        err.to_string(),
        format!(
            "Ports {p}\u{2013}{p} are all in use. \
             Close the program using them or start NetBot with --port <number>."
        )
    );
}

#[test]
fn default_ports_are_47800_to_47809() {
    assert_eq!(DEFAULT_PORTS, 47800..=47809);
}

// -------------------------------------------------------------------- run

#[tokio::test]
async fn run_serves_until_the_shutdown_future_completes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = oneshot::channel::<()>();
    let server = tokio::spawn(run(listener, app_state(port), async {
        stopped.await.ok();
    }));

    let res = get(&browser(), format!("http://127.0.0.1:{port}/")).await;
    assert_eq!(res.status(), 200);

    stop.send(()).unwrap();
    let result = within("run to return after shutdown", server)
        .await
        .unwrap();
    assert!(result.is_ok(), "{result:?}");
}

// ------------------------------------------------------------ UI fallback

#[tokio::test]
async fn a_client_route_gets_the_html_shell() {
    let server = Server::start().await;

    // No session needed for static content.
    let res = get(&browser(), server.url("/devices")).await;

    assert_eq!(res.status(), 200);
    let content_type = header(&res, CONTENT_TYPE);
    assert!(content_type.starts_with("text/html"), "{content_type:?}");
    let body = within("the body", res.text()).await.unwrap();
    assert!(body.to_lowercase().contains("<html"), "{body}");
}

#[tokio::test]
async fn the_root_is_html_that_browsers_revalidate() {
    let server = Server::start().await;

    let res = get(&browser(), server.url("/")).await;

    assert_eq!(res.status(), 200);
    assert!(header(&res, CONTENT_TYPE).starts_with("text/html"));
    assert_eq!(header(&res, CACHE_CONTROL), "no-cache");
}

#[tokio::test]
async fn head_gets_the_headers_without_a_body() {
    let server = Server::start().await;

    let res = within("an HTTP response", browser().head(server.url("/")).send())
        .await
        .unwrap();

    assert_eq!(res.status(), 200);
    assert!(header(&res, CONTENT_TYPE).starts_with("text/html"));
    assert!(within("the body", res.text()).await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_api_paths_stay_json_not_html() {
    let server = Server::start().await;
    let c = server.login().await;

    for path in ["/api", "/api/nope", "/api/scans/1/extra"] {
        let body = json_error(get(&c, server.url(path)).await, 404, "not_found").await;
        assert!(body["hint"].is_null(), "{path}: {body}");
    }

    // Without a session the guard answers first, still as JSON.
    json_error(
        get(&browser(), server.url("/api/nope")).await,
        401,
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn writing_to_a_ui_path_is_405_json() {
    let server = Server::start().await;

    let res = within(
        "an HTTP response",
        browser().post(server.url("/devices")).send(),
    )
    .await
    .unwrap();

    assert_eq!(header(&res, ALLOW), "GET, HEAD");
    let body = json_error(res, 405, "method_not_allowed").await;
    assert!(body["hint"].is_null(), "{body}");
}

#[tokio::test]
async fn the_guard_covers_ui_paths_too() {
    let server = Server::start().await;
    let c = browser();

    let foreign_host = within(
        "an HTTP response",
        c.get(server.url("/devices"))
            .header("Host", "evil.example")
            .send(),
    )
    .await
    .unwrap();
    json_error(foreign_host, 403, "forbidden_host").await;

    let foreign_origin = within(
        "an HTTP response",
        c.get(server.url("/"))
            .header("Origin", "http://evil.example")
            .send(),
    )
    .await
    .unwrap();
    json_error(foreign_origin, 403, "forbidden_origin").await;
}

/// One raw `GET`, returned whole. `reqwest` would tidy `..` away before
/// sending, so odd request targets go over a bare socket.
async fn raw_get(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request =
        format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    within("the response", stream.read_to_string(&mut response))
        .await
        .unwrap();
    response
}

#[tokio::test]
async fn odd_request_targets_get_the_html_shell_not_a_file_or_an_error() {
    let server = Server::start().await;

    // Absolute, drive-letter, UNC-looking and parent-directory targets are
    // not names under `ui/dist`, so they are answered like any unknown path.
    for target in [
        "//etc/os-release",
        "///host/share/x",
        "/C:/Windows/win.ini",
        "/../x",
        "/a/../../x",
        "/./x",
    ] {
        let response = raw_get(server.port, target).await.to_lowercase();
        assert!(response.starts_with("http/1.1 200"), "{target}: {response}");
        assert!(
            response.contains("content-type: text/html"),
            "{target}: {response}"
        );
    }
}
