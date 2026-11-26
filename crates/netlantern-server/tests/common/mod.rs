//! Helpers shared by the end-to-end test binaries: a real server on
//! `127.0.0.1:0` on a simulated network, a cookie-keeping client, and a
//! response reader. Each binary uses only some of them.
#![allow(dead_code)]

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use netlantern_core::discover::DiscoverConfig;
use netlantern_core::{Db, DiagnoseConfig, ScanConfig, Store};
use netlantern_platform::{NetIo, Platform, SimNetwork};
use netlantern_server::{
    router, AccessPolicy, AppState, HostInfo, LogTaps, LoopbackLanBinder, PhoneAccess,
};
use reqwest::header::HeaderMap;
use reqwest::{Client, RequestBuilder};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::timeout;

/// The longest any single wait may take before the test fails.
pub const WAIT: Duration = Duration::from_secs(10);

pub async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {WAIT:?} waiting for {what}"))
}

/// A server running in the background on its own port, plus a handle on its
/// state (for launch tokens and the event bus).
pub struct Server {
    pub port: u16,
    pub state: AppState,
}

impl Server {
    /// A server on one of the simulated networks in `scenarios/`.
    pub async fn start(scenario: &str) -> Self {
        let sim = Arc::new(sim(scenario));
        Self::with_platform(sim.clone(), sim).await
    }

    pub async fn with_platform(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>) -> Self {
        Self::with_store(platform, netio, Store::new(Db::open_in_memory().unwrap())).await
    }

    pub async fn with_store(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
    ) -> Self {
        Self::build(platform, netio, store, None).await
    }

    /// A server whose debug info shows the log lines `logs` holds.
    pub async fn with_logs(scenario: &str, logs: Arc<Mutex<Vec<String>>>) -> Self {
        let sim = Arc::new(sim(scenario));
        let store = Store::new(Db::open_in_memory().unwrap());
        let taps = LogTaps {
            lines: Arc::new(move || logs.lock().unwrap().clone()),
            problems: Arc::new(Vec::new),
        };
        Self::build(sim.clone(), sim, store, Some(taps)).await
    }

    async fn build(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        logs: Option<LogTaps>,
    ) -> Self {
        Self::build_with(platform, netio, store, logs, false).await
    }

    /// A server with phone access attached, on `LoopbackLanBinder` (the phone
    /// listener is a second loopback port), as the exe wires it.
    pub async fn with_phone(scenario: &str, store: Store) -> Self {
        let sim = Arc::new(sim(scenario));
        Self::build_with(sim.clone(), sim, store, None, true).await
    }

    /// A server run by the given host (the exe or the desktop app).
    pub async fn with_host(host: HostInfo) -> Self {
        let sim = Arc::new(sim("home"));
        let store = Store::new(Db::open_in_memory().unwrap());
        Self::build_host(sim.clone(), sim, store, None, false, Some(host)).await
    }

    async fn build_with(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        logs: Option<LogTaps>,
        phone: bool,
    ) -> Self {
        Self::build_host(platform, netio, store, logs, phone, None).await
    }

    async fn build_host(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        logs: Option<LogTaps>,
        phone: bool,
        host: Option<HostInfo>,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let mut state = AppState::new(
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
            DiagnoseConfig::default(),
            AccessPolicy::for_port(port, &[]),
        );
        state.logs = logs;
        if let Some(host) = host {
            state.host = host;
        }
        if phone {
            let access = PhoneAccess::new(
                Arc::clone(&state.platform),
                state.store.clone(),
                Arc::clone(&state.auth),
                Arc::clone(&state.bus),
                Arc::new(LoopbackLanBinder),
                std::path::PathBuf::from("netlantern.exe"),
                port,
                true,
            );
            state.phone = Some(Arc::clone(&access));
            access.attach_router(router(state.clone()));
        }
        let app = router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { port, state }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// A browser that has redeemed a fresh launch token, and its session
    /// cookie as a `Cookie` header value (`nl_session=<id>`).
    pub async fn login(&self) -> (Client, String) {
        let client = browser();
        let token = self.state.auth.issue_launch_token();
        let res = post_session(self, &client, &token).await;
        assert_eq!(res.status, 204);
        let cookie = res.headers["set-cookie"].to_str().unwrap();
        let pair = cookie.split(';').next().unwrap().to_string();
        (client, pair)
    }
}

pub fn sim(scenario: &str) -> SimNetwork {
    SimNetwork::load(format!(
        "{}/../../scenarios/{scenario}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A client that keeps cookies, as a browser does, and ignores any system
/// proxy so loopback requests stay on loopback.
pub fn browser() -> Client {
    Client::builder()
        .cookie_store(true)
        .no_proxy()
        .build()
        .unwrap()
}

/// A response with its body parsed. Building one asserts the API-version
/// header and that a non-empty body is JSON (never axum's plain-text
/// rejections).
pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Value,
}

pub async fn send(request: RequestBuilder) -> Reply {
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

pub async fn post_session(server: &Server, client: &Client, token: &str) -> Reply {
    send(
        client
            .post(server.url("/api/session"))
            .json(&json!({ "token": token })),
    )
    .await
}
