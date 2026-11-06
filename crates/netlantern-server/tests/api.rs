//! The REST API and the WebSocket event stream, end to end: a real server on
//! `127.0.0.1:0` driven by the simulated networks in `scenarios/`, with
//! `reqwest` (cookie store on) as the browser and `tokio-tungstenite` as the
//! WebSocket client. Every wait has a timeout, so a bug fails instead of
//! hanging.

use std::future::Future;
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use netlantern_core::discover::DiscoverConfig;
use netlantern_core::{Db, EventKind, ScanConfig, ScanId, Store};
use netlantern_platform::{
    ArpEntry, Interface, MacAddr, NetIo, PingReply, Platform, PlatformError, SimNetwork, TcpProbe,
};
use netlantern_server::{router, AccessPolicy, AppState};
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
        let sim = Arc::new(sim(scenario));
        Self::with_platform(sim.clone(), sim).await
    }

    async fn with_platform(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>) -> Self {
        Self::with_store(platform, netio, Store::new(Db::open_in_memory().unwrap())).await
    }

    async fn with_store(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>, store: Store) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = AppState::new(
            platform,
            netio,
            store,
            // Short timeouts: these tests run on the real clock.
            ScanConfig {
                icmp_timeout: Duration::from_millis(50),
                tcp_timeout: Duration::from_millis(50),
                arp_poll_interval: Duration::from_millis(100),
                discover: DiscoverConfig {
                    window: Duration::from_millis(200),
                    ..DiscoverConfig::default()
                },
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
    /// cookie as a `Cookie` header value (`nl_session=<id>`).
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

fn sim(scenario: &str) -> SimNetwork {
    SimNetwork::load(format!(
        "{}/../../scenarios/{scenario}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A client that keeps cookies, as a browser does, and ignores any system
/// proxy so loopback requests stay on loopback.
fn browser() -> Client {
    Client::builder()
        .cookie_store(true)
        .no_proxy()
        .build()
        .unwrap()
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
            .get("x-netlantern-api-version")
            .map(|v| v.to_str().unwrap()),
        Some("1"),
        "every response carries x-netlantern-api-version: 1"
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
            assert_eq!(response.headers()["x-netlantern-api-version"], "1");
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
    let id = pair.strip_prefix("nl_session=").expect("cookie name");
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
    let server = Server::with_platform(Arc::new(Unavailable), Arc::new(sim("home"))).await;
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

#[tokio::test]
async fn a_failing_database_is_an_error_not_an_empty_answer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("netlantern.db");
    let home = Arc::new(sim("home"));
    let server = Server::with_store(home.clone(), home, Store::new(Db::open(&path).unwrap())).await;
    let (c, _) = server.login().await;
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch("DROP TABLE scans; DROP TABLE networks;")
        .unwrap();

    // Not "no devices" and not "no such scan": the store is broken.
    for path in [
        "/api/devices",
        "/api/devices/1",
        "/api/scans/1",
        "/api/scans/latest",
    ] {
        let body = assert_status_code(get(&server, &c, path).await, 500, "storage_error");
        // A fixed sentence, never the raw database error.
        assert_eq!(
            body["message"],
            "Net Lantern couldn't read or write its data."
        );
        assert_eq!(
            body["hint"],
            "Restart Net Lantern. If this keeps happening, copy the log into a bug report."
        );
    }
}

// ---------------------------------------------------------------- devices

async fn patch_device(server: &Server, client: &Client, id: i64, body: Value) -> Reply {
    send(
        client
            .patch(server.url(&format!("/api/devices/{id}")))
            .json(&body),
    )
    .await
}

async fn device_list(server: &Server, client: &Client) -> Vec<Value> {
    let reply = get(server, client, "/api/devices").await;
    assert_eq!(reply.status, 200, "body: {}", reply.body);
    reply.body.as_array().unwrap().clone()
}

#[tokio::test]
async fn devices_can_be_renamed_starred_and_marked_seen() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert!(device_list(&server, &c).await.is_empty());
    assert_status_code(get(&server, &c, "/api/devices/1").await, 404, "not_found");
    assert_status_code(
        patch_device(&server, &c, 1, json!({"starred": true})).await,
        404,
        "not_found",
    );
    assert_status_code(
        get(&server, &c, "/api/scans/latest").await,
        404,
        "not_found",
    );

    run_quick_scan(&server, &c).await;
    let list = device_list(&server, &c).await;
    assert_eq!(list.len(), HOME_DEVICES);
    let nas = list.iter().find(|d| d["ip"] == "192.168.1.70").unwrap();
    assert_eq!(nas["type"], "nas");
    assert_eq!(nas["vendor"], "Synology");
    let id = nas["id"].as_i64().unwrap();

    let one = get(&server, &c, &format!("/api/devices/{id}")).await;
    assert_eq!(one.status, 200);
    assert_eq!(one.body["ip"], "192.168.1.70");

    let r = patch_device(
        &server,
        &c,
        id,
        json!({"name": "  Backup box  ", "starred": true}),
    )
    .await;
    assert_eq!(r.status, 200, "body: {}", r.body);
    assert_eq!(r.body["display_name"], "Backup box");
    assert_eq!(r.body["starred"], true);
    assert_eq!(r.body["is_new"], false);

    // Absent leaves the name alone; the change is persisted.
    let r = patch_device(&server, &c, id, json!({"starred": false})).await;
    assert_eq!(r.body["display_name"], "Backup box");
    assert_eq!(r.body["starred"], false);
    let one = get(&server, &c, &format!("/api/devices/{id}")).await;
    assert_eq!(one.body["display_name"], "Backup box");

    // 64 characters (not bytes) after trimming is fine; 65 is not.
    let ok = patch_device(
        &server,
        &c,
        id,
        json!({"name": format!(" {} ", "\u{e9}".repeat(64))}),
    )
    .await;
    assert_eq!(ok.status, 200, "body: {}", ok.body);
    let too_long = patch_device(&server, &c, id, json!({"name": "x".repeat(65)})).await;
    let body = assert_status_code(too_long, 400, "bad_request");
    assert_eq!(body["message"], "Names can be at most 64 characters.");

    assert_status_code(
        patch_device(&server, &c, id, json!({"colour": "red"})).await,
        400,
        "bad_request",
    );
    assert_status_code(
        patch_device(&server, &c, id, json!({"is_new": true})).await,
        400,
        "bad_request",
    );
    assert_eq!(
        patch_device(&server, &c, id, json!({"is_new": false}))
            .await
            .status,
        200
    );
    let bad_json = send(
        c.patch(server.url(&format!("/api/devices/{id}")))
            .header("content-type", "application/json")
            .body("{nope"),
    )
    .await;
    assert_status_code(bad_json, 400, "bad_request");
    assert_status_code(
        patch_device(&server, &c, id, json!({"starred": "yes"})).await,
        400,
        "bad_request",
    );
    assert_status_code(
        patch_device(&server, &c, 99999, json!({"starred": true})).await,
        404,
        "not_found",
    );
    assert_status_code(get(&server, &c, "/api/devices/abc").await, 404, "not_found");
    assert_status_code(
        get(&server, &c, "/api/devices/99999").await,
        404,
        "not_found",
    );

    // null clears the name, falling back to the discovered one.
    let r = patch_device(&server, &c, id, json!({"name": null})).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body["display_name"], "DiskStation");
}

#[tokio::test]
async fn latest_scan_is_the_most_recent_snapshot_and_beats_the_id_route() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let id = run_quick_scan(&server, &c).await;
    let latest = get(&server, &c, "/api/scans/latest").await;
    assert_eq!(latest.status, 200, "body: {}", latest.body);
    assert_eq!(latest.body["state"], "done");
    assert_eq!(latest.body["id"], id);
    assert!(latest.body.as_object().unwrap().contains_key("error"));
    assert!(latest.body.as_object().unwrap().contains_key("isolated"));
}

/// A platform whose network can be swapped underneath it, as when the laptop
/// moves between networks.
struct Switchable(Mutex<Arc<SimNetwork>>);

impl Switchable {
    fn switch(&self, sim: SimNetwork) {
        *self.0.lock().unwrap() = Arc::new(sim);
    }
    fn sim(&self) -> Arc<SimNetwork> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait]
impl Platform for Switchable {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.sim().active_interface().await
    }
    async fn ping(&self, a: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.sim().ping(a, t).await
    }
    async fn arp_resolve(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.sim().arp_resolve(a, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.sim().arp_table().await
    }
    async fn tcp_probe(&self, a: Ipv4Addr, p: u16, t: Duration) -> Result<TcpProbe, PlatformError> {
        self.sim().tcp_probe(a, p, t).await
    }
}

/// The home scenario behind another router: same addressing, another MAC.
fn home_with_another_router() -> SimNetwork {
    let path = format!("{}/../../scenarios/home.json", env!("CARGO_MANIFEST_DIR"));
    let mut json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for d in json["devices"].as_array_mut().unwrap() {
        if d["ip"] == "192.168.1.1" {
            d["mac"] = "50:C7:BF:00:00:99".into();
        }
    }
    SimNetwork::from_json(&json.to_string()).unwrap()
}

#[tokio::test]
async fn another_network_never_shows_or_changes_the_last_ones_devices() {
    let platform = Arc::new(Switchable(Mutex::new(Arc::new(sim("home")))));
    let server = Server::with_platform(platform.clone(), Arc::new(sim("home"))).await;
    let (c, _) = server.login().await;
    let scan_a = run_quick_scan(&server, &c).await;
    let list = device_list(&server, &c).await;
    assert_eq!(list.len(), HOME_DEVICES);
    let id_a = list[0]["id"].as_i64().unwrap();

    // Network B: the same addressing behind a router with another MAC.
    platform.switch(home_with_another_router());
    assert!(device_list(&server, &c).await.is_empty());
    assert_status_code(
        get(&server, &c, &format!("/api/devices/{id_a}")).await,
        404,
        "not_found",
    );
    assert_status_code(
        patch_device(&server, &c, id_a, json!({"name": "Mine now"})).await,
        404,
        "not_found",
    );
    assert_status_code(
        get(&server, &c, "/api/scans/latest").await,
        404,
        "not_found",
    );

    // Once B is scanned it lists its own devices, never A's ids.
    let scan_b = run_quick_scan(&server, &c).await;
    let list_b = device_list(&server, &c).await;
    assert_eq!(list_b.len(), HOME_DEVICES);
    assert!(list_b.iter().all(|d| d["id"].as_i64() != Some(id_a)));
    assert_status_code(
        get(&server, &c, &format!("/api/devices/{id_a}")).await,
        404,
        "not_found",
    );
    assert_eq!(
        get(&server, &c, "/api/scans/latest").await.body["id"],
        scan_b
    );

    // Back on A: its device is untouched by the PATCH attempted on B.
    platform.switch(sim("home"));
    let one = get(&server, &c, &format!("/api/devices/{id_a}")).await;
    assert_eq!(one.status, 200, "body: {}", one.body);
    assert_ne!(one.body["display_name"], "Mine now");
    assert_eq!(
        get(&server, &c, "/api/scans/latest").await.body["id"],
        scan_a
    );
}

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
    // Devices stream during the sweep and again, enriched, at the merge.
    let devices: std::collections::HashSet<i64> = events
        .iter()
        .filter(|e| e["type"] == "scan.device")
        .map(|e| e["data"]["device"]["id"].as_i64().unwrap())
        .collect();
    assert_eq!(devices.len(), HOME_DEVICES);
    assert_eq!(
        events.last().unwrap(),
        &json!({
            "seq": seen + events.len() as u64,
            "api_version": 1,
            "type": "scan.done",
            "data": {
                "scan_id": id,
                "state": "done",
                "error": null,
                "isolated": false,
                "devices_found": 13
            },
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
        connect_ws(port, Some("nl_session=nope"), Some(&origin)).await,
        401,
    );
}

#[tokio::test]
async fn plain_get_on_the_event_stream_is_400_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(get(&server, &c, "/api/events").await, 400, "bad_request");
}
