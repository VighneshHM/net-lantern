//! The REST API and the WebSocket event stream, end to end: a real server on
//! `127.0.0.1:0` driven by the simulated networks in `scenarios/`, with
//! `reqwest` (cookie store on) as the browser and `tokio-tungstenite` as the
//! WebSocket client. Every wait has a timeout, so a bug fails instead of
//! hanging.

use std::future::Future;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use netbot_core::{EventKind, ScanConfig, ScanId};
use netbot_platform::{
    ArpEntry, Interface, MacAddr, PingReply, Platform, PlatformError, SimNetwork, TcpProbe,
};
use netbot_server::{router, AccessPolicy, AppState};
use reqwest::header::HeaderMap;
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// The longest any single wait may take before the test fails.
const WAIT: Duration = Duration::from_secs(10);

/// Devices in `scenarios/home.json`.
const HOME_DEVICES: usize = 13;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------- helpers

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {WAIT:?} waiting for {what}"))
}

/// A server running in the background on its own port, plus a handle on its
/// state (for launch tokens and the event bus).
struct Server {
    port: u16,
    state: AppState,
}

impl Server {
    /// A server on one of the simulated networks in `scenarios/`.
    async fn start(scenario: &str) -> Self {
        let sim = SimNetwork::load(format!(
            "{}/../../scenarios/{scenario}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Self::with_platform(Arc::new(sim)).await
    }

    async fn with_platform(platform: Arc<dyn Platform>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = AppState::new(
            platform,
            ScanConfig {
                icmp_timeout: Duration::from_millis(50),
                ..Default::default()
            },
            AccessPolicy::for_port(port, &[]),
        );
        let app = router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { port, state }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// A browser that has redeemed a fresh launch token, and its session
    /// cookie as a `Cookie` header value (`nb_session=<id>`).
    async fn login(&self) -> (Client, String) {
        let client = browser();
        let token = self.state.auth.issue_launch_token();
        let res = post_session(self, &client, &token).await;
        assert_eq!(res.status, 204);
        let cookie = res.headers["set-cookie"].to_str().unwrap();
        let pair = cookie.split(';').next().unwrap().to_string();
        (client, pair)
    }
}

/// A client that keeps cookies, as a browser does.
fn browser() -> Client {
    Client::builder().cookie_store(true).build().unwrap()
}

/// A platform where every OS call fails for a reason other than being offline.
struct Unavailable;

const UNAVAILABLE: &str = "the network service is stopped";

fn unavailable() -> PlatformError {
    PlatformError::Unavailable {
        reason: UNAVAILABLE.to_string(),
    }
}

#[async_trait]
impl Platform for Unavailable {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        Err(unavailable())
    }

    async fn ping(&self, _: Ipv4Addr, _: Duration) -> Result<Option<PingReply>, PlatformError> {
        Err(unavailable())
    }

    async fn arp_resolve(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        Err(unavailable())
    }

    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        Err(unavailable())
    }

    async fn tcp_probe(&self, _: Ipv4Addr, _: u16, _: Duration) -> Result<TcpProbe, PlatformError> {
        Err(unavailable())
    }
}

/// A response with its body parsed. Building one asserts the API-version
/// header and that a non-empty body is JSON (never axum's plain-text
/// rejections).
struct Reply {
    status: u16,
    headers: HeaderMap,
    body: Value,
}

async fn send(request: RequestBuilder) -> Reply {
    let res = within("an HTTP response", request.send()).await.unwrap();
    assert_eq!(
        res.headers()
            .get("x-netbot-api-version")
            .map(|v| v.to_str().unwrap()),
        Some("1"),
        "every response carries x-netbot-api-version: 1"
    );
    let status = res.status().as_u16();
    let headers = res.headers().clone();
    let text = within("an HTTP body", res.text()).await.unwrap();
    let body = if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("body is not JSON: {text:?}"))
    };
    Reply {
        status,
        headers,
        body,
    }
}

/// Asserts a structured error: status, code, a non-empty message, and a
/// `hint` key. Returns the body for further checks.
fn assert_status_code(reply: Reply, status: u16, code: &str) -> Value {
    assert_eq!(reply.status, status, "body: {}", reply.body);
    assert_eq!(reply.body["code"], code, "body: {}", reply.body);
    assert!(
        reply.body["message"]
            .as_str()
            .is_some_and(|m| !m.is_empty()),
        "message must be a non-empty string: {}",
        reply.body
    );
    assert!(
        reply.body.as_object().unwrap().contains_key("hint"),
        "hint is always present: {}",
        reply.body
    );
    reply.body
}

async fn post_session(server: &Server, client: &Client, token: &str) -> Reply {
    send(
        client
            .post(server.url("/api/session"))
            .json(&json!({ "token": token })),
    )
    .await
}

async fn get(server: &Server, client: &Client, path: &str) -> Reply {
    send(client.get(server.url(path))).await
}

async fn post_scan(server: &Server, client: &Client, mode: &str) -> Reply {
    send(
        client
            .post(server.url("/api/scans"))
            .json(&json!({ "mode": mode })),
    )
    .await
}

async fn snapshot(server: &Server, client: &Client, id: u64) -> Value {
    let reply = get(server, client, &format!("/api/scans/{id}")).await;
    assert_eq!(reply.status, 200, "body: {}", reply.body);
    reply.body
}

/// Starts a quick scan and returns its id.
async fn start_quick_scan(server: &Server, client: &Client) -> u64 {
    let reply = post_scan(server, client, "quick").await;
    assert_eq!(reply.status, 202, "body: {}", reply.body);
    assert_eq!(reply.body.as_object().unwrap().len(), 1, "{}", reply.body);
    reply.body["scan_id"].as_u64().expect("scan_id is a number")
}

/// Runs a quick scan and returns once its `scan.done` has been published, so
/// the bus is quiet afterwards.
async fn run_quick_scan(server: &Server, client: &Client) -> u64 {
    let mut rx = server.state.bus.subscribe();
    let id = start_quick_scan(server, client).await;
    within("scan.done on the bus", async {
        loop {
            let event = rx.recv().await.unwrap();
            if let EventKind::ScanDone { scan_id, .. } = event.kind {
                if scan_id == ScanId(id) {
                    return;
                }
            }
        }
    })
    .await;
    id
}

fn ws_request(port: u16, cookie: Option<&str>, origin: Option<&str>) -> WsRequest {
    let mut request = format!("ws://127.0.0.1:{port}/api/events")
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    if let Some(cookie) = cookie {
        headers.insert("cookie", cookie.parse().unwrap());
    }
    if let Some(origin) = origin {
        headers.insert("origin", origin.parse().unwrap());
    }
    request
}

/// The error is boxed: `tungstenite::Error` is large.
async fn connect(request: WsRequest) -> Result<Ws, Box<WsError>> {
    within(
        "the WebSocket handshake",
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map(|(ws, _response)| ws)
    .map_err(Box::new)
}

async fn connect_ws(
    port: u16,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Result<Ws, Box<WsError>> {
    connect(ws_request(port, cookie, origin)).await
}

/// Asserts the upgrade was refused with `status` (and still carried the
/// version header).
fn assert_http_status(result: Result<Ws, Box<WsError>>, status: u16) {
    match result.map_err(|err| *err) {
        Err(WsError::Http(response)) => {
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()["x-netbot-api-version"], "1");
        }
        Err(other) => panic!("expected an HTTP {status} refusal, got {other}"),
        Ok(_) => panic!("expected an HTTP {status} refusal, but the upgrade was accepted"),
    }
}

/// The next text message, parsed as JSON. Control frames are skipped.
async fn next_json(ws: &mut Ws) -> Value {
    within("a WebSocket message", async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    return serde_json::from_str(&text)
                        .unwrap_or_else(|_| panic!("message is not JSON: {text}"))
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(other)) => panic!("unexpected message: {other:?}"),
                Some(Err(err)) => panic!("WebSocket error: {err}"),
                None => panic!("WebSocket closed"),
            }
        }
    })
    .await
}

fn is_lower_hex_64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ---------------------------------------------------------------- session

#[tokio::test]
async fn launch_token_is_exchanged_once_for_a_strict_http_only_cookie() {
    let server = Server::start("home").await;
    let token = server.state.auth.issue_launch_token();

    let c = browser();
    let res = post_session(&server, &c, &token).await;
    assert_eq!(res.status, 204);
    assert!(res.body.is_null(), "204 has no body");
    let set_cookie = res.headers["set-cookie"].to_str().unwrap();
    let (pair, attributes) = set_cookie.split_once(';').unwrap();
    let id = pair.strip_prefix("nb_session=").expect("cookie name");
    assert!(is_lower_hex_64(id), "session id: {id:?}");
    assert_ne!(id, token, "the session id is not the launch token");
    assert_eq!(attributes, " HttpOnly; SameSite=Strict; Path=/");

    // The cookie is the session: the same browser can now use the API.
    let devices = get(&server, &c, "/api/devices").await;
    assert_eq!(devices.status, 200);
    assert_eq!(devices.body, json!([]));

    // The token was used up; another browser can neither reuse it nor get in.
    let c2 = browser();
    assert_status_code(
        post_session(&server, &c2, &token).await,
        401,
        "unauthorized",
    );
    assert_status_code(get(&server, &c2, "/api/devices").await, 401, "unauthorized");
    assert_status_code(
        post_session(&server, &c2, "nope").await,
        401,
        "unauthorized",
    );
}

#[tokio::test]
async fn malformed_session_requests_are_400_json() {
    let server = Server::start("home").await;
    let c = browser();
    let url = server.url("/api/session");
    for request in [
        c.post(&url)
            .header("content-type", "application/json")
            .body("{not json"),
        c.post(&url).json(&json!({ "tok": "x" })),
        c.post(&url).json(&json!({ "token": 7 })),
        c.post(&url),
    ] {
        assert_status_code(send(request).await, 400, "bad_request");
    }
}

// ---------------------------------------------------------------- network

#[tokio::test]
async fn network_reports_the_active_interface() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let res = get(&server, &c, "/api/network").await;
    assert_eq!(res.status, 200);
    assert_eq!(
        res.body,
        json!({
            "ip": "192.168.1.23",
            "prefix": 24,
            "gateway": "192.168.1.1",
            "dns_servers": ["192.168.1.1"],
            "kind": "wifi",
            "mac": "F4:8E:38:9A:20:7C",
        })
    );
}

#[tokio::test]
async fn platform_failures_other_than_offline_are_500_platform_error() {
    let server = Server::with_platform(Arc::new(Unavailable)).await;
    let (c, _) = server.login().await;
    let message = format!("unavailable: {UNAVAILABLE}");

    let body = assert_status_code(
        get(&server, &c, "/api/network").await,
        500,
        "platform_error",
    );
    assert_eq!(body["message"], message.as_str());
    assert!(body["hint"].is_null());

    let body = assert_status_code(post_scan(&server, &c, "quick").await, 500, "platform_error");
    assert_eq!(body["message"], message.as_str());
}

// ---------------------------------------------------------------- methods

#[tokio::test]
async fn a_wrong_method_is_405_json_but_only_after_the_guard() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;

    for (request, allowed) in [
        (c.delete(server.url("/api/devices")), "GET"),
        (c.put(server.url("/api/scans")), "POST"),
        (c.post(server.url("/api/events")), "GET"),
        (c.get(server.url("/api/session")), "POST"),
    ] {
        let reply = send(request).await;
        let allow = reply.headers["allow"].to_str().unwrap().to_string();
        let body = assert_status_code(reply, 405, "method_not_allowed");
        assert!(body["hint"].is_null(), "{body}");
        assert!(allow.contains(allowed), "Allow: {allow}");
    }

    // Without a session the guard answers first, so nothing about the routes
    // leaks.
    assert_status_code(
        send(browser().delete(server.url("/api/devices"))).await,
        401,
        "unauthorized",
    );
}

// ------------------------------------------------------------------ scans

#[tokio::test]
async fn quick_scan_runs_to_done_and_a_second_start_while_running_is_409() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;

    let id = start_quick_scan(&server, &c).await;
    let again = assert_status_code(
        post_scan(&server, &c, "quick").await,
        409,
        "scan_in_progress",
    );
    assert_eq!(again["scan_id"], id);

    let done = within("the scan snapshot to reach done", async {
        loop {
            let snap = snapshot(&server, &c, id).await;
            if snap["state"] == "done" {
                return snap;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert_eq!(done["id"], id);
    assert_eq!(done["mode"], "quick");
    assert_eq!(done["devices_found"], 13);
    assert_eq!(done["total"], 254);
    assert_eq!(done["done"], 254);

    let devices = get(&server, &c, "/api/devices").await;
    assert_eq!(devices.status, 200);
    assert_eq!(devices.body.as_array().unwrap().len(), HOME_DEVICES);
}

#[tokio::test]
async fn deep_scan_is_501_unsupported() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(post_scan(&server, &c, "deep").await, 501, "unsupported");
}

#[tokio::test]
async fn malformed_scan_requests_are_400_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let url = server.url("/api/scans");
    for request in [
        c.post(&url)
            .header("content-type", "application/json")
            .body("not json"),
        c.post(&url).json(&json!({ "mode": "turbo" })),
        c.post(&url).json(&json!({ "mode": 1 })),
        c.post(&url).json(&json!({})),
        c.post(&url),
    ] {
        assert_status_code(send(request).await, 400, "bad_request");
    }
    // Nothing was started.
    assert_status_code(get(&server, &c, "/api/scans/1").await, 404, "not_found");
}

#[tokio::test]
async fn unknown_scans_and_api_paths_are_404_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    for path in [
        "/api/scans/999",
        "/api/scans/abc",
        "/api/scans/-1",
        "/api/nope",
        "/api/scans/1/extra",
    ] {
        let body = assert_status_code(get(&server, &c, path).await, 404, "not_found");
        assert!(body["hint"].is_null(), "{path}: {body}");
    }
    // Without a session the guard answers first.
    assert_status_code(
        get(&server, &browser(), "/api/nope").await,
        401,
        "unauthorized",
    );
}

// ---------------------------------------------------------------- offline

#[tokio::test]
async fn offline_network_and_scan_are_503_not_connected() {
    let offline = Server::start("offline").await;
    let (offline_client, _) = offline.login().await;

    let body = assert_status_code(
        get(&offline, &offline_client, "/api/network").await,
        503,
        "not_connected",
    );
    assert!(body["hint"].as_str().is_some_and(|h| !h.is_empty()));
    assert_status_code(
        post_scan(&offline, &offline_client, "quick").await,
        503,
        "not_connected",
    );
    // An unsupported mode is reported as such, connected or not.
    assert_status_code(
        post_scan(&offline, &offline_client, "deep").await,
        501,
        "unsupported",
    );
}

// -------------------------------------------------------------- websocket

#[tokio::test]
async fn websocket_says_hello_then_streams_every_event_in_seq_order() {
    let server = Server::start("home").await;
    let (c, cookie) = server.login().await;
    // A finished scan first, so the hello has a non-zero seq to report.
    run_quick_scan(&server, &c).await;
    let seen = server.state.bus.last_seq();
    assert!(seen > 0);

    let mut ws = connect_ws(server.port, Some(&cookie), Some(&server.origin()))
        .await
        .unwrap();
    let hello = next_json(&mut ws).await;
    assert_eq!(
        hello,
        json!({ "type": "hello", "seq": seen, "api_version": 1 })
    );

    let id = start_quick_scan(&server, &c).await;
    let mut events = Vec::new();
    loop {
        let event = next_json(&mut ws).await;
        let last = event["type"] == "scan.done";
        events.push(event);
        if last {
            break;
        }
    }

    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    let expected: Vec<u64> = (seen + 1..=seen + events.len() as u64).collect();
    assert_eq!(
        seqs, expected,
        "seqs follow hello.seq with no gap or repeat"
    );
    for event in &events {
        assert_eq!(event["api_version"], 1, "{event}");
        assert_eq!(event["data"]["scan_id"], id, "{event}");
    }
    let devices = events.iter().filter(|e| e["type"] == "scan.device").count();
    assert_eq!(devices, HOME_DEVICES);
    assert_eq!(
        events.last().unwrap(),
        &json!({
            "seq": seen + events.len() as u64,
            "api_version": 1,
            "type": "scan.done",
            "data": { "scan_id": id, "state": "done", "devices_found": 13 },
        })
    );
}

#[tokio::test]
async fn websocket_answers_pings() {
    let server = Server::start("home").await;
    let (_, cookie) = server.login().await;
    let mut ws = connect_ws(server.port, Some(&cookie), Some(&server.origin()))
        .await
        .unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");

    ws.send(Message::Ping(vec![1, 2, 3].into())).await.unwrap();
    let reply = within("a pong", ws.next()).await.unwrap().unwrap();
    assert_eq!(reply, Message::Pong(vec![1, 2, 3].into()));
}

#[tokio::test]
async fn websocket_upgrade_is_refused_cross_origin_cross_host_or_without_session() {
    let server = Server::start("home").await;
    let (_, cookie) = server.login().await;
    let port = server.port;
    let origin = server.origin();

    // Cross-site WebSocket hijacking: a page elsewhere, with the cookie.
    assert_http_status(
        connect_ws(port, Some(&cookie), Some("http://evil.example")).await,
        403,
    );
    // DNS rebinding: a foreign Host, even with the cookie and our Origin.
    let mut rebound = ws_request(port, Some(&cookie), Some(&origin));
    rebound
        .headers_mut()
        .insert("host", format!("evil.example:{port}").parse().unwrap());
    assert_http_status(connect(rebound).await, 403);
    // No session, or a made-up one.
    assert_http_status(connect_ws(port, None, Some(&origin)).await, 401);
    assert_http_status(
        connect_ws(port, Some("nb_session=nope"), Some(&origin)).await,
        401,
    );
}

#[tokio::test]
async fn plain_get_on_the_event_stream_is_400_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(get(&server, &c, "/api/events").await, 400, "bad_request");
}
