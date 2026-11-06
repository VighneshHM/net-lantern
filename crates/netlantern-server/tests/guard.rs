//! Security layer: launch tokens, sessions, the Host/Origin/session guard, and
//! the structured error body. Requests are driven straight through a tiny
//! router with `tower::ServiceExt::oneshot`, so no sockets are involved.

use std::collections::HashSet;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::{routing, Router};
use netlantern_server::{guard, AccessPolicy, ApiError, Auth, AuthLimits, GuardState, RefusalLog};
use serde_json::Value;
use tower::ServiceExt;

const PORT: u16 = 47800;

// ---------------------------------------------------------------- helpers

/// A request under construction; headers are only set when asked for, so tests
/// can leave `Host` or `Origin` out entirely.
struct Req {
    method: Method,
    path: String,
    headers: Vec<(&'static str, String)>,
}

fn get(path: &str) -> Req {
    Req::new(Method::GET, path)
}

fn post(path: &str) -> Req {
    Req::new(Method::POST, path)
}

impl Req {
    fn new(method: Method, path: &str) -> Self {
        Self {
            method,
            path: path.to_string(),
            headers: Vec::new(),
        }
    }

    fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }

    fn host(self, value: &str) -> Self {
        self.header("host", value)
    }

    fn origin(self, value: &str) -> Self {
        self.header("origin", value)
    }

    fn cookie(self, session_id: &str) -> Self {
        self.header("cookie", &format!("nl_session={session_id}"))
    }

    fn build(self) -> Request<Body> {
        let mut builder = Request::builder().method(self.method).uri(self.path);
        for (name, value) in self.headers {
            builder = builder.header(name, value);
        }
        builder.body(Body::empty()).unwrap()
    }
}

/// A guarded router plus the `Auth` behind it.
struct Fixture {
    auth: Arc<Auth>,
    app: Router,
}

impl Fixture {
    fn new() -> Self {
        Self::with_policy(AccessPolicy::for_port(PORT, &[]))
    }

    fn with_policy(policy: AccessPolicy) -> Self {
        let auth = Arc::new(Auth::new());
        let state = GuardState {
            auth: auth.clone(),
            policy: Arc::new(policy),
            refusals: Arc::default(),
        };
        let app = Router::new()
            .route("/", routing::get(|| async { "index" }))
            .route("/api/ping", routing::get(|| async { "pong" }))
            .route(
                "/api/session",
                routing::post(|| async { StatusCode::NO_CONTENT }).get(|| async { "peek" }),
            )
            .layer(from_fn_with_state(state, guard));
        Self { auth, app }
    }

    /// A fresh, valid session id.
    fn session(&self) -> String {
        let token = self.auth.issue_launch_token();
        self.auth.redeem(&token).expect("fresh token redeems")
    }

    async fn send(&self, req: Req) -> Response {
        self.app.clone().oneshot(req.build()).await.unwrap()
    }
}

async fn json_body(res: Response) -> Value {
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).expect("body is JSON")
}

/// Asserts the status, the API-version header, and the structured error body.
/// Returns the parsed body for further checks.
async fn assert_status_code(res: Response, status: u16, code: &str) -> Value {
    assert_eq!(res.status().as_u16(), status);
    assert_eq!(res.headers()["x-netlantern-api-version"], "1");
    let body = json_body(res).await;
    assert_eq!(body["code"], code);
    assert!(
        body["message"].as_str().is_some_and(|m| !m.is_empty()),
        "message must be a non-empty string: {body}"
    );
    body
}

fn is_lower_hex_64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ------------------------------------------------------------------- Auth

#[test]
fn launch_token_is_64_lowercase_hex_and_redeems_exactly_once() {
    let auth = Auth::new();
    let t = auth.issue_launch_token();
    assert!(is_lower_hex_64(&t), "token: 64 lowercase hex chars");

    let sid = auth.redeem(&t);
    assert!(sid.is_some());
    assert!(is_lower_hex_64(&sid.unwrap()), "session id: 64 hex chars");
    assert!(auth.redeem(&t).is_none(), "tokens are one-time");

    assert!(auth.redeem("nope").is_none());
    assert!(auth.redeem("").is_none());
}

#[test]
fn issued_values_are_random_and_never_interchangeable() {
    let auth = Auth::new();
    let t1 = auth.issue_launch_token();
    let t2 = auth.issue_launch_token();
    assert_ne!(t1, t2);

    let s1 = auth.redeem(&t1).unwrap();
    let s2 = auth.redeem(&t2).unwrap();
    assert_ne!(s1, s2);

    assert!(auth.is_session(&s1));
    assert!(auth.is_session(&s2));
    assert!(!auth.is_session("nope"));
    assert!(!auth.is_session(""));
    // A launch token is not a session, and a session id is not a launch token.
    let t3 = auth.issue_launch_token();
    assert!(!auth.is_session(&t3));
    assert!(auth.redeem(&s1).is_none());
    // An unredeemed token does not become a session by being tried as one.
    assert!(auth.redeem(&t3).is_some());
}

#[test]
fn a_token_redeems_once_even_under_concurrency() {
    let auth = Arc::new(Auth::new());
    let token = auth.issue_launch_token();

    let handles: Vec<_> = (0..16)
        .map(|_| {
            let auth = auth.clone();
            let token = token.clone();
            std::thread::spawn(move || auth.redeem(&token))
        })
        .collect();
    let winners: Vec<String> = handles
        .into_iter()
        .filter_map(|h| h.join().unwrap())
        .collect();

    assert_eq!(winners.len(), 1, "exactly one redeemer wins");
    assert!(auth.is_session(&winners[0]));
    assert_eq!(
        winners.iter().collect::<HashSet<_>>().len(),
        1,
        "one session was created"
    );
}

#[test]
fn auth_debug_output_never_contains_secrets() {
    let auth = Auth::new();
    let pending = auth.issue_launch_token();
    let redeemed = auth.issue_launch_token();
    let sid = auth.redeem(&redeemed).unwrap();

    let printed = format!("{auth:?} {auth:#?}");
    for secret in [&pending, &redeemed, &sid] {
        assert!(
            !printed.contains(secret.as_str()),
            "secret leaked: {printed}"
        );
    }
}

// ----------------------------------------------------------- AccessPolicy

#[test]
fn policy_for_port_lists_loopback_names_and_dev_origins() {
    let p = AccessPolicy::for_port(47801, &[]);
    assert_eq!(p.allowed_hosts, ["127.0.0.1:47801", "localhost:47801"]);
    assert_eq!(
        p.allowed_origins,
        ["http://127.0.0.1:47801", "http://localhost:47801"]
    );

    let p = AccessPolicy::for_port(47801, &["http://localhost:5173".to_string()]);
    assert_eq!(p.allowed_hosts.len(), 2, "dev origins never widen Host");
    assert_eq!(
        p.allowed_origins,
        [
            "http://127.0.0.1:47801",
            "http://localhost:47801",
            "http://localhost:5173"
        ]
    );
}

#[test]
fn dev_origins_are_normalized_to_what_browsers_send() {
    let p = AccessPolicy::for_port(
        47801,
        &[
            "http://localhost:5173/".to_string(),
            " HTTP://LocalHost:5174 ".to_string(),
        ],
    );
    assert_eq!(
        p.allowed_origins[2..],
        ["http://localhost:5173", "http://localhost:5174"]
    );
}

#[tokio::test]
async fn a_dev_origin_given_with_a_trailing_slash_is_accepted() {
    let f = Fixture::with_policy(AccessPolicy::for_port(
        PORT,
        &["http://localhost:5173/".to_string()],
    ));
    let sid = f.session();
    let res = f
        .send(
            get("/api/ping")
                .host("127.0.0.1:47800")
                .origin("http://localhost:5173")
                .cookie(&sid),
        )
        .await;
    assert_eq!(res.status(), 200);
}

// ------------------------------------------------------------ guard: Host

#[tokio::test]
async fn foreign_host_is_rejected_as_dns_rebinding() {
    let f = Fixture::new();
    let body = assert_status_code(
        f.send(get("/").host("evil.example:47800")).await,
        403,
        "forbidden_host",
    )
    .await;
    assert!(body["hint"].is_null());
}

#[tokio::test]
async fn missing_host_is_rejected() {
    let f = Fixture::new();
    assert_status_code(f.send(get("/")).await, 403, "forbidden_host").await;
}

#[tokio::test]
async fn host_must_match_exactly_including_the_port() {
    let f = Fixture::new();
    for host in [
        "127.0.0.1",                    // no port
        "127.0.0.1:47801",              // other port
        "127.0.0.1:47800.evil.example", // suffix
        "evil.example.127.0.0.1:47800", // prefix
        "[::1]:47800",                  // not in the list
        "0.0.0.0:47800",
        "",
    ] {
        assert_status_code(f.send(get("/").host(host)).await, 403, "forbidden_host").await;
    }
}

#[tokio::test]
async fn host_comparison_ignores_ascii_case() {
    let f = Fixture::new();
    for host in ["LOCALHOST:47800", "LocalHost:47800", "127.0.0.1:47800"] {
        assert_eq!(f.send(get("/").host(host)).await.status(), 200, "{host}");
    }
}

#[tokio::test]
async fn host_is_checked_even_for_the_session_exchange() {
    let f = Fixture::new();
    assert_status_code(
        f.send(post("/api/session").host("evil.example:47800"))
            .await,
        403,
        "forbidden_host",
    )
    .await;
}

// ---------------------------------------------------------- guard: Origin

#[tokio::test]
async fn foreign_origin_is_rejected() {
    let f = Fixture::new();
    let body = assert_status_code(
        f.send(
            get("/")
                .host("127.0.0.1:47800")
                .origin("http://evil.example"),
        )
        .await,
        403,
        "forbidden_origin",
    )
    .await;
    assert!(body["hint"].is_null());
}

#[tokio::test]
async fn origin_null_is_rejected() {
    let f = Fixture::new();
    assert_status_code(
        f.send(get("/").host("127.0.0.1:47800").origin("null"))
            .await,
        403,
        "forbidden_origin",
    )
    .await;
}

#[tokio::test]
async fn origin_must_match_exactly() {
    let f = Fixture::new();
    for origin in [
        "http://localhost:47800.evil.example", // suffix
        "http://evil.example/http://localhost:47800",
        "http://localhost:47801",  // other port
        "https://localhost:47800", // other scheme
        "http://localhost:47800/", // trailing slash is not an origin
        "http://LOCALHOST:47800",  // origins compare exactly
        "http://localhost",
        "",
    ] {
        assert_status_code(
            f.send(get("/").host("localhost:47800").origin(origin))
                .await,
            403,
            "forbidden_origin",
        )
        .await;
    }
}

#[tokio::test]
async fn same_origin_and_absent_origin_pass() {
    let f = Fixture::new();
    for origin in ["http://127.0.0.1:47800", "http://localhost:47800"] {
        let res = f
            .send(get("/").host("127.0.0.1:47800").origin(origin))
            .await;
        assert_eq!(res.status(), 200, "{origin}");
    }
    assert_eq!(
        f.send(get("/").host("127.0.0.1:47800")).await.status(),
        200,
        "no Origin header (same-origin GET, curl, native client)"
    );
}

#[tokio::test]
async fn dev_origin_is_allowed_only_when_configured() {
    let plain = Fixture::new();
    let sid = plain.session();
    let req = || {
        get("/api/ping")
            .host("127.0.0.1:47800")
            .origin("http://localhost:5173")
            .cookie(&sid)
    };
    assert_status_code(plain.send(req()).await, 403, "forbidden_origin").await;

    let dev_policy = AccessPolicy::for_port(PORT, &["http://localhost:5173".to_string()]);
    let dev = Fixture::with_policy(dev_policy);
    let sid = dev.session();
    let res = dev
        .send(
            get("/api/ping")
                .host("127.0.0.1:47800")
                .origin("http://localhost:5173")
                .cookie(&sid),
        )
        .await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["x-netlantern-api-version"], "1");

    // The dev origin does not make a foreign Host acceptable.
    assert_status_code(
        dev.send(
            get("/api/ping")
                .host("evil.example:47800")
                .origin("http://localhost:5173")
                .cookie(&sid),
        )
        .await,
        403,
        "forbidden_host",
    )
    .await;
}

// -------------------------------------------------------- guard: session

#[tokio::test]
async fn api_without_session_is_unauthorized() {
    let f = Fixture::new();
    let res = f.send(get("/api/ping").host("127.0.0.1:47800")).await;
    let body = assert_status_code(res, 401, "unauthorized").await;
    // `hint` is always present in the body, as an explicit null.
    assert!(body.as_object().unwrap().contains_key("hint"));
    assert!(body["hint"].is_null());
    assert!(!body.as_object().unwrap().contains_key("scan_id"));
}

#[tokio::test]
async fn api_with_session_localhost_and_same_origin_passes() {
    let f = Fixture::new();
    let sid = f.session();
    let r = f
        .send(
            get("/api/ping")
                .host("localhost:47800")
                .origin("http://localhost:47800")
                .cookie(&sid),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["x-netlantern-api-version"], "1");
}

#[tokio::test]
async fn session_cookie_is_found_among_other_cookies_and_matched_exactly() {
    let f = Fixture::new();
    let sid = f.session();
    let host = "127.0.0.1:47800";

    for cookie in [
        format!("theme=dark; nl_session={sid}; other=1"),
        format!("nl_session={sid}; other=1"),
        format!("other=1; nl_session={sid}"),
    ] {
        let res = f
            .send(get("/api/ping").host(host).header("cookie", &cookie))
            .await;
        assert_eq!(res.status(), 200, "{cookie}");
    }

    let launch_token = f.auth.issue_launch_token();
    for cookie in [
        "nl_session=".to_string(),
        "nl_session=nope".to_string(),
        format!("xnl_session={sid}"),
        format!("nl_session_x={sid}"),
        format!("other={sid}"),
        format!("nl_session={launch_token}"), // a launch token is not a session
        format!("nl_session={}", sid.to_uppercase()),
    ] {
        let res = f
            .send(get("/api/ping").host(host).header("cookie", &cookie))
            .await;
        assert_status_code(res, 401, "unauthorized").await;
    }
}

#[tokio::test]
async fn static_paths_need_no_session() {
    let f = Fixture::new();
    let res = f.send(get("/").host("127.0.0.1:47800")).await;
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["x-netlantern-api-version"], "1");
}

#[tokio::test]
async fn session_exchange_is_not_blocked_by_the_session_check() {
    let f = Fixture::new();
    let host = "127.0.0.1:47800";
    // POST /api/session is how a session is obtained, so it is exempt ...
    let res = f.send(post("/api/session").host(host)).await;
    assert_eq!(res.status(), 204);
    assert_eq!(res.headers()["x-netlantern-api-version"], "1");
    // ... but only for POST and only for that exact path.
    assert_status_code(
        f.send(get("/api/session").host(host)).await,
        401,
        "unauthorized",
    )
    .await;
    assert_status_code(
        f.send(post("/api/session/").host(host)).await,
        401,
        "unauthorized",
    )
    .await;
    assert_status_code(
        f.send(post("/api/session/extra").host(host)).await,
        401,
        "unauthorized",
    )
    .await;
}

#[tokio::test]
async fn the_api_prefix_is_a_path_segment_not_a_string_prefix() {
    let f = Fixture::new();
    let host = "127.0.0.1:47800";
    // `/api` itself and anything below it is API, even where no route exists:
    // the guard answers before routing, so unknown routes leak nothing.
    assert_status_code(f.send(get("/api").host(host)).await, 401, "unauthorized").await;
    assert_status_code(
        f.send(get("/api/nope").host(host)).await,
        401,
        "unauthorized",
    )
    .await;
    // `/apix` is not under `/api/`: it falls through to the router's 404.
    assert_eq!(f.send(get("/apix").host(host)).await.status(), 404);
}

#[tokio::test]
async fn session_check_uses_the_original_path_when_mounted_in_a_nested_router() {
    // `Router::nest` strips the prefix before the inner router (and any layer
    // on it) sees the request. The guard must still treat it as `/api/...`.
    let auth = Arc::new(Auth::new());
    let state = GuardState {
        auth: auth.clone(),
        policy: Arc::new(AccessPolicy::for_port(PORT, &[])),
        refusals: Arc::default(),
    };
    let inner = Router::new()
        .route("/ping", routing::get(|| async { "nested pong" }))
        .route(
            "/session",
            routing::post(|| async { StatusCode::NO_CONTENT }),
        )
        .layer(from_fn_with_state(state, guard));
    let app = Router::new().nest("/api", inner);
    let host = "127.0.0.1:47800";

    let res = app
        .clone()
        .oneshot(get("/api/ping").host(host).build())
        .await
        .unwrap();
    assert_status_code(res, 401, "unauthorized").await;

    let sid = auth.redeem(&auth.issue_launch_token()).unwrap();
    let res = app
        .clone()
        .oneshot(get("/api/ping").host(host).cookie(&sid).build())
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["x-netlantern-api-version"], "1");

    // The session exchange stays exempt, judged by its original path too.
    let res = app
        .oneshot(post("/api/session").host(host).build())
        .await
        .unwrap();
    assert_eq!(res.status(), 204);
}

// ------------------------------------------------------------ guard order

#[tokio::test]
async fn checks_run_host_then_origin_then_session() {
    let f = Fixture::new();
    // Bad Host wins over bad Origin and missing session.
    assert_status_code(
        f.send(
            get("/api/ping")
                .host("evil.example:47800")
                .origin("http://evil.example"),
        )
        .await,
        403,
        "forbidden_host",
    )
    .await;
    // Bad Origin wins over a missing session.
    assert_status_code(
        f.send(
            get("/api/ping")
                .host("127.0.0.1:47800")
                .origin("http://evil.example"),
        )
        .await,
        403,
        "forbidden_origin",
    )
    .await;
    // Only then is the session consulted.
    assert_status_code(
        f.send(get("/api/ping").host("127.0.0.1:47800")).await,
        401,
        "unauthorized",
    )
    .await;
}

// -------------------------------------------------------------- ApiError

#[tokio::test]
async fn constructors_use_the_documented_status_and_code() {
    let cases = [
        (ApiError::unauthorized(), 401, "unauthorized"),
        (ApiError::forbidden_host(), 403, "forbidden_host"),
        (ApiError::forbidden_origin(), 403, "forbidden_origin"),
        (ApiError::not_connected(), 503, "not_connected"),
        (ApiError::scan_in_progress(3), 409, "scan_in_progress"),
        (
            ApiError::unsupported("Deep scan arrives later."),
            501,
            "unsupported",
        ),
        (ApiError::not_found(), 404, "not_found"),
        (
            ApiError::bad_request("mode must be quick"),
            400,
            "bad_request",
        ),
    ];
    for (err, status, code) in cases {
        assert_eq!(err.status.as_u16(), status);
        assert_eq!(err.code, code);
        let res = err.into_response();
        assert_eq!(res.status().as_u16(), status);
        assert_eq!(json_body(res).await["code"], code);
    }
}

#[tokio::test]
async fn error_body_is_code_message_hint_with_hint_null_when_absent() {
    let res = ApiError::not_found().into_response();
    assert_eq!(res.headers()["content-type"], "application/json");
    let body = json_body(res).await;
    assert_eq!(body["code"], "not_found");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    let obj = body.as_object().unwrap();
    assert!(obj.contains_key("hint") && body["hint"].is_null());
    assert!(
        !obj.contains_key("scan_id"),
        "scan_id is omitted unless set"
    );
    assert_eq!(obj.len(), 3);
}

#[tokio::test]
async fn not_connected_carries_its_hint() {
    let body = json_body(ApiError::not_connected().into_response()).await;
    assert_eq!(
        body["hint"],
        "Connect to a Wi-Fi or Ethernet network, then try again."
    );
}

#[tokio::test]
async fn scan_in_progress_includes_scan_id_and_nothing_else_does() {
    let res = ApiError::scan_in_progress(7).into_response();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("\"scan_id\":7"), "{text}");

    for err in [
        ApiError::unauthorized(),
        ApiError::forbidden_host(),
        ApiError::forbidden_origin(),
        ApiError::not_connected(),
        ApiError::unsupported("x"),
        ApiError::not_found(),
        ApiError::bad_request("x"),
    ] {
        let body = json_body(err.into_response()).await;
        assert!(!body.as_object().unwrap().contains_key("scan_id"), "{body}");
    }
}

#[tokio::test]
async fn handler_errors_pass_through_the_guard_with_the_version_header() {
    let auth = Arc::new(Auth::new());
    let state = GuardState {
        auth: auth.clone(),
        policy: Arc::new(AccessPolicy::for_port(PORT, &[])),
        refusals: Arc::default(),
    };
    let app = Router::new()
        .route(
            "/api/busy",
            routing::get(|| async { ApiError::scan_in_progress(7) }),
        )
        .layer(from_fn_with_state(state, guard));
    let token = auth.issue_launch_token();
    let sid = auth.redeem(&token).unwrap();

    let req = get("/api/busy")
        .host("127.0.0.1:47800")
        .cookie(&sid)
        .build();
    let res = app.oneshot(req).await.unwrap();
    let body = assert_status_code(res, 409, "scan_in_progress").await;
    assert_eq!(body["scan_id"], 7);
}

// ---------------------------------------------------- bounds and refusal log

fn ms(n: u64) -> std::time::Duration {
    std::time::Duration::from_millis(n)
}

#[tokio::test]
async fn an_expired_launch_token_does_not_redeem() {
    let auth = Auth::with_limits(AuthLimits {
        token_ttl: ms(50),
        max_tokens: 2,
        max_sessions: 2,
    });
    let t = auth.issue_launch_token();
    tokio::time::sleep(ms(100)).await;
    assert!(auth.redeem(&t).is_none());
    // And it is consumed: a fresh one still works.
    assert!(auth.redeem(&auth.issue_launch_token()).is_some());
}

#[test]
fn issuing_past_the_token_bound_drops_the_oldest() {
    let auth = Auth::with_limits(AuthLimits {
        token_ttl: ms(60_000),
        max_tokens: 2,
        max_sessions: 2,
    });
    let (a, b, c) = (
        auth.issue_launch_token(),
        auth.issue_launch_token(),
        auth.issue_launch_token(),
    );
    assert!(auth.redeem(&a).is_none());
    assert!(auth.redeem(&b).is_some());
    assert!(auth.redeem(&c).is_some());
}

#[test]
fn creating_past_the_session_bound_drops_the_oldest() {
    let auth = Auth::with_limits(AuthLimits {
        token_ttl: ms(60_000),
        max_tokens: 8,
        max_sessions: 2,
    });
    let session = || auth.redeem(&auth.issue_launch_token()).unwrap();
    let (s1, s2, s3) = (session(), session(), session());
    assert!(!auth.is_session(&s1));
    assert!(auth.is_session(&s2));
    assert!(auth.is_session(&s3));
}

#[test]
fn the_default_limits_are_five_minutes_eight_tokens_thirty_two_sessions() {
    let limits = AuthLimits::default();
    assert_eq!(limits.token_ttl, std::time::Duration::from_secs(300));
    assert_eq!((limits.max_tokens, limits.max_sessions), (8, 32));
}

#[test]
fn the_refusal_log_reports_each_pair_once() {
    let log = RefusalLog::default();
    assert!(log.first_time("host", "evil:1"));
    assert!(!log.first_time("host", "evil:1"));
    assert!(log.first_time("origin", "evil:1"));
}
