//! The request guard. Every request, including WebSocket upgrades, passes
//! three checks in this order, and the first failure answers:
//!
//! 1. **Host** must be one of the loopback names for our port, or the phone
//!    listener's address while phone access is On (defeats DNS rebinding). A
//!    missing Host is rejected.
//! 2. **Origin**, if present, must be one we serve (the phone listener's
//!    origin only while On) or a configured dev origin (defeats cross-site
//!    requests and cross-site WebSocket hijacking). `Origin: null` is
//!    rejected.
//!
//! One guard serves both listeners: the loopback one and the phone one.
//! 3. **Session**: paths under `/api` need a valid `nl_session` cookie, except
//!    `POST /api/session`, where a session is obtained.
//!
//! Every response the guard returns, and every response passing through it,
//! carries `x-netlantern-api-version`.

use std::sync::Arc;

use axum::extract::{OriginalUri, Request, State};
use axum::http::header::GetAll;
use axum::http::header::{COOKIE, HOST, ORIGIN};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use netlantern_core::API_VERSION;

use crate::auth::{Auth, SessionKind};
use crate::error::ApiError;
use crate::phone::PhoneAccess;
use crate::refusals::{truncate, RefusalLog};

/// Name of the session cookie (`HttpOnly; SameSite=Strict; Path=/`).
pub const SESSION_COOKIE: &str = "nl_session";

const API_VERSION_HEADER: HeaderName = HeaderName::from_static("x-netlantern-api-version");

/// Which `Host` and `Origin` values this server answers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPolicy {
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
}

impl AccessPolicy {
    /// Loopback names on `port`, plus any `--dev-origin` values. Dev origins
    /// widen only `Origin`, never `Host`.
    ///
    /// Dev origins are normalized to the form browsers send: surrounding
    /// whitespace and a trailing `/` are dropped and scheme and host are
    /// lowercased, so `http://LocalHost:5173/` matches `http://localhost:5173`.
    pub fn for_port(port: u16, dev_origins: &[String]) -> Self {
        let mut allowed_origins = vec![
            format!("http://127.0.0.1:{port}"),
            format!("http://localhost:{port}"),
        ];
        allowed_origins.extend(dev_origins.iter().map(|origin| normalize_origin(origin)));
        Self {
            allowed_hosts: vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            allowed_origins,
        }
    }
}

/// An origin is `scheme://host[:port]`, all of which compare case-insensitively
/// and none of which is case-significant, so the whole value is lowercased.
fn normalize_origin(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

#[derive(Clone)]
pub struct GuardState {
    pub auth: Arc<Auth>,
    pub policy: Arc<AccessPolicy>,
    pub refusals: Arc<RefusalLog>,
    /// Adds the phone listener's Host and Origin while phone access is On.
    pub phone: Option<Arc<PhoneAccess>>,
}

/// Mount with `axum::middleware::from_fn_with_state(state, guard)` as the
/// outermost layer, so it sits in front of every route and the fallback. The
/// session check judges the original request path, so it also holds if the
/// guard ends up on a router that is later nested.
pub async fn guard(State(state): State<GuardState>, mut request: Request, next: Next) -> Response {
    let mut response = match check(&state, &request) {
        Ok(kind) => {
            // Handlers that must refuse phone sessions read this extension.
            if let Some(kind) = kind {
                request.extensions_mut().insert(kind);
            }
            next.run(request).await
        }
        Err(error) => error.into_response(),
    };
    response
        .headers_mut()
        .insert(API_VERSION_HEADER, HeaderValue::from(API_VERSION));
    response
}

/// Passes the request, with the kind of its session when it has a valid one.
fn check(state: &GuardState, request: &Request) -> Result<Option<SessionKind>, ApiError> {
    let headers = request.headers();
    let phone = state.phone.as_deref();
    let phone_host = phone.and_then(PhoneAccess::allowed_host);
    if !host_allowed(headers, &state.policy.allowed_hosts, phone_host.as_deref()) {
        log_refusal(&state.refusals, "host", headers.get_all(HOST));
        return Err(ApiError::forbidden_host());
    }
    let phone_origin = phone.and_then(PhoneAccess::allowed_origin);
    if !origin_allowed(
        headers,
        &state.policy.allowed_origins,
        phone_origin.as_deref(),
    ) {
        log_refusal(&state.refusals, "origin", headers.get_all(ORIGIN));
        return Err(ApiError::forbidden_origin());
    }
    let kind = valid_session(&state.auth, headers).and_then(|id| state.auth.session_kind(id));
    if needs_session(request.method(), original_path(request)) && kind.is_none() {
        return Err(ApiError::unauthorized());
    }
    Ok(kind)
}

/// Warns about a refused header value, the first time only (spec section 10).
fn log_refusal(log: &RefusalLog, kind: &'static str, values: GetAll<'_, HeaderValue>) {
    let value = values
        .iter()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .collect::<Vec<_>>()
        .join(", ");
    if log.first_time(kind, &value) {
        tracing::warn!(kind, value = %truncate(&value, 100), "refused request");
    }
}

/// Exactly one or more Host headers, every one allowed (ASCII case-insensitive,
/// exact otherwise). No Host header at all is not allowed. `phone` is the
/// phone listener's host, present only while phone access is On.
fn host_allowed(headers: &HeaderMap, allowed: &[String], phone: Option<&str>) -> bool {
    let mut hosts = headers.get_all(HOST).iter().peekable();
    hosts.peek().is_some()
        && hosts.all(|value| {
            value.to_str().is_ok_and(|host| {
                allowed.iter().any(|a| a.eq_ignore_ascii_case(host))
                    || phone.is_some_and(|p| p.eq_ignore_ascii_case(host))
            })
        })
}

/// No Origin header is fine; otherwise every one must match exactly. `phone`
/// is the phone listener's origin, present only while phone access is On.
fn origin_allowed(headers: &HeaderMap, allowed: &[String], phone: Option<&str>) -> bool {
    headers.get_all(ORIGIN).iter().all(|value| {
        value
            .to_str()
            .is_ok_and(|origin| allowed.iter().any(|a| a == origin) || phone == Some(origin))
    })
}

fn needs_session(method: &Method, path: &str) -> bool {
    let is_api = path == "/api" || path.starts_with("/api/");
    is_api && !(method == Method::POST && path == "/api/session")
}

/// The first `nl_session=<id>` among the `;`-separated cookies whose id is a
/// live session (of either kind).
pub(crate) fn valid_session<'h>(auth: &Auth, headers: &'h HeaderMap) -> Option<&'h str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find(|(name, value)| *name == SESSION_COOKIE && auth.is_session(value))
        .map(|(_, value)| value)
}

/// The path the client asked for. `Router::nest` strips its prefix from
/// `request.uri()` before an inner router (and any layer on it) runs, so the
/// session check must not read that: it would see `/ping` for `/api/ping` and
/// fail open. axum records the untouched URI as `OriginalUri`.
fn original_path(request: &Request) -> &str {
    request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().path(), |original| original.0.path())
}
