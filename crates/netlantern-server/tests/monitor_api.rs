//! The monitoring routes, end to end: a real server on `127.0.0.1:0` whose
//! monitor runs for real against `printer-offline` with the clock sped up
//! (scenario time x`SPEED`, monitor intervals divided by `SPEED`), so a whole
//! outage fits in a few seconds. Every wait has a deadline.
//!
//! The scenario clock starts when the server is built, so the outage is
//! placed late enough for the scan and the star to land before it, and is
//! long enough that an ack lands well before the resolve, even on a slow
//! runner.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use netlantern_core::discover::DiscoverConfig;
use netlantern_core::{
    Db, DiagnoseConfig, EventKind, MonitorConfig, NoopNotifier, ScanConfig, ScanId, Store,
    TokioClock,
};
use netlantern_platform::SimNetwork;
use netlantern_server::{router, AccessPolicy, AppState, MonitorParts};
use reqwest::{Client, Method};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const SPEED: u32 = 60;
/// The printer is offline from scenario second 300 to 900: 5 s to 15 s after
/// the server starts. It alerts about 2.5 s into that, after its grace.
const OUTAGE_S: (u64, u64) = (300, 900);
const WAIT: Duration = Duration::from_secs(20);

struct Server {
    port: u16,
    state: AppState,
    client: Client,
    cookie: String,
}

impl Server {
    async fn start() -> Self {
        let path = format!(
            "{}/../../scenarios/printer-offline.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut json: Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        json["timeline"] = json!([{
            "ip": "192.168.1.40", "from_s": OUTAGE_S.0, "until_s": OUTAGE_S.1,
            "set": {"present": false}
        }]);
        let sim = SimNetwork::from_json(&json.to_string()).unwrap();
        let sim = Arc::new(sim.with_time_scale(SPEED));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let epoch = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let state = AppState::with_monitor(
            sim.clone(),
            sim,
            Store::new(Db::open_in_memory().unwrap()),
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
            DiagnoseConfig::default(),
            MonitorParts {
                config: MonitorConfig::default().scaled(SPEED),
                notifier: Arc::new(NoopNotifier),
                clock: Arc::new(TokioClock::new(epoch)),
            },
            AccessPolicy::for_port(port, &[]),
        );
        let app = router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::builder()
            .cookie_store(true)
            .no_proxy()
            .build()
            .unwrap();
        let token = state.auth.issue_launch_token();
        let res = client
            .post(format!("http://127.0.0.1:{port}/api/session"))
            .json(&json!({ "token": token }))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 204);
        let cookie = res.headers()["set-cookie"].to_str().unwrap();
        let cookie = cookie.split(';').next().unwrap().to_string();
        state.monitor.start();
        Self {
            port,
            state,
            client,
            cookie,
        }
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut req = self
            .client
            .request(method, format!("http://127.0.0.1:{}{path}", self.port));
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = within("an HTTP response", req.send()).await.unwrap();
        let status = res.status().as_u16();
        let text = res.text().await.unwrap();
        let body = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text:?}"))
        };
        (status, body)
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        self.call(Method::GET, path, None).await
    }

    async fn scan(&self) {
        let mut rx = self.state.bus.subscribe();
        let (status, body) = self
            .call(Method::POST, "/api/scans", Some(json!({"mode": "quick"})))
            .await;
        assert_eq!(status, 202, "{body}");
        let id = ScanId(body["scan_id"].as_u64().unwrap());
        within("scan.done", async {
            loop {
                if let EventKind::ScanDone { scan_id, .. } = rx.recv().await.unwrap().kind {
                    if scan_id == id {
                        return;
                    }
                }
            }
        })
        .await;
    }

    /// The printer's device id (the device at 192.168.1.40).
    async fn printer(&self) -> i64 {
        let (_, devices) = self.get("/api/devices").await;
        devices
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["ip"] == "192.168.1.40")
            .expect("the printer")["id"]
            .as_i64()
            .unwrap()
    }

    async fn star_printer(&self) -> i64 {
        let id = self.printer().await;
        let (status, body) = self
            .call(
                Method::PATCH,
                &format!("/api/devices/{id}"),
                Some(json!({"starred": true, "name": "Office printer"})),
            )
            .await;
        assert_eq!(status, 200, "{body}");
        id
    }

    /// Polls `path` until `done` accepts the answer.
    async fn until(&self, what: &str, path: &str, done: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + WAIT;
        loop {
            let (status, body) = self.get(path).await;
            assert_eq!(status, 200, "{body}");
            if done(&body) {
                return body;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {WAIT:?} waiting for {what}"))
}

fn assert_error(reply: (u16, Value), status: u16, code: &str) -> Value {
    assert_eq!(reply.0, status, "{}", reply.1);
    assert_eq!(reply.1["code"], code, "{}", reply.1);
    assert!(reply.1["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(reply.1.as_object().unwrap().contains_key("hint"));
    reply.1
}

#[tokio::test]
async fn monitor_reports_waiting_for_scan_then_monitoring() {
    let s = Server::start().await;
    let (status, body) = s.get("/api/monitor").await;
    assert_eq!(status, 200);
    assert_eq!(body["state"], "waiting_for_scan");
    assert_eq!(body["session_started_at"], Value::Null);

    s.scan().await;
    let body = s
        .until("monitoring", "/api/monitor", |b| b["state"] == "monitoring")
        .await;
    assert!(body["session_started_at"].is_i64(), "{body}");
    assert!(body["network_id"].is_i64(), "{body}");
}

#[tokio::test]
async fn starring_the_printer_leads_to_an_alert_an_incident_and_a_resolve() {
    let s = Server::start().await;
    s.scan().await;
    let mut ws = {
        let mut req = format!("ws://127.0.0.1:{}/api/events", s.port)
            .into_client_request()
            .unwrap();
        req.headers_mut()
            .insert("cookie", s.cookie.parse().unwrap());
        req.headers_mut().insert(
            "origin",
            format!("http://127.0.0.1:{}", s.port).parse().unwrap(),
        );
        within("ws", tokio_tungstenite::connect_async(req))
            .await
            .unwrap()
            .0
    };
    let printer = s.star_printer().await;

    let alert = within("alert.changed", async {
        loop {
            let Some(Ok(Message::Text(text))) = ws.next().await else {
                continue;
            };
            let v: Value = serde_json::from_str(&text).unwrap();
            if v["type"] == "alert.changed" {
                return v["data"]["alert"].clone();
            }
        }
    })
    .await;
    assert_eq!(alert["title"], "Office printer is offline");
    assert_eq!(alert["key"], format!("starred_offline:{printer}"));

    let active = s
        .until("an active alert", "/api/alerts?state=active", |b| {
            !b.as_array().unwrap().is_empty()
        })
        .await;
    let alert = &active[0];
    assert_eq!(alert["title"], "Office printer is offline");
    let (id, incident_id) = (alert["id"].as_i64().unwrap(), alert["incident_id"].clone());

    let (status, incident) = s.get(&format!("/api/incidents/{incident_id}")).await;
    assert_eq!(status, 200, "{incident}");
    let kinds: Vec<&str> = incident["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"detected"), "{kinds:?}");
    assert!(kinds.contains(&"alerted"), "{kinds:?}");

    let (status, acked) = s
        .call(Method::POST, &format!("/api/alerts/{id}/ack"), None)
        .await;
    assert_eq!(status, 200, "{acked}");
    assert_eq!(acked["state"], "acked");

    s.until("the alert to resolve", "/api/alerts?state=resolved", |b| {
        !b.as_array().unwrap().is_empty()
    })
    .await;
    let (_, incident) = s.get(&format!("/api/incidents/{incident_id}")).await;
    assert!(incident["resolved_at"].is_i64(), "{incident}");
}

#[tokio::test]
async fn metrics_validate_their_query() {
    let s = Server::start().await;
    // No network yet: an empty series, not an error.
    let (status, body) = s
        .get("/api/metrics?target=gateway&metric=rtt_ms&range=1h")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["points"], json!([]));
    assert_eq!(body["sessions"], json!([]));

    let body = assert_error(
        s.get("/api/metrics?target=gateway&metric=rtt_ms&range=2h")
            .await,
        400,
        "bad_request",
    );
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("1h, 24h, 7d, 30d"));
    assert_error(
        s.get("/api/metrics?target=router&metric=rtt_ms&range=1h")
            .await,
        400,
        "bad_request",
    );
    assert_error(
        s.get("/api/metrics?target=gateway&metric=speed&range=1h")
            .await,
        400,
        "bad_request",
    );
    assert_error(
        s.get("/api/metrics?target=gateway").await,
        400,
        "bad_request",
    );

    s.scan().await;
    s.until(
        "gateway points",
        "/api/metrics?target=gateway&metric=rtt_ms&range=1h",
        |b| !b["points"].as_array().unwrap().is_empty(),
    )
    .await;
    let (status, body) = s
        .get("/api/metrics?target=device:1&metric=rtt_ms&range=7d")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["resolution"], "five_min");
}

#[tokio::test]
async fn rules_need_a_known_network_and_round_trip() {
    let s = Server::start().await;
    assert_error(s.get("/api/rules").await, 409, "no_network");
    let all = json!({"starred_offline": true, "internet_down": true, "latency_high": true,
        "packet_loss": true, "new_device": true, "notifications": true});
    assert_error(
        s.call(Method::PUT, "/api/rules", Some(all.clone())).await,
        409,
        "no_network",
    );

    s.scan().await;
    let (status, body) = s.get("/api/rules").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, all);

    let changed = json!({"starred_offline": true, "internet_down": false, "latency_high": false,
        "packet_loss": true, "new_device": false, "notifications": false});
    let (status, body) = s
        .call(Method::PUT, "/api/rules", Some(changed.clone()))
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, changed);
    assert_eq!(s.get("/api/rules").await.1, changed);

    // All six fields are required and nothing else is accepted.
    assert_error(
        s.call(
            Method::PUT,
            "/api/rules",
            Some(json!({"latency_high": false})),
        )
        .await,
        400,
        "bad_request",
    );
    let mut extra = changed.clone();
    extra["bogus"] = json!(true);
    assert_error(
        s.call(Method::PUT, "/api/rules", Some(extra)).await,
        400,
        "bad_request",
    );
}

#[tokio::test]
async fn device_checks_are_validated() {
    let s = Server::start().await;
    s.scan().await;
    let id = s.printer().await;
    let path = format!("/api/devices/{id}");
    let patch = |checks: Value| s.call(Method::PATCH, &path, Some(json!({ "checks": checks })));
    let tcp = |port: u64| json!({"kind": "tcp", "port": port});

    let six: Vec<Value> = (1..=6).map(tcp).collect();
    let body = assert_error(patch(json!(six)).await, 400, "bad_request");
    assert!(body["message"].as_str().unwrap().contains('5'), "{body}");
    for bad in [
        json!({"kind": "http", "port": 80, "path": "status", "https": false}),
        json!({"kind": "http", "port": 80, "path": "/a b", "https": false}),
        json!({"kind": "http", "port": 80, "path": format!("/{}", "a".repeat(200)), "https": false}),
        json!({"kind": "http", "port": 0, "path": "/", "https": false}),
        json!({"kind": "tcp", "port": 0}),
        json!({"kind": "tcp", "port": 70000}),
    ] {
        assert_error(patch(json!([bad])).await, 400, "bad_request");
    }

    let good = json!([tcp(631), {"kind": "http", "port": 80, "path": "/status", "https": false}]);
    let (status, body) = patch(good.clone()).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["checks"], good);
    assert_eq!(s.get(&path).await.1["checks"], good);
    // Clearing works.
    let (status, body) = patch(json!([])).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["checks"], json!([]));
}

#[tokio::test]
async fn acking_a_resolved_alert_is_a_conflict() {
    let s = Server::start().await;
    s.scan().await;
    s.star_printer().await;
    let resolved = s
        .until("a resolved alert", "/api/alerts?state=resolved", |b| {
            !b.as_array().unwrap().is_empty()
        })
        .await;
    let id = resolved[0]["id"].as_i64().unwrap();
    let body = assert_error(
        s.call(Method::POST, &format!("/api/alerts/{id}/ack"), None)
            .await,
        409,
        "alert_resolved",
    );
    assert_eq!(body["message"], "This alert has already been resolved.");

    assert_error(
        s.call(Method::POST, "/api/alerts/99999/ack", None).await,
        404,
        "not_found",
    );
    assert_error(
        s.call(Method::POST, "/api/alerts/abc/ack", None).await,
        404,
        "not_found",
    );
    assert_error(s.get("/api/incidents/99999").await, 404, "not_found");
    // Bad filters are 400s; the limit is capped rather than refused.
    assert_error(s.get("/api/alerts?state=bogus").await, 400, "bad_request");
    assert_error(s.get("/api/alerts?limit=x").await, 400, "bad_request");
    let (status, _) = s.get("/api/alerts?limit=5000").await;
    assert_eq!(status, 200);
}
