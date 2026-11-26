//! The REST API and the WebSocket event stream, end to end: a real server on
//! `127.0.0.1:0` driven by the simulated networks in `scenarios/`, with
//! `reqwest` (cookie store on) as the browser and `tokio-tungstenite` as the
//! WebSocket client. Every wait has a timeout, so a bug fails instead of
//! hanging.

mod common;

use common::{browser, post_session, send, sim, within, Reply, Server};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use netlantern_core::{Db, EventKind, ScanId, Store};
use netlantern_platform::{
    ArpEntry, Interface, MacAddr, NearbyNetwork, PingReply, Platform, PlatformError, SimNetwork,
    TcpProbe, WifiConnection,
};
use netlantern_server::{Autostart, HostInfo, HostKind};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::client::Request as WsRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Devices in `scenarios/home.json`.
const HOME_DEVICES: usize = 13;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------- helpers

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
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi adapter".into(),
        })
    }

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
    async fn network_category(
        &self,
    ) -> Result<netlantern_platform::NetworkCategory, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "not simulated".into(),
        })
    }
    async fn firewall_status(
        &self,
        _exe: &std::path::Path,
        _port: u16,
    ) -> Result<netlantern_platform::FirewallStatus, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "not simulated".into(),
        })
    }

    async fn os_description(&self) -> String {
        "unknown".to_string()
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
            "network_id": null,
            "scan_consent": false,
            "category": "private",
            "ssid": "Lantern Home",
            "replaced_router": null,
        })
    );
}

async fn put_consent(server: &Server, client: &Client, granted: bool) -> Reply {
    send(
        client
            .put(server.url("/api/network/consent"))
            .json(&json!({ "granted": granted })),
    )
    .await
}

async fn wait_for_state(server: &Server, client: &Client, id: u64, state: &str) -> Value {
    within(&format!("the scan to reach {state}"), async {
        loop {
            let snap = snapshot(server, client, id).await;
            if snap["state"] == state {
                return snap;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
}

#[tokio::test]
async fn network_reports_consent_and_id() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let before = get(&server, &c, "/api/network").await.body;
    assert!(before["network_id"].is_null(), "{before}");
    assert_eq!(before["scan_consent"], false);
    assert_eq!(before["ip"], "192.168.1.23", "interface fields stay flat");

    run_quick_scan(&server, &c).await;
    let after = get(&server, &c, "/api/network").await.body;
    assert!(after["network_id"].is_number(), "{after}");
    assert_eq!(after["scan_consent"], false);

    let put = put_consent(&server, &c, true).await;
    assert_eq!(put.status, 200, "body: {}", put.body);
    assert_eq!(put.body["network_id"], after["network_id"]);
    assert_eq!(put.body["scan_consent"], true);
    assert_eq!(put.body.as_object().unwrap().len(), 2);
    let granted = get(&server, &c, "/api/network").await.body;
    assert_eq!(granted["scan_consent"], true);

    let revoked = put_consent(&server, &c, false).await;
    assert_eq!(revoked.body["scan_consent"], false);
}

#[tokio::test]
async fn malformed_consent_requests_are_400_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let url = server.url("/api/network/consent");
    for request in [
        c.put(&url)
            .header("content-type", "application/json")
            .body("not json"),
        c.put(&url).json(&json!({ "granted": "yes" })),
        c.put(&url).json(&json!({})),
        c.put(&url),
    ] {
        assert_status_code(send(request).await, 400, "bad_request");
    }
}

#[tokio::test]
async fn consent_with_a_dead_router_is_503_network_unidentified() {
    let server = Server::start("router-down").await;
    let (c, _) = server.login().await;
    let body = assert_status_code(
        put_consent(&server, &c, true).await,
        503,
        "network_unidentified",
    );
    assert_eq!(
        body["hint"],
        "Check that you're connected to your router, then try again."
    );
}

#[tokio::test]
async fn consent_offline_is_503_not_connected() {
    let server = Server::start("offline").await;
    let (c, _) = server.login().await;
    assert_status_code(put_consent(&server, &c, true).await, 503, "not_connected");
}

#[tokio::test]
async fn consent_needs_a_session_and_our_origin() {
    let server = Server::start("home").await;
    // No session cookie.
    assert_status_code(
        put_consent(&server, &browser(), true).await,
        401,
        "unauthorized",
    );
    // A page elsewhere, with the cookie.
    let (c, _) = server.login().await;
    assert_status_code(
        send(
            c.put(server.url("/api/network/consent"))
                .header("origin", "http://evil.example")
                .json(&json!({ "granted": true })),
        )
        .await,
        403,
        "forbidden_origin",
    );
    // Neither request changed anything.
    assert_eq!(
        get(&server, &c, "/api/network").await.body["scan_consent"],
        false
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
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

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
    async fn network_category(
        &self,
    ) -> Result<netlantern_platform::NetworkCategory, PlatformError> {
        self.sim().network_category().await
    }
    async fn firewall_status(
        &self,
        exe: &std::path::Path,
        port: u16,
    ) -> Result<netlantern_platform::FirewallStatus, PlatformError> {
        self.sim().firewall_status(exe, port).await
    }

    async fn os_description(&self) -> String {
        self.sim().os_description().await
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
async fn deep_scan_without_consent_is_403_consent_needed() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let body = assert_status_code(post_scan(&server, &c, "deep").await, 403, "consent_needed");
    assert_eq!(
        body["message"],
        "Deep scan needs your permission for this network."
    );
    assert_eq!(
        body["hint"],
        "Confirm that you own this network or have permission to scan it."
    );
}

#[tokio::test]
async fn consent_then_deep_scan_runs_to_done() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_eq!(put_consent(&server, &c, true).await.status, 200);
    let started = post_scan(&server, &c, "deep").await;
    assert_eq!(started.status, 202, "body: {}", started.body);
    let id = started.body["scan_id"].as_u64().unwrap();
    let done = wait_for_state(&server, &c, id, "done").await;
    assert_eq!(done["mode"], "deep");

    let devices = device_list(&server, &c).await;
    let router = devices
        .iter()
        .find(|d| d["ip"] == "192.168.1.1")
        .expect("the router is listed");
    let ports = router["ports"].as_array().unwrap();
    assert!(
        ports
            .iter()
            .any(|p| p["port"] == 80 && p["service"] == "http" && p["banner"] == "Archer AX55"),
        "ports: {ports:?}"
    );
    assert!(router["os_guess"].is_object(), "{router}");
    assert!(router["deep_scanned_at"].is_number(), "{router}");
}

#[tokio::test]
async fn cancel_routes() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_eq!(put_consent(&server, &c, true).await.status, 200);
    let id = post_scan(&server, &c, "deep").await.body["scan_id"]
        .as_u64()
        .unwrap();
    let cancel_path = format!("/api/scans/{id}/cancel");

    let cancelled = post_empty(&server, &c, &cancel_path).await;
    assert_eq!(cancelled.status, 202, "body: {}", cancelled.body);
    assert_eq!(cancelled.body["id"], id);
    assert_eq!(cancelled.body["mode"], "deep");
    wait_for_state(&server, &c, id, "cancelled").await;

    assert_status_code(
        post_empty(&server, &c, &cancel_path).await,
        409,
        "scan_not_running",
    );
    assert_status_code(
        post_empty(&server, &c, "/api/scans/9999/cancel").await,
        404,
        "not_found",
    );
    assert_status_code(
        post_empty(&server, &browser(), &cancel_path).await,
        401,
        "unauthorized",
    );
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
    // A deep scan offline is not connected too, before any consent question.
    assert_status_code(
        post_scan(&offline, &offline_client, "deep").await,
        503,
        "not_connected",
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

/// The next close frame, within `limit`. Pings and pongs are skipped; any
/// other message, an error, or the stream ending without one fails.
async fn expect_close(ws: &mut Ws, limit: Duration) {
    let closed = timeout(limit, async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_))) => return,
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                Some(Ok(other)) => panic!("unexpected message: {other:?}"),
                Some(Err(err)) => panic!("WebSocket error before a close frame: {err}"),
                None => panic!("WebSocket ended without a close frame"),
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "no close frame within {limit:?}");
}

#[tokio::test]
async fn revoking_phone_sessions_closes_their_websockets() {
    let server = Server::start("home").await;
    let origin = server.origin();

    // A phone signs in with the reusable phone token.
    let token = server.state.auth.rotate_phone_token();
    let phone = browser();
    let res = post_session(&server, &phone, &token).await;
    assert_eq!(res.status, 204);
    let set_cookie = res.headers["set-cookie"].to_str().unwrap();
    let (phone_cookie, attributes) = set_cookie.split_once(';').unwrap();
    assert_eq!(attributes, " HttpOnly; SameSite=Strict; Path=/");
    assert_eq!(get(&server, &phone, "/api/network").await.status, 200);

    let (local, local_cookie) = server.login().await;
    let mut phone_ws = connect_ws(server.port, Some(phone_cookie), Some(&origin))
        .await
        .unwrap();
    let mut local_ws = connect_ws(server.port, Some(&local_cookie), Some(&origin))
        .await
        .unwrap();
    assert_eq!(next_json(&mut phone_ws).await["type"], "hello");
    assert_eq!(next_json(&mut local_ws).await["type"], "hello");

    server.state.auth.end_phone_sessions();

    expect_close(&mut phone_ws, Duration::from_secs(1)).await;
    assert_status_code(
        get(&server, &phone, "/api/network").await,
        401,
        "unauthorized",
    );

    // The local socket is still served: it answers a ping.
    local_ws
        .send(Message::Ping(vec![4, 5, 6].into()))
        .await
        .unwrap();
    let reply = within("a pong", local_ws.next()).await.unwrap().unwrap();
    assert_eq!(reply, Message::Pong(vec![4, 5, 6].into()));
    assert_eq!(get(&server, &local, "/api/network").await.status, 200);
}

#[tokio::test]
async fn plain_get_on_the_event_stream_is_400_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(get(&server, &c, "/api/events").await, 400, "bad_request");
}

// ------------------------------------------------- diagnosis and Wi-Fi

async fn post_empty(server: &Server, client: &Client, path: &str) -> Reply {
    send(client.post(server.url(path))).await
}

/// Polls until the diagnosis reaches `done`.
async fn diagnosis_when_done(server: &Server, client: &Client, id: u64) -> Value {
    within("the diagnosis to finish", async {
        loop {
            let run = get(server, client, &format!("/api/diagnose/{id}")).await;
            assert_eq!(run.status, 200, "body: {}", run.body);
            if run.body["state"] == "done" {
                return run.body;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
}

#[tokio::test]
async fn diagnose_starts_and_can_be_fetched() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;

    let started = post_empty(&server, &c, "/api/diagnose").await;
    assert_eq!(started.status, 202, "body: {}", started.body);
    assert_eq!(
        started.body,
        json!({ "run_id": 1, "already_running": false })
    );

    let run = get(&server, &c, "/api/diagnose/1").await;
    assert_eq!(run.status, 200, "body: {}", run.body);
    assert!(run.body["steps"].is_array(), "body: {}", run.body);

    let done = diagnosis_when_done(&server, &c, 1).await;
    assert_eq!(done["id"], 1);
    let latest = get(&server, &c, "/api/diagnose/latest").await;
    assert_eq!(latest.status, 200);
    assert_eq!(latest.body["id"], 1);
}

#[tokio::test]
async fn second_diagnose_returns_the_running_id() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;

    let first = post_empty(&server, &c, "/api/diagnose").await;
    let second = post_empty(&server, &c, "/api/diagnose").await;
    assert_eq!(first.status, 202);
    assert_eq!(second.status, 202, "never a 409: {}", second.body);
    assert_eq!(second.body["run_id"], first.body["run_id"]);
    assert_eq!(second.body["already_running"], true);
    server
        .state
        .diagnose
        .wait(netlantern_core::DiagnosisId(1))
        .await;
}

#[tokio::test]
async fn latest_diagnose_is_404_before_any_run() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(
        get(&server, &c, "/api/diagnose/latest").await,
        404,
        "not_found",
    );
}

#[tokio::test]
async fn unknown_or_bad_diagnose_id_is_404_json() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(get(&server, &c, "/api/diagnose/99").await, 404, "not_found");
    assert_status_code(
        get(&server, &c, "/api/diagnose/abc").await,
        404,
        "not_found",
    );
}

#[tokio::test]
async fn wifi_status_on_the_home_scenario() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let reply = get(&server, &c, "/api/wifi").await;
    assert_eq!(reply.status, 200, "body: {}", reply.body);
    assert_eq!(reply.body["interface_kind"], "wifi");
    assert_eq!(reply.body["connection"]["ssid"], "Lantern Home");
    assert_eq!(reply.body["connection"]["band"], "5ghz");
}

#[tokio::test]
async fn wifi_with_location_denied_is_403_location_permission() {
    let server = Server::start("location-denied").await;
    let (c, _) = server.login().await;
    let body = assert_status_code(
        get(&server, &c, "/api/wifi").await,
        403,
        "location_permission",
    );
    assert_eq!(
        body["message"],
        "Windows is hiding Wi-Fi details from Net Lantern."
    );
    assert_eq!(
        body["hint"],
        r#"Turn on Location services and "Let desktop apps access your location" in Settings > Privacy & security > Location."#
    );
}

#[tokio::test]
async fn nearby_scan_returns_networks_and_congestion() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let reply = post_empty(&server, &c, "/api/wifi/scan").await;
    assert_eq!(reply.status, 200, "body: {}", reply.body);
    let networks = reply.body["networks"].as_array().unwrap();
    assert!(!networks.is_empty());
    let signals: Vec<i64> = networks
        .iter()
        .map(|n| n["rssi_dbm"].as_i64().unwrap())
        .collect();
    assert!(
        signals.windows(2).all(|w| w[0] >= w[1]),
        "strongest first: {signals:?}"
    );
    assert!(reply.body["congestion"]["advice"]
        .as_str()
        .is_some_and(|a| !a.is_empty()));
}

#[tokio::test]
async fn nearby_scan_without_an_adapter_is_503_wifi_unavailable() {
    let server = Server::with_platform(Arc::new(Unavailable), Arc::new(sim("home"))).await;
    let (c, _) = server.login().await;
    let body = assert_status_code(
        post_empty(&server, &c, "/api/wifi/scan").await,
        503,
        "wifi_unavailable",
    );
    assert_eq!(
        body["message"],
        "Wi-Fi details aren't available: no Wi-Fi adapter."
    );
    assert_eq!(body["hint"], "Check that Wi-Fi is turned on.");
}

#[tokio::test]
async fn diagnose_routes_need_a_session() {
    let server = Server::start("home").await;
    let c = browser();
    assert_status_code(
        post_empty(&server, &c, "/api/diagnose").await,
        401,
        "unauthorized",
    );
    assert_status_code(get(&server, &c, "/api/wifi").await, 401, "unauthorized");
}

#[tokio::test]
async fn every_new_route_needs_a_session() {
    let server = Server::start("home").await;
    let c = browser();
    for path in [
        "/api/monitor",
        "/api/metrics?target=gateway&metric=rtt_ms&range=1h",
        "/api/alerts",
        "/api/incidents/1",
        "/api/rules",
    ] {
        assert_status_code(get(&server, &c, path).await, 401, "unauthorized");
    }
    for (method, path) in [
        (reqwest::Method::POST, "/api/alerts/1/ack"),
        (reqwest::Method::PUT, "/api/rules"),
    ] {
        assert_status_code(
            send(c.request(method, server.url(path)).json(&json!({}))).await,
            401,
            "unauthorized",
        );
    }
}

// ------------------------------------------------- category, ssid, new router

const OLD_ROUTER: &str = "50:C7:BF:00:00:01";

/// A server on the home scenario whose store already knows the home network
/// (its SSID and subnet) behind another router, so the router looks replaced.
async fn home_with_replaced_router() -> (Server, i64) {
    let store = Store::new(Db::open_in_memory().unwrap());
    let old = store
        .network_for(OLD_ROUTER.parse().unwrap(), "192.168.1.0/24", 10)
        .await
        .unwrap();
    store.set_ssid_label(old, "Lantern Home").await.unwrap();
    let sim = Arc::new(sim("home"));
    let server = Server::with_store(sim.clone(), sim, store).await;
    (server, old.0)
}

async fn post_decision(server: &Server, client: &Client, keep: bool) -> Reply {
    send(
        client
            .post(server.url("/api/network/replaced-router"))
            .json(&json!({ "keep": keep })),
    )
    .await
}

#[tokio::test]
async fn network_reports_category_ssid_and_no_pending_router() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let body = get(&server, &c, "/api/network").await.body;
    assert_eq!(body["category"], "private", "{body}");
    assert_eq!(body["ssid"], "Lantern Home");
    assert!(body["replaced_router"].is_null());
}

#[tokio::test]
async fn network_reports_public_on_the_cafe_network() {
    let server = Server::start("cafe").await;
    let (c, _) = server.login().await;
    let body = get(&server, &c, "/api/network").await.body;
    assert_eq!(body["category"], "public", "{body}");
    assert_eq!(body["ssid"], "Cafe Guest");
}

#[tokio::test]
async fn pending_replaced_router_blocks_scans_with_409() {
    let (server, old) = home_with_replaced_router().await;
    let (c, _) = server.login().await;
    let body = get(&server, &c, "/api/network").await.body;
    assert_eq!(
        body["replaced_router"],
        json!({ "network_id": old, "label": "Lantern Home" })
    );

    let hint = "Answer the question on the Dashboard: keep this network's devices and names, or start a new network.";
    let scan = assert_status_code(
        post_scan(&server, &c, "quick").await,
        409,
        "network_decision_needed",
    );
    assert_eq!(
        scan["message"],
        "This looks like a network you know with a new router."
    );
    assert_eq!(scan["hint"], hint);
    let consent = assert_status_code(
        put_consent(&server, &c, true).await,
        409,
        "network_decision_needed",
    );
    assert_eq!(consent["hint"], hint);
}

#[tokio::test]
async fn answering_keep_clears_the_prompt_and_scans_start() {
    let (server, old) = home_with_replaced_router().await;
    let (c, _) = server.login().await;
    let res = post_decision(&server, &c, true).await;
    assert_eq!(res.status, 200, "body: {}", res.body);
    assert_eq!(res.body, json!({ "network_id": old }));

    let body = get(&server, &c, "/api/network").await.body;
    assert!(body["replaced_router"].is_null(), "{body}");
    let id = run_quick_scan(&server, &c).await;
    assert_eq!(snapshot(&server, &c, id).await["state"], "done");
}

#[tokio::test]
async fn answering_when_nothing_is_pending_is_404_no_decision_pending() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let body = assert_status_code(
        post_decision(&server, &c, true).await,
        404,
        "no_decision_pending",
    );
    assert_eq!(
        body["message"],
        "There is no new-router question to answer."
    );
    assert!(body["hint"].is_string());
}

// -------------------------------------------------------------- debug info

#[tokio::test]
async fn debug_info_requires_a_session_and_is_redacted() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let server = Server::with_logs("home", Arc::clone(&logs)).await;
    let anonymous = browser();
    assert_status_code(
        get(&server, &anonymous, "/api/debug-info").await,
        401,
        "unauthorized",
    );

    let (c, _cookie) = server.login().await;
    run_quick_scan(&server, &c).await;
    let devices = get(&server, &c, "/api/devices").await.body;
    let devices = devices.as_array().unwrap().clone();
    assert!(devices.len() > 3, "the home demo has devices");
    let network = get(&server, &c, "/api/network").await.body;

    // Every private value the scenario holds, as it would appear in a log.
    let mut private: Vec<String> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for device in &devices {
        for key in ["ip", "mac"] {
            private.extend(device[key].as_str().map(str::to_string));
        }
        for key in ["name", "hostname", "display_name"] {
            names.extend(device[key].as_str().map(str::to_string));
        }
        for service in device["services"].as_array().unwrap() {
            let (source, value) = (service["source"].as_str(), service["value"].as_str());
            if let (Some(source), Some(value)) = (source, value) {
                if ["mdns", "netbios", "ssdp", "dns"].contains(&source)
                    && !is_protocol_string(value)
                {
                    names.push(value.to_string());
                }
            }
        }
    }
    private.extend(network["ip"].as_str().map(str::to_string));
    private.extend(network["gateway"].as_str().map(str::to_string));
    names.extend(network["ssid"].as_str().map(str::to_string));
    names.retain(|n| n.chars().count() >= 2);
    private.retain(|p| p != "1.1.1.1" && p != "8.8.8.8");
    assert!(
        private.len() > 6 && names.len() > 6,
        "{private:?} {names:?}"
    );

    let token = "9f8e7d6c5b4a39281706f5e4d3c2b1a0";
    {
        let mut lines = logs.lock().unwrap();
        for value in private.iter().chain(&names) {
            lines.push(format!("INFO saw {value} on the network"));
        }
        lines.push(format!("INFO open http://127.0.0.1:8080/?t={token}"));
        lines.push("INFO pinged 1.1.1.1 and 8.8.8.8".to_string());
    }

    let res = within(
        "the debug info",
        c.get(server.url("/api/debug-info")).send(),
    )
    .await
    .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    assert_eq!(
        res.headers()["content-type"].to_str().unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(res.headers()["cache-control"].to_str().unwrap(), "no-store");
    let text = res.text().await.unwrap();
    let lower = text.to_lowercase();

    assert!(text.starts_with("Net Lantern "), "{text}");
    assert!(text.contains("OS: Simulated network"), "{text}");
    assert!(text.contains("Interface: wifi"), "{text}");
    assert!(text.contains("Monitoring: "), "{text}");
    for value in private.iter().chain(&names) {
        assert!(
            !lower.contains(&value.to_lowercase()),
            "{value} leaked: {text}"
        );
    }
    assert!(!text.contains(token) && !text.contains("?t="), "{text}");
    for shown in ["x.x.x.x", "xx:xx:xx:xx:xx:xx", "[name]", "[redacted]"] {
        assert!(text.contains(shown), "{shown} missing: {text}");
    }
    assert!(text.contains("pinged 1.1.1.1 and 8.8.8.8"), "{text}");
}

/// Service types and SSDP search targets: not names, and not private.
fn is_protocol_string(value: &str) -> bool {
    value.starts_with('_')
        || ["upnp:", "urn:", "uuid:", "ssdp:"]
            .iter()
            .any(|p| value.starts_with(p))
}

async fn put_update(server: &Server, client: &Client, body: Value) -> Reply {
    send(client.put(server.url("/api/update")).json(&body)).await
}

#[tokio::test]
async fn update_routes_need_a_session_and_round_trip_the_setting() {
    let server = Server::start("home").await;
    // No session: nothing is readable or settable.
    assert_status_code(
        get(&server, &browser(), "/api/update").await,
        401,
        "unauthorized",
    );
    assert_status_code(
        put_update(&server, &browser(), json!({ "enabled": true })).await,
        401,
        "unauthorized",
    );

    let (c, _) = server.login().await;
    let first = get(&server, &c, "/api/update").await;
    assert_eq!(first.status, 200);
    assert_eq!(first.body["setting"], Value::Null, "not answered yet");
    assert_eq!(first.body["current"], env!("CARGO_PKG_VERSION"));
    assert_eq!(first.body["newer"], false);
    assert_eq!(first.body["url"], Value::Null);

    let mut events = server.state.bus.subscribe();
    let on = put_update(&server, &c, json!({ "enabled": true })).await;
    assert_eq!(on.status, 200);
    assert_eq!(on.body["setting"], true);
    let event = within("update.changed", events.recv()).await.unwrap();
    assert_eq!(event.kind, EventKind::UpdateChanged {});
    assert_eq!(get(&server, &c, "/api/update").await.body["setting"], true);

    let off = put_update(&server, &c, json!({ "enabled": false })).await;
    assert_eq!(off.body["setting"], false);
    assert_eq!(get(&server, &c, "/api/update").await.body["setting"], false);

    assert_status_code(
        put_update(&server, &c, json!({ "enabled": "yes" })).await,
        400,
        "bad_request",
    );
}

// ---------------------------------------------------------- delete all data

const CONFIRM: &str = "delete all data";

async fn post_reset(server: &Server, client: &Client, body: Value) -> Reply {
    send(client.post(server.url("/api/data/reset")).json(&body)).await
}

/// Every event on `rx` up to and including the next `data.reset`.
async fn events_until_data_reset(
    rx: &mut tokio::sync::broadcast::Receiver<netlantern_core::Event>,
) -> Vec<EventKind> {
    within("data.reset on the bus", async {
        let mut seen = Vec::new();
        loop {
            let event = rx.recv().await.unwrap();
            let last = event.kind == EventKind::DataReset {};
            seen.push(event.kind);
            if last {
                return seen;
            }
        }
    })
    .await
}

async fn rows_in(db: &Db, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    db.call(move |c| c.query_row(&sql, [], |r| r.get(0)))
        .await
        .unwrap()
}

#[tokio::test]
async fn reset_needs_a_session_and_the_exact_confirm_text() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let scan = run_quick_scan(&server, &c).await;
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    let mut events = server.state.bus.subscribe();

    // No session, or another site's page: refused before anything runs.
    assert_status_code(
        post_reset(&server, &browser(), json!({ "confirm": CONFIRM })).await,
        401,
        "unauthorized",
    );
    assert_status_code(
        send(
            c.post(server.url("/api/data/reset"))
                .header("origin", "http://evil.example")
                .json(&json!({ "confirm": CONFIRM })),
        )
        .await,
        403,
        "forbidden_origin",
    );

    let url = server.url("/api/data/reset");
    for request in [
        c.post(&url),
        c.post(&url)
            .header("content-type", "application/json")
            .body("not json"),
        c.post(&url).json(&json!({})),
        c.post(&url).json(&json!({ "confirm": "" })),
        c.post(&url).json(&json!({ "confirm": "yes" })),
        c.post(&url).json(&json!({ "confirm": "Delete all data" })),
        c.post(&url).json(&json!({ "confirm": " delete all data" })),
        c.post(&url).json(&json!({ "confirm": "delete all data." })),
        c.post(&url).json(&json!({ "confirm": true })),
        c.post(&url).json(&json!({ "confirm": CONFIRM, "also": 1 })),
        c.post(&url).json(&json!([CONFIRM])),
    ] {
        let body = assert_status_code(send(request).await, 400, "bad_request");
        assert_eq!(
            body["message"],
            r#"Expected a JSON body {"confirm": "delete all data"}."#
        );
    }
    assert_status_code(
        get(&server, &c, "/api/data/reset").await,
        405,
        "method_not_allowed",
    );

    // Nothing at all happened.
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    assert_eq!(snapshot(&server, &c, scan).await["state"], "done");
    assert!(get(&server, &c, "/api/network").await.body["network_id"].is_number());
    sleep(Duration::from_millis(100)).await;
    assert!(
        events.try_recv().is_err(),
        "no event: nothing was stopped or deleted"
    );

    let done = post_reset(&server, &c, json!({ "confirm": CONFIRM })).await;
    assert_eq!(done.status, 204, "body: {}", done.body);
    assert_eq!(done.body, Value::Null);
    assert!(device_list(&server, &c).await.is_empty());
}

#[tokio::test]
async fn reset_during_a_running_scan_and_monitor_leaves_a_clean_empty_store() {
    let db = Db::open_in_memory().unwrap();
    let store = Store::new(db.clone());
    let server = Server::with_phone("home", store.clone()).await;
    server.state.monitor.start();
    server.state.updates.start();
    let (c, _) = server.login().await;
    let update = send(
        c.put(server.url("/api/update"))
            .json(&json!({ "enabled": false })),
    )
    .await;
    assert_eq!(update.body["setting"], false);

    // 1. A deep scan with consent, and phone access on.
    assert_eq!(put_consent(&server, &c, true).await.status, 200);
    let mut events = server.state.bus.subscribe();
    let started = post_scan(&server, &c, "deep").await;
    assert_eq!(started.status, 202, "body: {}", started.body);
    let scan = started.body["scan_id"].as_u64().unwrap();
    let phone = send(
        c.post(server.url("/api/phone-access"))
            .json(&json!({ "enabled": true })),
    )
    .await;
    assert_eq!(phone.status, 200, "body: {}", phone.body);
    assert_eq!(phone.body["state"], "on");
    let phone_url = phone.body["url"].as_str().unwrap().to_string();

    // 2. Delete everything while the scan runs.
    let reset = post_reset(&server, &c, json!({ "confirm": CONFIRM })).await;

    // 3. Empty, waiting for a scan, phone access off, and clients told.
    assert_eq!(reset.status, 204, "body: {}", reset.body);
    let seen = events_until_data_reset(&mut events).await;
    assert!(
        seen.iter().any(|k| matches!(
            k,
            EventKind::ScanDone { scan_id, state: netlantern_core::ScanState::Cancelled, .. }
                if *scan_id == ScanId(scan)
        )),
        "the running scan was cancelled before the data went: {seen:?}"
    );
    assert!(
        seen.contains(&EventKind::PhoneChanged {}),
        "phone access said it changed: {seen:?}"
    );
    assert!(device_list(&server, &c).await.is_empty());
    let monitor = get(&server, &c, "/api/monitor").await;
    assert_eq!(
        monitor.body["state"], "waiting_for_scan",
        "{}",
        monitor.body
    );
    assert!(monitor.body["last"].is_null(), "{}", monitor.body);
    let phone = get(&server, &c, "/api/phone-access").await;
    assert_eq!(phone.body["state"], "off", "{}", phone.body);
    assert!(phone.body["url"].is_null());
    assert!(
        browser().get(&phone_url).send().await.is_err(),
        "the phone listener is closed"
    );
    let network = get(&server, &c, "/api/network").await.body;
    assert!(network["network_id"].is_null(), "{network}");
    assert_eq!(network["scan_consent"], false);
    assert_status_code(
        get(&server, &c, &format!("/api/scans/{scan}")).await,
        404,
        "not_found",
    );
    assert_status_code(
        get(&server, &c, "/api/scans/latest").await,
        404,
        "not_found",
    );
    let alerts = get(&server, &c, "/api/alerts").await.body;
    assert_eq!(alerts, json!([]));
    let update = get(&server, &c, "/api/update").await;
    assert!(
        update.body["setting"].is_null(),
        "asked again: {}",
        update.body
    );
    assert_eq!(store.global_setting("phone_access").await.unwrap(), None);
    for table in [
        "networks",
        "devices",
        "device_ports",
        "scans",
        "settings",
        "monitor_sessions",
        "samples",
    ] {
        assert_eq!(rows_in(&db, table).await, 0, "{table}");
    }
    // The saved "on" is gone, so a later restore leaves it off.
    server.state.phone.as_ref().unwrap().restore().await;
    assert_eq!(
        get(&server, &c, "/api/phone-access").await.body["state"],
        "off"
    );

    // 4. Again: nothing left to delete, still fine.
    let again = post_reset(&server, &c, json!({ "confirm": CONFIRM })).await;
    assert_eq!(again.status, 204, "body: {}", again.body);
    events_until_data_reset(&mut events).await;

    // And Net Lantern works as new.
    let id = run_quick_scan(&server, &c).await;
    assert_eq!(snapshot(&server, &c, id).await["state"], "done");
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    server
        .state
        .monitor
        .stop(netlantern_core::EndReason::Quit)
        .await;
    server.state.updates.stop();
}

#[tokio::test]
async fn two_resets_at_once_both_succeed_and_leave_one_empty_store() {
    let server = Server::start("home").await;
    server.state.monitor.start();
    let (c, _) = server.login().await;
    run_quick_scan(&server, &c).await;
    let mut events = server.state.bus.subscribe();

    let (a, b) = tokio::join!(
        post_reset(&server, &c, json!({ "confirm": CONFIRM })),
        post_reset(&server, &c, json!({ "confirm": CONFIRM })),
    );
    assert_eq!((a.status, b.status), (204, 204), "{} {}", a.body, b.body);
    events_until_data_reset(&mut events).await;
    events_until_data_reset(&mut events).await;

    assert!(device_list(&server, &c).await.is_empty());
    assert_eq!(
        get(&server, &c, "/api/monitor").await.body["state"],
        "waiting_for_scan"
    );
    let id = run_quick_scan(&server, &c).await;
    assert_eq!(snapshot(&server, &c, id).await["state"], "done");
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    server
        .state
        .monitor
        .stop(netlantern_core::EndReason::Quit)
        .await;
}

#[tokio::test]
async fn a_reset_that_fails_keeps_the_data_and_monitoring_runs_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("netlantern.db");
    let home = Arc::new(sim("home"));
    let server = Server::with_store(home.clone(), home, Store::new(Db::open(&path).unwrap())).await;
    let (c, _) = server.login().await;
    run_quick_scan(&server, &c).await;
    server.state.monitor.start();
    // The last delete fails, after the other tables were emptied in the same
    // transaction.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER keep_networks BEFORE DELETE ON networks
             BEGIN SELECT RAISE(ABORT, 'held by a test'); END;",
        )
        .unwrap();
    let mut events = server.state.bus.subscribe();

    let body = assert_status_code(
        post_reset(&server, &c, json!({ "confirm": CONFIRM })).await,
        500,
        "reset_failed",
    );
    assert_eq!(body["message"], "Net Lantern couldn't delete its data.");
    assert!(
        body["hint"]
            .as_str()
            .unwrap()
            .starts_with("Nothing was deleted. Phone access was turned off."),
        "{body}"
    );
    let text = body.to_string().to_lowercase();
    for leak in [
        "held by a test",
        "netlantern.db",
        "sqlite",
        "networks",
        "trigger",
    ] {
        assert!(!text.contains(leak), "{leak}: {text}");
    }

    // All or nothing: every device is still there, and monitoring is back.
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    within("a monitor.tick after the failed reset", async {
        loop {
            let event = events.recv().await.unwrap();
            assert_ne!(event.kind, EventKind::DataReset {}, "nothing was reset");
            if matches!(event.kind, EventKind::MonitorTick { .. }) {
                return;
            }
        }
    })
    .await;
    assert_eq!(
        get(&server, &c, "/api/monitor").await.body["state"],
        "monitoring"
    );
    server
        .state
        .monitor
        .stop(netlantern_core::EndReason::Quit)
        .await;
}

#[tokio::test]
async fn a_backup_that_cannot_be_removed_is_reported_after_the_data_is_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("netlantern.db");
    let home = Arc::new(sim("home"));
    let server = Server::with_store(home.clone(), home, Store::new(Db::open(&path).unwrap())).await;
    server.state.monitor.start();
    let (c, _) = server.login().await;
    run_quick_scan(&server, &c).await;
    let bak = dir.path().join("netlantern.db.bak");
    std::fs::create_dir(&bak).unwrap();
    std::fs::write(bak.join("inside"), b"x").unwrap();
    let mut events = server.state.bus.subscribe();

    let body = assert_status_code(
        post_reset(&server, &c, json!({ "confirm": CONFIRM })).await,
        500,
        "reset_failed",
    );
    assert_eq!(
        body["message"],
        "Your data was cleared, but an old backup file could not be removed."
    );
    let text = body.to_string().to_lowercase();
    assert!(
        !text.contains("netlantern.db") && !text.contains(".bak"),
        "{text}"
    );

    // The rows are gone, so everything else is as after a good reset.
    events_until_data_reset(&mut events).await;
    assert!(device_list(&server, &c).await.is_empty());
    assert_eq!(
        get(&server, &c, "/api/monitor").await.body["state"],
        "waiting_for_scan"
    );
    assert!(get(&server, &c, "/api/network").await.body["network_id"].is_null());
    server
        .state
        .monitor
        .stop(netlantern_core::EndReason::Quit)
        .await;
}

#[tokio::test]
async fn writes_wait_for_a_reset_and_then_find_the_network_gone() {
    let db = Db::open_in_memory().unwrap();
    let store = Store::new(db.clone());
    let home = Arc::new(sim("home"));
    let server = Server::with_store(home.clone(), home, store.clone()).await;
    let (c, _) = server.login().await;
    run_quick_scan(&server, &c).await;
    assert_eq!(get(&server, &c, "/api/rules").await.status, 200);

    // A reset in progress: it holds the lock while it deletes.
    let reset = server.state.reset.write().await;
    let rules = tokio::spawn({
        let c = c.clone();
        let url = server.url("/api/rules");
        async move {
            send(c.put(url).json(&json!({
                "starred_offline": false, "internet_down": true, "latency_high": true,
                "packet_loss": true, "new_device": true, "notifications": false
            })))
            .await
        }
    });
    sleep(Duration::from_millis(300)).await;
    assert!(!rules.is_finished(), "the write waits for the reset");
    // Reads are not held up.
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    server.state.store.reset_all().await.unwrap();
    server.state.scans.forget_network();
    drop(reset);

    let reply = within("the held write", rules).await.unwrap();
    assert_status_code(reply, 409, "no_network");
    assert_eq!(
        rows_in(&db, "settings").await,
        0,
        "no row for the deleted network"
    );
}

#[tokio::test]
async fn diagnoses_and_the_radio_scan_from_before_a_reset_are_gone() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let id = post_empty(&server, &c, "/api/diagnose").await.body["run_id"]
        .as_u64()
        .unwrap();
    diagnosis_when_done(&server, &c, id).await;
    assert_eq!(post_empty(&server, &c, "/api/wifi/scan").await.status, 200);

    let reset = post_reset(&server, &c, json!({ "confirm": CONFIRM })).await;
    assert_eq!(reset.status, 204, "body: {}", reset.body);

    assert_status_code(
        get(&server, &c, &format!("/api/diagnose/{id}")).await,
        404,
        "not_found",
    );
    assert_status_code(
        get(&server, &c, "/api/diagnose/latest").await,
        404,
        "not_found",
    );
    let (_, ran) = server
        .state
        .wifi
        .nearby_with_freshness(Duration::from_secs(3600))
        .await
        .unwrap();
    assert!(
        ran,
        "the cached radio scan was dropped, so this one scanned"
    );
}

#[tokio::test]
async fn a_scan_that_will_not_stop_fails_the_reset_and_deletes_nothing() {
    let db = Db::open_in_memory().unwrap();
    let store = Store::new(db.clone());
    // No phone access: turning it off would write to the busy store first.
    let home = Arc::new(sim("home"));
    let server = Server::with_store(home.clone(), home, store.clone()).await;
    let (c, _) = server.login().await;
    run_quick_scan(&server, &c).await;
    assert_eq!(put_consent(&server, &c, true).await.status, 200);
    let scan = post_scan(&server, &c, "deep").await.body["scan_id"]
        .as_u64()
        .unwrap();
    // The database thread is busy for 7 s, so the cancelled scan cannot
    // record its end within the reset's 5 s.
    let busy = tokio::spawn({
        let db = db.clone();
        async move {
            db.call(|_| {
                std::thread::sleep(Duration::from_secs(7));
                Ok(())
            })
            .await
        }
    });
    sleep(Duration::from_millis(50)).await;

    let body = assert_status_code(
        post_reset(&server, &c, json!({ "confirm": CONFIRM })).await,
        500,
        "reset_failed",
    );
    assert_eq!(body["message"], "Net Lantern couldn't delete its data.");
    assert_eq!(
        body["hint"],
        "Nothing was deleted. Phone access was turned off. Try again. If this keeps happening, copy the debug info into a bug report."
    );

    busy.await.unwrap().unwrap();
    wait_for_state(&server, &c, scan, "cancelled").await;
    assert_eq!(device_list(&server, &c).await.len(), HOME_DEVICES);
    assert!(get(&server, &c, "/api/network").await.body["network_id"].is_number());
    assert_eq!(rows_in(&db, "networks").await, 1);
}

// ------------------------------------------------------------ host, autostart

/// Start at login kept in a flag, or failing every call.
struct FakeAutostart {
    on: std::sync::atomic::AtomicBool,
    fails: bool,
    /// How many times `set_enabled` was called.
    sets: std::sync::atomic::AtomicUsize,
}

impl Autostart for FakeAutostart {
    fn is_enabled(&self) -> Result<bool, String> {
        if self.fails {
            return Err("the registry said no".into());
        }
        Ok(self.on.load(std::sync::atomic::Ordering::SeqCst))
    }

    fn set_enabled(&self, on: bool) -> Result<(), String> {
        self.sets.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fails {
            return Err("the registry said no".into());
        }
        self.on.store(on, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn desktop_host(fails: bool) -> HostInfo {
    HostInfo {
        kind: HostKind::Desktop,
        version: "9.9.9",
        autostart: Some(Arc::new(FakeAutostart {
            on: std::sync::atomic::AtomicBool::new(false),
            fails,
            sets: std::sync::atomic::AtomicUsize::new(0),
        })),
    }
}

async fn put_autostart(server: &Server, client: &Client, enabled: bool) -> Reply {
    send(
        client
            .put(server.url("/api/host/autostart"))
            .json(&json!({ "enabled": enabled })),
    )
    .await
}

#[tokio::test]
async fn host_reports_exe_without_autostart() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    let reply = get(&server, &c, "/api/host").await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.body["kind"], "exe");
    assert_eq!(reply.body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(reply.body["autostart"]["available"], false);
    assert!(reply.body["autostart"]["enabled"].is_null());
}

#[tokio::test]
async fn desktop_autostart_round_trips() {
    let server = Server::with_host(desktop_host(false)).await;
    let (c, _) = server.login().await;
    let reply = get(&server, &c, "/api/host").await;
    assert_eq!(reply.body["kind"], "desktop");
    assert_eq!(reply.body["version"], "9.9.9");
    assert_eq!(reply.body["autostart"]["available"], true);
    assert_eq!(reply.body["autostart"]["enabled"], false);

    let on = put_autostart(&server, &c, true).await;
    assert_eq!(on.status, 200, "{}", on.body);
    assert_eq!(on.body["autostart"]["enabled"], true);
    assert_eq!(
        get(&server, &c, "/api/host").await.body["autostart"]["enabled"],
        true
    );
    let off = put_autostart(&server, &c, false).await;
    assert_eq!(off.body["autostart"]["enabled"], false);
    assert_eq!(off.body["kind"], "desktop");
}

#[tokio::test]
async fn autostart_unavailable_is_409() {
    let server = Server::start("home").await;
    let (c, _) = server.login().await;
    assert_status_code(
        put_autostart(&server, &c, true).await,
        409,
        "autostart_unavailable",
    );
}

#[tokio::test]
async fn autostart_error_is_500_autostart_failed() {
    let server = Server::with_host(desktop_host(true)).await;
    let (c, _) = server.login().await;
    // A failing read is not an error for the page: the switch is just unknown.
    let reply = get(&server, &c, "/api/host").await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.body["autostart"]["available"], true);
    assert!(reply.body["autostart"]["enabled"].is_null());

    let body = assert_status_code(
        put_autostart(&server, &c, true).await,
        500,
        "autostart_failed",
    );
    assert!(
        !body.to_string().contains("registry"),
        "the OS text must not reach the client: {body}"
    );
    assert_eq!(body["message"], "Couldn't change start at login.");
    // The failed change left the reported state as it was.
    let after = get(&server, &c, "/api/host").await;
    assert!(after.body["autostart"]["enabled"].is_null());
}

#[tokio::test]
async fn malformed_and_repeated_autostart_requests() {
    let server = Server::with_host(desktop_host(false)).await;
    let (c, _) = server.login().await;
    for body in [json!({}), json!({ "enabled": "yes" })] {
        let reply = send(c.put(server.url("/api/host/autostart")).json(&body)).await;
        assert_status_code(reply, 400, "bad_request");
    }
    let reply = send(
        c.put(server.url("/api/host/autostart"))
            .header("content-type", "application/json")
            .body("not json"),
    )
    .await;
    assert_status_code(reply, 400, "bad_request");
    // The same value twice is fine and changes nothing more.
    for _ in 0..2 {
        let on = put_autostart(&server, &c, true).await;
        assert_eq!(on.status, 200, "{}", on.body);
        assert_eq!(on.body["autostart"]["enabled"], true);
    }
    assert_eq!(
        get(&server, &c, "/api/host").await.body["autostart"]["enabled"],
        true
    );
}

#[tokio::test]
async fn phone_sessions_read_the_host_but_cannot_change_start_at_login() {
    let flag = Arc::new(FakeAutostart {
        on: std::sync::atomic::AtomicBool::new(false),
        fails: false,
        sets: std::sync::atomic::AtomicUsize::new(0),
    });
    let server = Server::with_host(HostInfo {
        kind: HostKind::Desktop,
        version: "9.9.9",
        autostart: Some(flag.clone()),
    })
    .await;
    // A phone session, made from the phone token.
    let phone = browser();
    let token = server.state.auth.rotate_phone_token();
    assert_eq!(post_session(&server, &phone, &token).await.status, 204);

    let read = get(&server, &phone, "/api/host").await;
    assert_eq!(read.status, 200, "{}", read.body);
    assert_eq!(read.body["autostart"]["enabled"], false);
    let body = assert_status_code(
        put_autostart(&server, &phone, true).await,
        403,
        "phone_not_allowed",
    );
    assert_eq!(body["message"], "This can only be changed on the laptop.");
    // Refused before the body is even read, and before any side effect.
    assert_status_code(
        send(
            phone
                .put(server.url("/api/host/autostart"))
                .header("content-type", "application/json")
                .body("not json"),
        )
        .await,
        403,
        "phone_not_allowed",
    );
    assert_eq!(flag.sets.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(!flag.on.load(std::sync::atomic::Ordering::SeqCst));

    // The laptop's own session still can.
    let (local, _) = server.login().await;
    assert_eq!(get(&server, &local, "/api/host").await.status, 200);
    assert_eq!(put_autostart(&server, &local, true).await.status, 200);
    assert_eq!(flag.sets.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(flag.on.load(std::sync::atomic::Ordering::SeqCst));
}

// Covers the guard only; the positive tests above cover the routes.
#[tokio::test]
async fn host_routes_need_a_session() {
    let server = Server::with_host(desktop_host(false)).await;
    let c = browser();
    assert_status_code(get(&server, &c, "/api/host").await, 401, "unauthorized");
    assert_status_code(put_autostart(&server, &c, true).await, 401, "unauthorized");
}
