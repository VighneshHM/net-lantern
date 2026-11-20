//! Phone access end to end: a main server on `127.0.0.1:0`, phone access with
//! `LoopbackLanBinder` (the phone listener is a second loopback port), the sim
//! platform and an in-memory store.
//!
//! No test prints or asserts on a token's value beyond comparing tokens.

use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use netlantern_core::{Db, DiagnoseConfig, ScanConfig, Store, PHONE_ACCESS_KEY};
use netlantern_platform::{FirewallVerdict, NetworkCategory, SimNetwork};
use netlantern_server::{
    router, run, AccessPolicy, AppState, LoopbackLanBinder, PhoneAccess, PhoneState, PhoneStatus,
};
use reqwest::header::SET_COOKIE;
use reqwest::{Client, Response};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// The longest any single wait may take before the test fails.
const WAIT: Duration = Duration::from_secs(10);

const PUBLIC_HINT: &str = "Open Settings → Network & internet → Wi-Fi → your network, and set Network profile type to Private.";

// ---------------------------------------------------------------- helpers

async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out after {WAIT:?} waiting for {what}"))
}

/// A running main server with phone access attached, on one scenario.
struct Lab {
    sim: Arc<SimNetwork>,
    store: Store,
    state: AppState,
    phone: Arc<PhoneAccess>,
    main_port: u16,
}

fn memory_store() -> Store {
    Store::new(Db::open_in_memory().unwrap())
}

async fn lab(scenario: &str) -> Lab {
    lab_with_store(scenario, memory_store()).await
}

/// What the exe does: build the state, the phone access from it, put the one
/// in the other, attach the router, serve.
async fn lab_with_store(scenario: &str, store: Store) -> Lab {
    let sim = SimNetwork::load(format!(
        "{}/../../scenarios/{scenario}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let sim = Arc::new(sim);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let main_port = listener.local_addr().unwrap().port();
    let mut state = AppState::new(
        sim.clone(),
        sim.clone(),
        store.clone(),
        ScanConfig::default(),
        DiagnoseConfig::default(),
        AccessPolicy::for_port(main_port, &[]),
    );
    let phone = PhoneAccess::new(
        sim.clone(),
        store.clone(),
        Arc::clone(&state.auth),
        Arc::clone(&state.bus),
        Arc::new(LoopbackLanBinder),
        PathBuf::from("netlantern.exe"),
        main_port,
        true,
    );
    state.phone = Some(Arc::clone(&phone));
    phone.attach_router(router(state.clone()));
    tokio::spawn(run(listener, state.clone(), std::future::pending()));
    Lab {
        sim,
        store,
        state,
        phone,
        main_port,
    }
}

impl Lab {
    fn main_url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.main_port)
    }

    /// A browser on this laptop, signed in with a launch token.
    async fn local(&self) -> Client {
        let client = browser();
        let token = self.state.auth.issue_launch_token();
        let res = post_json(
            &client,
            self.main_url("/api/session"),
            json!({ "token": token }),
        )
        .await;
        assert_eq!(res.status(), 204);
        client
    }

    async fn setting(&self) -> Option<String> {
        self.store.global_setting(PHONE_ACCESS_KEY).await.unwrap()
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

async fn post_json(client: &Client, url: String, body: Value) -> Response {
    within("an HTTP response", client.post(url).json(&body).send())
        .await
        .unwrap()
}

/// `http://host:port/?t=token` split into the base and the token.
fn split(url: &str) -> (String, String) {
    let (base, token) = url
        .split_once("/?t=")
        .unwrap_or_else(|| panic!("not a phone URL"));
    assert!(base.starts_with("http://"));
    assert_eq!(token.len(), 64, "the token is 32 bytes of hex");
    (base.to_string(), token.to_string())
}

fn phone_url(status: &PhoneStatus) -> (String, String) {
    split(status.url.as_deref().expect("a URL while On"))
}

/// Redeems `token` on the phone listener; returns the status and the cookie.
async fn redeem(client: &Client, base: &str, token: &str) -> (u16, Option<String>) {
    let res = post_json(
        client,
        format!("{base}/api/session"),
        json!({ "token": token }),
    )
    .await;
    let cookie = res
        .headers()
        .get(SET_COOKIE)
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    (res.status().as_u16(), cookie)
}

async fn json_body(res: Response) -> Value {
    within("a JSON body", res.json()).await.unwrap()
}

fn addr_of(base: &str) -> SocketAddr {
    base.trim_start_matches("http://").parse().unwrap()
}

/// Steps the paused clock past one 5 s watcher tick and waits until the
/// status satisfies `done`. Real time resumes before returning, so HTTP
/// timeouts in the caller run on the real clock.
async fn step_watcher(phone: &PhoneAccess, done: impl Fn(&PhoneStatus) -> bool) -> PhoneStatus {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5)).await;
    let mut reached = None;
    for _ in 0..1000 {
        let status = phone.status().await;
        if done(&status) {
            reached = Some(status);
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::time::resume();
    reached.expect("the watcher acted on one 5 s step")
}

// ------------------------------------------------------------------ tests

#[tokio::test]
async fn enable_serves_the_dashboard_on_the_phone_address_with_the_token() {
    let lab = lab("home").await;
    let status = lab.phone.enable().await.unwrap();
    assert_eq!(status.state, PhoneState::On);
    let (base, token) = phone_url(&status);
    assert_ne!(
        addr_of(&base).port(),
        lab.main_port,
        "a listener of its own"
    );

    let phone = browser();
    assert_eq!(get(&phone, format!("{base}/")).await.status(), 200);
    let (code, cookie) = redeem(&phone, &base, &token).await;
    assert_eq!(code, 204);
    assert!(cookie.is_some());
    assert_eq!(
        get(&phone, format!("{base}/api/network")).await.status(),
        200
    );
    // The token is reusable: a second phone signs in too.
    assert_eq!(redeem(&browser(), &base, &token).await.0, 204);
}

#[tokio::test]
async fn refused_without_token() {
    let lab = lab("home").await;
    let (base, _token) = phone_url(&lab.phone.enable().await.unwrap());
    let phone = browser();

    assert_eq!(
        get(&phone, format!("{base}/api/network")).await.status(),
        401
    );
    let wrong = "0".repeat(64);
    assert_eq!(redeem(&phone, &base, &wrong).await.0, 401);
    assert_eq!(
        get(&phone, format!("{base}/api/network")).await.status(),
        401
    );
}

#[tokio::test]
async fn lan_listener_refuses_a_foreign_host() {
    let lab = lab("home").await;
    let (base, _token) = phone_url(&lab.phone.enable().await.unwrap());
    let host = lab.phone.allowed_host().unwrap();
    assert_eq!(format!("http://{host}"), base);
    assert_eq!(lab.phone.allowed_origin().unwrap(), base);
    let c = browser();

    let foreign_host = within(
        "an HTTP response",
        c.get(format!("{base}/"))
            .header("Host", "evil.example")
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(foreign_host.status(), 403);
    assert_eq!(json_body(foreign_host).await["code"], "forbidden_host");

    let foreign_origin = within(
        "an HTTP response",
        c.get(format!("{base}/"))
            .header("Origin", "http://evil.example")
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(foreign_origin.status(), 403);
    assert_eq!(json_body(foreign_origin).await["code"], "forbidden_origin");

    // Its own Origin is fine.
    let same_origin = within(
        "an HTTP response",
        c.get(format!("{base}/")).header("Origin", &base).send(),
    )
    .await
    .unwrap();
    assert_eq!(same_origin.status(), 200);

    // The guard is shared: the phone host passes on the main listener while
    // On, and is refused once phone access is off.
    let as_phone_host = |c: &Client| {
        c.get(lab.main_url("/"))
            .header("Host", &host)
            .header("Origin", &base)
            .send()
    };
    let before = within("an HTTP response", as_phone_host(&c)).await.unwrap();
    assert_eq!(before.status(), 200);
    lab.phone.disable().await;
    assert_eq!(lab.phone.allowed_host(), None);
    assert_eq!(lab.phone.allowed_origin(), None);
    let after = within("an HTTP response", as_phone_host(&c)).await.unwrap();
    assert_eq!(after.status(), 403);
    assert_eq!(json_body(after).await["code"], "forbidden_host");
}

#[tokio::test]
async fn old_phone_token_is_refused_after_a_restart_of_phone_access() {
    let lab = lab("home").await;
    let (_, first) = phone_url(&lab.phone.enable().await.unwrap());
    lab.phone.disable().await;
    let (base, second) = phone_url(&lab.phone.enable().await.unwrap());
    assert_ne!(first, second);

    assert_eq!(redeem(&browser(), &base, &first).await.0, 401);
    assert_eq!(redeem(&browser(), &base, &second).await.0, 204);
    // Not on the main listener either.
    let res = post_json(
        &browser(),
        lab.main_url("/api/session"),
        json!({ "token": first }),
    )
    .await;
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn disable_ends_phone_sessions_and_stops_within_the_grace() {
    let lab = lab("home").await;
    let (base, token) = phone_url(&lab.phone.enable().await.unwrap());
    let addr = addr_of(&base);

    // An idle keep-alive client: its pooled connection stays open.
    let phone = browser();
    let (code, cookie) = redeem(&phone, &base, &token).await;
    assert_eq!(code, 204);
    let cookie = cookie.unwrap();
    assert_eq!(
        get(&phone, format!("{base}/api/network")).await.status(),
        200
    );

    // An open phone WebSocket.
    let mut request = format!("ws://{addr}/api/events")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    let (mut ws, _) = within(
        "the WebSocket handshake",
        tokio_tungstenite::connect_async(request),
    )
    .await
    .unwrap();
    let hello = within("the hello", ws.next()).await.unwrap().unwrap();
    assert!(hello.to_text().unwrap().contains("hello"));

    let started = Instant::now();
    let off = within("disable", lab.phone.disable()).await;
    let took = started.elapsed();
    assert!(took < Duration::from_millis(2500), "disable took {took:?}");
    assert_eq!(off.state, PhoneState::Off);
    assert_eq!(off.url, None);
    assert_eq!(lab.setting().await.as_deref(), Some("off"));

    // The socket closes.
    within("the WebSocket to close", async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;

    // The session is gone everywhere, the main listener included.
    let res = within(
        "an HTTP response",
        browser()
            .get(lab.main_url("/api/network"))
            .header("cookie", &cookie)
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(res.status(), 401);

    // Nothing listens on the phone port any more.
    assert!(
        within("a connection attempt", TcpStream::connect(addr))
            .await
            .is_err(),
        "the phone port still accepts connections"
    );
}

#[tokio::test]
async fn public_network_refuses_enable_with_409_public_network() {
    let lab = lab("cafe").await;
    let local = lab.local().await;

    let res = post_json(
        &local,
        lab.main_url("/api/phone-access"),
        json!({ "enabled": true }),
    )
    .await;
    assert_eq!(res.status(), 409);
    let body = json_body(res).await;
    assert_eq!(body["code"], "public_network", "{body}");
    assert_eq!(body["hint"], PUBLIC_HINT, "{body}");

    // Saved "on", so it resumes on a Private network; paused until then.
    assert_eq!(lab.setting().await.as_deref(), Some("on"));
    let status = json_body(get(&local, lab.main_url("/api/phone-access")).await).await;
    assert_eq!(status["state"], "paused", "{status}");
    assert_eq!(status["reason"], "public_network", "{status}");
    assert_eq!(status["url"], Value::Null, "{status}");
    assert_eq!(lab.phone.allowed_host(), None);

    // The direct call says the same.
    let err = lab.phone.enable().await.unwrap_err();
    assert_eq!(err.status.as_u16(), 409);
    assert_eq!(err.code, "public_network");
}

#[tokio::test]
async fn phone_access_routes_need_a_session_and_toggle() {
    let lab = lab("home").await;
    let stranger = browser();
    assert_eq!(
        get(&stranger, lab.main_url("/api/phone-access"))
            .await
            .status(),
        401
    );
    assert_eq!(
        post_json(
            &stranger,
            lab.main_url("/api/phone-access"),
            json!({ "enabled": true })
        )
        .await
        .status(),
        401
    );

    let local = lab.local().await;
    let mut events = lab.state.bus.subscribe();
    let on = post_json(
        &local,
        lab.main_url("/api/phone-access"),
        json!({ "enabled": true }),
    )
    .await;
    assert_eq!(on.status(), 200);
    let on = json_body(on).await;
    assert_eq!(on["state"], "on", "{on}");
    assert_eq!(on["demo"], true);
    assert!(on["url"].as_str().unwrap().contains("/?t="));
    let event = within("phone.changed", events.recv()).await.unwrap();
    assert_eq!(
        serde_json::to_value(&event).unwrap()["type"],
        "phone.changed"
    );

    let off = post_json(
        &local,
        lab.main_url("/api/phone-access"),
        json!({ "enabled": false }),
    )
    .await;
    assert_eq!(off.status(), 200);
    let off = json_body(off).await;
    assert_eq!(off["state"], "off", "{off}");
    assert_eq!(off["url"], Value::Null);

    let bad = post_json(
        &local,
        lab.main_url("/api/phone-access"),
        json!({ "enabled": "yes" }),
    )
    .await;
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn rebinds_when_the_address_changes_and_pauses_on_a_public_network() {
    let lab = lab("home").await;
    let first = lab.phone.enable().await.unwrap();
    let (old_base, token) = phone_url(&first);
    let old_host = lab.phone.allowed_host().unwrap();
    let phone = browser();
    let (code, cookie) = redeem(&phone, &old_base, &token).await;
    assert_eq!(code, 204);
    let cookie = cookie.unwrap();

    // A new address: rebound on it, same token, old sessions ended.
    lab.sim.set_interface_ip(Ipv4Addr::new(192, 168, 1, 77));
    let moved = step_watcher(&lab.phone, |s| s.url != first.url).await;
    assert_eq!(moved.state, PhoneState::On);
    let (new_base, same_token) = phone_url(&moved);
    assert_ne!(new_base, old_base);
    assert_eq!(same_token, token, "a rebind keeps the token");
    assert_ne!(lab.phone.allowed_host().unwrap(), old_host);

    let c = browser();
    let old = within(
        "an HTTP response",
        c.get(lab.main_url("/")).header("Host", &old_host).send(),
    )
    .await
    .unwrap();
    assert_eq!(old.status(), 403, "the old host is refused");
    assert_eq!(get(&c, format!("{new_base}/")).await.status(), 200);
    let res = within(
        "an HTTP response",
        c.get(format!("{new_base}/api/network"))
            .header("cookie", &cookie)
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(res.status(), 401, "sessions bound to the old host end");
    assert_eq!(redeem(&browser(), &new_base, &token).await.0, 204);

    // Public: paused, listener closed.
    lab.sim.set_category(NetworkCategory::Public);
    let paused = step_watcher(&lab.phone, |s| s.state == PhoneState::Paused).await;
    assert_eq!(paused.reason.as_deref(), Some("public_network"));
    assert_eq!(paused.url, None);
    assert_eq!(lab.phone.allowed_host(), None);
    assert!(within(
        "a connection attempt",
        TcpStream::connect(addr_of(&new_base))
    )
    .await
    .is_err());

    // Private again: On, with a new token.
    lab.sim.set_category(NetworkCategory::Private);
    let resumed = step_watcher(&lab.phone, |s| s.state == PhoneState::On).await;
    let (base, new_token) = phone_url(&resumed);
    assert_ne!(new_token, token);
    assert_eq!(redeem(&browser(), &base, &token).await.0, 401);
    assert_eq!(redeem(&browser(), &base, &new_token).await.0, 204);
}

#[tokio::test]
async fn restore_turns_phone_access_back_on_from_the_saved_setting() {
    let store = memory_store();
    store
        .set_global_setting(PHONE_ACCESS_KEY, "on")
        .await
        .unwrap();
    let lab = lab_with_store("home", store).await;
    assert_eq!(lab.phone.status().await.state, PhoneState::Off);

    lab.phone.restore().await;
    let status = lab.phone.status().await;
    assert_eq!(status.state, PhoneState::On);
    let (base, token) = phone_url(&status);
    assert_eq!(get(&browser(), format!("{base}/")).await.status(), 200);
    assert_eq!(redeem(&browser(), &base, &token).await.0, 204);
    lab.phone.stop().await;
    assert_eq!(
        lab.setting().await.as_deref(),
        Some("on"),
        "quitting keeps the setting"
    );

    // Saved "off" (or nothing saved) stays off.
    for saved in [Some("off"), None] {
        let store = memory_store();
        if let Some(value) = saved {
            store
                .set_global_setting(PHONE_ACCESS_KEY, value)
                .await
                .unwrap();
        }
        let lab = lab_with_store("home", store).await;
        lab.phone.restore().await;
        assert_eq!(lab.phone.status().await.state, PhoneState::Off);
        assert_eq!(lab.phone.allowed_host(), None);
    }

    // Saved "on" on a Public network restores as paused.
    let store = memory_store();
    store
        .set_global_setting(PHONE_ACCESS_KEY, "on")
        .await
        .unwrap();
    let cafe = lab_with_store("cafe", store).await;
    cafe.phone.restore().await;
    let status = cafe.phone.status().await;
    assert_eq!(status.state, PhoneState::Paused);
    assert_eq!(status.reason.as_deref(), Some("public_network"));
}

#[tokio::test]
async fn status_reports_firewall_verdict_and_demo_flag() {
    let lab = lab("home").await;
    let off = lab.phone.status().await;
    assert_eq!(
        off,
        PhoneStatus {
            state: PhoneState::Off,
            reason: None,
            url: None,
            firewall: None,
            category: None,
            demo: true,
        }
    );

    let on = lab.phone.enable().await.unwrap();
    assert_eq!(on.state, PhoneState::On);
    assert_eq!(on.reason, None);
    assert_eq!(on.firewall, Some(FirewallVerdict::Allowed));
    assert_eq!(on.category, Some(NetworkCategory::Private));
    assert!(on.demo);
    assert_eq!(lab.phone.status().await, on);
}

#[tokio::test]
async fn enable_before_attach_is_unavailable() {
    let sim = Arc::new(
        SimNetwork::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scenarios/home.json"
        ))
        .unwrap(),
    );
    let store = memory_store();
    let state = AppState::new(
        sim.clone(),
        sim.clone(),
        store.clone(),
        ScanConfig::default(),
        DiagnoseConfig::default(),
        AccessPolicy::for_port(1, &[]),
    );
    let phone = PhoneAccess::new(
        sim,
        store.clone(),
        Arc::clone(&state.auth),
        Arc::clone(&state.bus),
        Arc::new(LoopbackLanBinder),
        PathBuf::from("netlantern.exe"),
        1,
        true,
    );
    let err = phone.enable().await.unwrap_err();
    assert_eq!(err.status.as_u16(), 503);
    assert_eq!(err.code, "phone_access_unavailable");
    assert_eq!(phone.status().await.state, PhoneState::Off);
    assert_eq!(state.auth.phone_token(), None);
    assert_eq!(store.global_setting(PHONE_ACCESS_KEY).await.unwrap(), None);
}

#[tokio::test]
async fn not_connected_refuses_enable_with_503() {
    let lab = lab("offline").await;
    let err = lab.phone.enable().await.unwrap_err();
    assert_eq!(err.status.as_u16(), 503);
    assert_eq!(err.code, "phone_access_unavailable");
    assert_eq!(err.message, "Phone access needs a network connection.");
    assert_eq!(lab.phone.status().await.state, PhoneState::Off);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_enable_and_disable_leave_a_consistent_state() {
    let lab = lab("home").await;
    for _ in 0..20 {
        let on = tokio::spawn({
            let phone = Arc::clone(&lab.phone);
            async move { phone.enable().await.is_ok() }
        });
        let off = tokio::spawn({
            let phone = Arc::clone(&lab.phone);
            async move { phone.disable().await }
        });
        let (on, off) = (on.await.unwrap(), off.await.unwrap());
        assert!(on);
        assert_eq!(off.state, PhoneState::Off);
        let status = lab.phone.status().await;
        let token = lab.state.auth.phone_token();
        match status.state {
            PhoneState::Off => {
                assert_eq!(token, None, "a token outlived the turn-off");
                assert_eq!(lab.phone.allowed_host(), None);
            }
            PhoneState::On => {
                assert!(token.is_some());
                assert!(lab.phone.allowed_host().is_some());
            }
            PhoneState::Paused => panic!("paused on a Private network"),
        }
        lab.phone.disable().await;
    }
}
