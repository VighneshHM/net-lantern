//! The REST API and the router that puts it, the event stream, and the UI
//! fallback behind the guard.
//!
//! Every error is an [`ApiError`] JSON body: extractor rejections (bad JSON, a
//! non-numeric id) are mapped rather than passed through as axum's plain-text
//! responses.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, SET_COOKIE};
use axum::http::Method;
use axum::http::StatusCode;
use axum::middleware::{from_fn_with_state, Next};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{get, patch, post, put};
use axum::Extension;
use axum::{Json, Router};
use netlantern_core::{
    AckOutcome, Alert, AlertFilter, AlertId, Check, CoreError, Device, DeviceId, DevicePatch,
    DiagnoseStart, DiagnosisId, DiagnosisRun, EndReason, EventKind, Incident, IncidentId, Metric,
    MetricSeries, MonitorStatus, NearbyScan, NetworkId, ReplacedRouter, Resolution, RuleSettings,
    ScanId, ScanMode, ScanSnapshot, StartOutcome, StoreError, Target, UpdateStatus, WifiStatus,
};
use netlantern_platform::{Interface, NetworkCategory};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::assets;
use crate::auth::SessionKind;
use crate::error::ApiError;
use crate::guard::{guard, GuardState, SESSION_COOKIE};
use crate::phone::{PhoneAccess, PhoneStatus};
use crate::state::{AppState, HostInfo, HostKind};
use crate::ws;

/// The whole HTTP surface. The guard is the outermost layer, added after
/// every route and both fallbacks, so nothing is reachable around it.
///
/// What no route matches goes to the UI (see [`assets`]), except under `/api`,
/// which stays a JSON 404.
pub fn router(state: AppState) -> Router {
    let guard_state = GuardState {
        auth: Arc::clone(&state.auth),
        policy: Arc::clone(&state.policy),
        refusals: Arc::clone(&state.refusals),
        phone: state.phone.clone(),
    };
    Router::new()
        .route("/api/session", post(create_session))
        .route("/api/network", get(network))
        .route("/api/network/consent", put(set_consent))
        .route("/api/network/replaced-router", post(decide_replaced_router))
        .route("/api/scans", post(start_scan))
        .route("/api/scans/latest", get(latest_scan))
        .route("/api/scans/{id}", get(scan))
        .route("/api/scans/{id}/cancel", post(cancel_scan))
        .route("/api/devices", get(devices))
        .route("/api/devices/{id}", get(device).merge(patch(patch_device)))
        .route("/api/diagnose", post(start_diagnosis))
        .route("/api/diagnose/latest", get(latest_diagnosis))
        .route("/api/diagnose/{id}", get(diagnosis))
        .route("/api/wifi", get(wifi_status))
        .route("/api/wifi/scan", post(wifi_scan))
        .route("/api/monitor", get(monitor_status))
        .route("/api/metrics", get(metrics))
        .route("/api/alerts", get(alerts))
        .route("/api/alerts/{id}/ack", post(ack_alert))
        .route("/api/incidents/{id}", get(incident))
        .route("/api/rules", get(rules).merge(put(set_rules)))
        .route(
            "/api/phone-access",
            get(phone_status).merge(post(set_phone_access)),
        )
        .route("/api/update", get(update_status).merge(put(set_update)))
        .route("/api/debug-info", get(debug_info))
        .route("/api/data/reset", post(reset_data))
        .route("/api/host", get(host_info))
        .route("/api/host/autostart", put(set_autostart))
        .route("/api/events", get(ws::events))
        // Applies only to the routes above, so it stays after the last one.
        .method_not_allowed_fallback(method_not_allowed)
        .fallback(assets::fallback)
        .layer(from_fn_with_state(Arc::clone(&state.reset), wait_for_reset))
        .with_state(state)
        .layer(from_fn_with_state(guard_state, guard))
}

/// `POST /api/session` body. No `Debug`, so the token cannot end up in a log.
#[derive(Deserialize)]
struct SessionRequest {
    token: String,
}

/// Exchanges a one-time launch token or the phone token for the session
/// cookie. Both kinds of session get the same cookie.
async fn create_session(
    State(state): State<AppState>,
    body: Result<Json<SessionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::bad_request(r#"Expected a JSON body {"token": "..."}."#))?;
    let (session, _kind) = state
        .auth
        .redeem(&request.token)
        .ok_or_else(ApiError::unauthorized)?;
    let cookie = format!("{SESSION_COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/");
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]))
}

/// The interface plus the identified network's id and deep-scan consent, the
/// Windows network category, the Wi-Fi name and any new-router question.
#[derive(Serialize)]
struct NetworkInfo {
    #[serde(flatten)]
    interface: Interface,
    network_id: Option<NetworkId>,
    scan_consent: bool,
    /// None when Windows' category cannot be read.
    category: Option<NetworkCategory>,
    /// None without Wi-Fi, without Location permission, or when hidden.
    ssid: Option<String>,
    replaced_router: Option<ReplacedRouter>,
}

async fn network(State(state): State<AppState>) -> Result<Json<NetworkInfo>, ApiError> {
    let interface = state
        .platform
        .active_interface()
        .await
        .map_err(CoreError::from)?;
    // The interface is still worth showing when the network cannot be
    // identified or its consent read: that only hides the network.
    let (network_id, scan_consent) = match state.scans.scan_consent().await {
        Ok(Some((id, granted))) => (Some(id), granted),
        Ok(None) => (None, false),
        Err(error) => {
            tracing::warn!(%error, "could not read this network's scan consent");
            (None, false)
        }
    };
    // Best effort like the rest: none of these may fail the route.
    let category = match state.platform.network_category().await {
        Ok(category) => Some(category),
        Err(error) => {
            tracing::warn!(%error, "could not read the network category");
            None
        }
    };
    let ssid = match state.platform.wifi_current().await {
        Ok(Some(connection)) if !connection.ssid.is_empty() => Some(connection.ssid),
        _ => None,
    };
    let replaced_router = match state.scans.replaced_router().await {
        Ok(pending) => pending,
        Err(error) => {
            tracing::warn!(%error, "could not read whether the router was replaced");
            None
        }
    };
    Ok(Json(NetworkInfo {
        interface,
        network_id,
        scan_consent,
        category,
        ssid,
        replaced_router,
    }))
}

#[derive(Deserialize)]
struct DecisionRequest {
    keep: bool,
}

#[derive(Serialize)]
struct Decided {
    network_id: NetworkId,
}

/// Answers the new-router question: keep the known network or start a new
/// one. 404 `no_decision_pending` when nothing is waiting for an answer.
async fn decide_replaced_router(
    State(state): State<AppState>,
    body: Result<Json<DecisionRequest>, JsonRejection>,
) -> Result<Json<Decided>, ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::bad_request(r#"Expected a JSON body {"keep": true}."#))?;
    let network_id = state.scans.decide_replaced_router(request.keep).await?;
    Ok(Json(Decided { network_id }))
}

#[derive(Deserialize)]
struct ConsentRequest {
    granted: bool,
}

#[derive(Serialize)]
struct ConsentSet {
    network_id: NetworkId,
    scan_consent: bool,
}

async fn set_consent(
    State(state): State<AppState>,
    body: Result<Json<ConsentRequest>, JsonRejection>,
) -> Result<Json<ConsentSet>, ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::bad_request(r#"Expected a JSON body {"granted": true}."#))?;
    let network_id = state.scans.set_scan_consent(request.granted).await?;
    Ok(Json(ConsentSet {
        network_id,
        scan_consent: request.granted,
    }))
}

#[derive(Deserialize)]
struct ScanRequest {
    mode: ScanMode,
}

#[derive(Serialize)]
struct ScanStarted {
    scan_id: ScanId,
}

async fn start_scan(
    State(state): State<AppState>,
    body: Result<Json<ScanRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(request) = body.map_err(|_| {
        ApiError::bad_request(r#"Expected a JSON body {"mode": "quick"} or {"mode": "deep"}."#)
    })?;
    match state.scans.start(request.mode).await? {
        StartOutcome::Started(scan_id) => Ok((StatusCode::ACCEPTED, Json(ScanStarted { scan_id }))),
        StartOutcome::AlreadyRunning(ScanId(id)) => Err(ApiError::scan_in_progress(id)),
    }
}

async fn scan(
    State(state): State<AppState>,
    id: Result<Path<u64>, PathRejection>,
) -> Result<Json<ScanSnapshot>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    state
        .scans
        .snapshot(ScanId(id))
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

/// Stops the running scan. 202 with its snapshot (it ends `cancelled` shortly);
/// 409 when it already finished, 404 when there is no such scan.
async fn cancel_scan(
    State(state): State<AppState>,
    id: Result<Path<u64>, PathRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    let id = ScanId(id);
    let cancelled = state.scans.cancel(id);
    let snapshot = state
        .scans
        .snapshot(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if !cancelled {
        return Err(ApiError::scan_not_running());
    }
    Ok((StatusCode::ACCEPTED, Json(snapshot)))
}

/// The devices of the network this laptop is on; none if it is not a known one.
async fn devices(State(state): State<AppState>) -> Result<Json<Vec<Device>>, ApiError> {
    let Some(network) = state.scans.current_network().await? else {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(state.store.list_devices(network).await?))
}

async fn device(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Device>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    let network = state
        .scans
        .current_network()
        .await?
        .ok_or_else(ApiError::not_found)?;
    state
        .store
        .get_device(network, DeviceId(id))
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

const MAX_NAME_CHARS: usize = 64;

/// `PATCH /api/devices/{id}` body. `name` tells absent (leave alone) from
/// `null` (clear the name, falling back to the discovered one).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DevicePatchRequest {
    #[serde(default, deserialize_with = "present")]
    name: Option<Option<String>>,
    starred: Option<bool>,
    is_new: Option<bool>,
    checks: Option<Vec<Check>>,
}

/// Wraps whatever is present, `null` included, in `Some`; `#[serde(default)]`
/// leaves an absent field `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

async fn patch_device(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
    body: Result<Json<DevicePatchRequest>, JsonRejection>,
) -> Result<Json<Device>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    let Json(request) = body.map_err(|_| {
        ApiError::bad_request(
            r#"Expected a JSON body with any of "name" (text or null), "starred" (true or false), "is_new" (false), "checks" (a list)."#,
        )
    })?;
    if let Some(Some(name)) = &request.name {
        if name.trim().chars().count() > MAX_NAME_CHARS {
            return Err(ApiError::bad_request(format!(
                "Names can be at most {MAX_NAME_CHARS} characters."
            )));
        }
    }
    if request.is_new == Some(true) {
        return Err(ApiError::bad_request(
            r#"A device can only be marked as seen: "is_new" must be false."#,
        ));
    }
    if let Some(checks) = &request.checks {
        validate_checks(checks, state.max_checks)?;
    }
    let network = state
        .scans
        .current_network()
        .await?
        .ok_or_else(ApiError::not_found)?;
    let patch = DevicePatch {
        name: request.name,
        starred: request.starred,
        is_new: request.is_new,
    };
    let id = DeviceId(id);
    let mut device = state.store.update_device(network, id, patch).await?;
    if let (Some(_), Some(checks)) = (&device, request.checks) {
        device = state.store.set_checks(network, id, checks).await?;
    }
    device.map(Json).ok_or_else(ApiError::not_found)
}

const MAX_PATH_CHARS: usize = 200;

fn validate_checks(checks: &[Check], max: usize) -> Result<(), ApiError> {
    if checks.len() > max {
        return Err(ApiError::bad_request(format!(
            "A device can have at most {max} checks."
        )));
    }
    for check in checks {
        let port = match check {
            Check::Tcp { port } | Check::Http { port, .. } => *port,
        };
        if port == 0 {
            return Err(ApiError::bad_request("Check ports must be 1 to 65535."));
        }
        if let Check::Http { path, .. } = check {
            if !path.starts_with('/') {
                return Err(ApiError::bad_request(
                    "An HTTP check path must start with /.",
                ));
            }
            if path.chars().count() > MAX_PATH_CHARS {
                return Err(ApiError::bad_request(format!(
                    "An HTTP check path can be at most {MAX_PATH_CHARS} characters."
                )));
            }
            if path.chars().any(char::is_whitespace) {
                return Err(ApiError::bad_request(
                    "An HTTP check path can't contain spaces.",
                ));
            }
        }
    }
    Ok(())
}

/// The most recent scan, running or finished; 404 if there has never been one.
async fn latest_scan(State(state): State<AppState>) -> Result<Json<ScanSnapshot>, ApiError> {
    state
        .scans
        .latest()
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

#[derive(Serialize)]
struct DiagnosisStarted {
    run_id: DiagnosisId,
    already_running: bool,
}

/// Starts a diagnosis, or hands back the one already running: never a
/// conflict, the UI just follows the id.
async fn start_diagnosis(State(state): State<AppState>) -> impl IntoResponse {
    let (run_id, already_running) = match state.diagnose.start().await {
        DiagnoseStart::Started(id) => (id, false),
        DiagnoseStart::AlreadyRunning(id) => (id, true),
    };
    (
        StatusCode::ACCEPTED,
        Json(DiagnosisStarted {
            run_id,
            already_running,
        }),
    )
}

async fn latest_diagnosis(State(state): State<AppState>) -> Result<Json<DiagnosisRun>, ApiError> {
    state
        .diagnose
        .latest()
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn diagnosis(
    State(state): State<AppState>,
    id: Result<Path<u64>, PathRejection>,
) -> Result<Json<DiagnosisRun>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    state
        .diagnose
        .get(DiagnosisId(id))
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn wifi_status(State(state): State<AppState>) -> Result<Json<WifiStatus>, ApiError> {
    state
        .wifi
        .status()
        .await
        .map(Json)
        .map_err(ApiError::from_wifi)
}

/// How old a cached radio scan may be before `POST /api/wifi/scan` rescans.
const NEARBY_MAX_AGE: Duration = Duration::from_secs(10);

async fn wifi_scan(State(state): State<AppState>) -> Result<Json<NearbyScan>, ApiError> {
    state
        .wifi
        .nearby(NEARBY_MAX_AGE)
        .await
        .map(Json)
        .map_err(ApiError::from_wifi)
}

/// The redacted text behind "Copy debug info".
async fn debug_info(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let text = crate::debug_info::build(&state).await?;
    Ok((
        [
            (CONTENT_TYPE, "text/plain; charset=utf-8"),
            (CACHE_CONTROL, "no-store"),
        ],
        text,
    ))
}

async fn monitor_status(State(state): State<AppState>) -> Json<MonitorStatus> {
    Json(state.monitor.status())
}

#[derive(Deserialize)]
struct MetricsQuery {
    target: Option<String>,
    metric: Option<String>,
    range: Option<String>,
}

const TARGETS: &str = "gateway, internet, dns, wifi, device:<id>";
const METRICS: &str = "rtt_ms, lookup_ms, rssi_dbm, link_rate_mbps";
const RANGES: &str = "1h, 24h, 7d, 30d";
const HOUR_MS: i64 = 3_600_000;

fn bad_query(_: QueryRejection) -> ApiError {
    ApiError::bad_request("Couldn't read the query string.")
}

async fn metrics(
    State(state): State<AppState>,
    query: Result<Query<MetricsQuery>, QueryRejection>,
) -> Result<Json<MetricSeries>, ApiError> {
    let Query(q) = query.map_err(bad_query)?;
    let target: Target = q
        .target
        .as_deref()
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| ApiError::bad_request(format!("target must be one of {TARGETS}.")))?;
    let metric: Metric = q
        .metric
        .as_deref()
        .and_then(|m| serde_json::from_value(serde_json::Value::String(m.to_string())).ok())
        .ok_or_else(|| ApiError::bad_request(format!("metric must be one of {METRICS}.")))?;
    let span = match q.range.as_deref() {
        Some("1h") => HOUR_MS,
        Some("24h") => 24 * HOUR_MS,
        Some("7d") => 7 * 24 * HOUR_MS,
        Some("30d") => 30 * 24 * HOUR_MS,
        _ => {
            return Err(ApiError::bad_request(format!(
                "range must be one of {RANGES}."
            )))
        }
    };
    let Some(network) = state.scans.current_network().await? else {
        let resolution = if span > 24 * HOUR_MS {
            Resolution::FiveMin
        } else {
            Resolution::Raw
        };
        return Ok(Json(MetricSeries {
            resolution,
            points: Vec::new(),
            sessions: Vec::new(),
        }));
    };
    let now = state.clock.now_ms();
    Ok(Json(
        state
            .store
            .metrics(network, target, metric, now - span, now)
            .await?,
    ))
}

#[derive(Deserialize)]
struct AlertsQuery {
    state: Option<String>,
    limit: Option<String>,
}

const DEFAULT_ALERTS: u32 = 50;
const MAX_ALERTS: u32 = 200;

async fn alerts(
    State(state): State<AppState>,
    query: Result<Query<AlertsQuery>, QueryRejection>,
) -> Result<Json<Vec<Alert>>, ApiError> {
    let Query(q) = query.map_err(bad_query)?;
    let filter = match q.state.as_deref() {
        None | Some("all") => AlertFilter::All,
        Some("active") => AlertFilter::Active,
        Some("open") => AlertFilter::Open,
        Some("acked") => AlertFilter::Acked,
        Some("resolved") => AlertFilter::Resolved,
        Some(_) => {
            return Err(ApiError::bad_request(
                "state must be one of all, active, open, acked, resolved.",
            ))
        }
    };
    let limit = match q.limit.as_deref() {
        None => DEFAULT_ALERTS,
        Some(text) => text
            .parse::<u32>()
            .map_err(|_| ApiError::bad_request("limit must be a whole number."))?
            .clamp(1, MAX_ALERTS),
    };
    let Some(network) = state.scans.current_network().await? else {
        return Ok(Json(Vec::new()));
    };
    Ok(Json(state.store.list_alerts(network, filter, limit).await?))
}

async fn ack_alert(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Alert>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    let network = state
        .scans
        .current_network()
        .await?
        .ok_or_else(ApiError::not_found)?;
    match state
        .store
        .ack_alert(network, AlertId(id), state.clock.now_ms())
        .await?
    {
        AckOutcome::Acked(alert) => {
            state.bus.publish(EventKind::AlertChanged {
                alert: alert.clone(),
            });
            Ok(Json(alert))
        }
        AckOutcome::AlreadyResolved(_) => Err(ApiError::alert_resolved()),
        AckOutcome::NotFound => Err(ApiError::not_found()),
    }
}

async fn incident(
    State(state): State<AppState>,
    id: Result<Path<i64>, PathRejection>,
) -> Result<Json<Incident>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::not_found())?;
    let network = state
        .scans
        .current_network()
        .await?
        .ok_or_else(ApiError::not_found)?;
    state
        .store
        .incident(network, IncidentId(id))
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn rules(State(state): State<AppState>) -> Result<Json<RuleSettings>, ApiError> {
    let network = state
        .scans
        .current_network()
        .await?
        .ok_or_else(ApiError::no_network)?;
    Ok(Json(state.store.rule_settings(network).await?))
}

/// `PUT /api/rules` body: all six fields, nothing else.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesRequest {
    starred_offline: bool,
    internet_down: bool,
    latency_high: bool,
    packet_loss: bool,
    new_device: bool,
    notifications: bool,
}

async fn set_rules(
    State(state): State<AppState>,
    body: Result<Json<RulesRequest>, JsonRejection>,
) -> Result<Json<RuleSettings>, ApiError> {
    let Json(r) = body.map_err(|_| {
        ApiError::bad_request(
            r#"Expected a JSON body with "starred_offline", "internet_down", "latency_high", "packet_loss", "new_device" and "notifications", each true or false."#,
        )
    })?;
    let network = state
        .scans
        .current_network()
        .await?
        .ok_or_else(ApiError::no_network)?;
    let settings = RuleSettings {
        starred_offline: r.starred_offline,
        internet_down: r.internet_down,
        latency_high: r.latency_high,
        packet_loss: r.packet_loss,
        new_device: r.new_device,
        notifications: r.notifications,
    };
    state.store.set_rule_settings(network, settings).await?;
    Ok(Json(settings))
}

fn phone(state: &AppState) -> Result<&Arc<PhoneAccess>, ApiError> {
    state.phone.as_ref().ok_or_else(|| {
        ApiError::phone_access_unavailable("Phone access isn't available in this build.")
    })
}

async fn phone_status(State(state): State<AppState>) -> Result<Json<PhoneStatus>, ApiError> {
    Ok(Json(phone(&state)?.status().await))
}

#[derive(Deserialize)]
struct PhoneAccessRequest {
    enabled: bool,
}

/// Turns phone access on or off. The status carries the phone URL (with its
/// token), which only a signed-in client ever sees.
async fn set_phone_access(
    State(state): State<AppState>,
    body: Result<Json<PhoneAccessRequest>, JsonRejection>,
) -> Result<Json<PhoneStatus>, ApiError> {
    let Json(request) = body.map_err(|_| {
        ApiError::bad_request(r#"Expected a JSON body {"enabled": true} or {"enabled": false}."#)
    })?;
    let phone = Arc::clone(phone(&state)?);
    // In a task of its own: axum drops this handler's future when the client
    // goes away, and a half-done turn-on or turn-off must still finish (and
    // publish `phone.changed`). A phone turning access off is cut off by the
    // turn-off itself, so its response is often lost.
    let transition = tokio::spawn(async move {
        if request.enabled {
            phone.enable().await
        } else {
            Ok(phone.disable().await)
        }
    });
    match transition.await {
        Ok(result) => result.map(Json),
        Err(error) => {
            tracing::warn!(%error, "the phone access change failed");
            Err(ApiError::internal("Phone access couldn't be changed."))
        }
    }
}

async fn update_status(State(state): State<AppState>) -> Result<Json<UpdateStatus>, ApiError> {
    Ok(Json(state.updates.status().await?))
}

#[derive(Deserialize)]
struct UpdateRequest {
    enabled: bool,
}

/// Turns the update check on or off. Turning it on checks right away, in the
/// background; `update.changed` tells the client when the answer is in.
async fn set_update(
    State(state): State<AppState>,
    body: Result<Json<UpdateRequest>, JsonRejection>,
) -> Result<Json<UpdateStatus>, ApiError> {
    let Json(request) = body.map_err(|_| {
        ApiError::bad_request(r#"Expected a JSON body {"enabled": true} or {"enabled": false}."#)
    })?;
    Ok(Json(state.updates.set_enabled(request.enabled).await?))
}

/// What `GET /api/host` and `PUT /api/host/autostart` answer.
#[derive(Serialize)]
struct HostBody {
    kind: HostKind,
    version: &'static str,
    autostart: AutostartBody,
}

#[derive(Serialize)]
struct AutostartBody {
    available: bool,
    /// `null` when there is no switch, or the OS could not say.
    enabled: Option<bool>,
}

/// Set once the first unreadable start-at-login state has been logged.
static AUTOSTART_READ_LOGGED: AtomicBool = AtomicBool::new(false);

async fn host_body(host: &HostInfo) -> Result<HostBody, ApiError> {
    let enabled = match &host.autostart {
        None => None,
        Some(autostart) => {
            let autostart = Arc::clone(autostart);
            // A blocking OS call (the registry).
            let read = tokio::task::spawn_blocking(move || autostart.is_enabled())
                .await
                .map_err(|error| {
                    tracing::error!(%error, "reading start at login panicked");
                    ApiError::internal("Net Lantern couldn't read start at login.")
                })?;
            match read {
                Ok(on) => Some(on),
                Err(_) => {
                    // The error text is not logged: it may hold a path.
                    if !AUTOSTART_READ_LOGGED.swap(true, Ordering::Relaxed) {
                        tracing::warn!("could not read the start at login setting");
                    }
                    None
                }
            }
        }
    };
    Ok(HostBody {
        kind: host.kind,
        version: host.version,
        autostart: AutostartBody {
            available: host.autostart.is_some(),
            enabled,
        },
    })
}

/// Which program hosts Net Lantern, its version, and whether it can start at login.
async fn host_info(State(state): State<AppState>) -> Result<Json<HostBody>, ApiError> {
    Ok(Json(host_body(&state.host).await?))
}

/// Turns start at login on or off (desktop app only). 409
/// `autostart_unavailable` without a switch; 500 `autostart_failed` when the
/// OS call fails.
async fn set_autostart(
    State(state): State<AppState>,
    session: Option<Extension<SessionKind>>,
    body: Result<Json<UpdateRequest>, JsonRejection>,
) -> Result<Json<HostBody>, ApiError> {
    // Before anything else: this changes the laptop's Windows login settings.
    if matches!(session, Some(Extension(SessionKind::Phone))) {
        return Err(ApiError::phone_not_allowed());
    }
    let Json(request) = body.map_err(|_| {
        ApiError::bad_request(r#"Expected a JSON body {"enabled": true} or {"enabled": false}."#)
    })?;
    let autostart = state
        .host
        .autostart
        .clone()
        .ok_or_else(ApiError::autostart_unavailable)?;
    let enabled = request.enabled;
    tokio::task::spawn_blocking(move || autostart.set_enabled(enabled))
        .await
        .map_err(|error| {
            tracing::error!(%error, "changing start at login panicked");
            ApiError::internal("Net Lantern couldn't change start at login.")
        })?
        .map_err(|reason| {
            // The host's text may hold a path: debug level only.
            tracing::debug!(%reason, "could not change the start at login setting");
            ApiError::autostart_failed()
        })?;
    Ok(Json(host_body(&state.host).await?))
}

/// The words the user types to confirm "Delete all data".
const RESET_CONFIRM: &str = "delete all data";

/// How long "Delete all data" waits for a running scan to stop.
const RESET_SCAN_WAIT: Duration = Duration::from_secs(5);

/// Deletes every stored record. 204 when done; 400 `bad_request`, with
/// nothing touched, unless the body is exactly the confirm text; 500
/// `reset_failed` when it could not be done.
async fn reset_data(
    State(state): State<AppState>,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    // Compared as a whole value: no other key, no array standing in for the
    // object, no other spelling.
    let expected = serde_json::json!({ "confirm": RESET_CONFIRM });
    let confirmed = matches!(&body, Ok(Json(value)) if *value == expected);
    if !confirmed {
        return Err(ApiError::bad_request(format!(
            r#"Expected a JSON body {{"confirm": "{RESET_CONFIRM}"}}."#
        )));
    }
    // In a task of its own: a client that goes away (a phone, cut off when
    // phone access turns off) must not leave monitoring stopped half-way.
    let task = tokio::spawn({
        let state = state.clone();
        async move { delete_all_data(&state).await }
    });
    match task.await {
        Ok(result) => result.map(|()| StatusCode::NO_CONTENT),
        Err(error) => {
            tracing::error!(%error, "deleting all data stopped part-way");
            // Whatever step it stopped at, monitoring runs again, once no
            // other reset is under way (one may be queued behind this).
            let _one_at_a_time = state.reset.write().await;
            state.monitor.start();
            Err(ApiError::reset_interrupted())
        }
    }
}

/// Every `/api` request that may write (any method but GET and HEAD)
/// waits while "Delete all data" runs, and holds it off while it runs
/// itself, so nothing is written half-way through a reset or with an id
/// from before it. Reads are never held up. The reset route takes the lock
/// itself.
async fn wait_for_reset(
    State(reset): State<Arc<tokio::sync::RwLock<()>>>,
    request: Request,
    next: Next,
) -> Response {
    let writes = !matches!(*request.method(), Method::GET | Method::HEAD)
        && request.uri().path().starts_with("/api/")
        && request.uri().path() != "/api/data/reset";
    if !writes {
        return next.run(request).await;
    }
    let _no_reset = reset.read().await;
    next.run(request).await
}

/// "Delete all data", in order: phone access off, no scan running, monitor
/// stopped, every table emptied (one transaction), what was kept in memory
/// forgotten, monitor started again, `data.reset` published. A failure
/// before the tables are emptied deletes nothing and still restarts the
/// monitor.
async fn delete_all_data(state: &AppState) -> Result<(), ApiError> {
    let _one_at_a_time = state.reset.write().await;
    if let Some(phone) = &state.phone {
        // Saves "off" (deleted below with the rest), ends phone sessions,
        // closes the listener and publishes `phone.changed`.
        phone.disable().await;
    }
    // No scan, consent change or new-router answer starts until the end.
    let _no_scans = state.scans.hold().await;
    if !state.scans.cancel_and_wait(RESET_SCAN_WAIT).await {
        tracing::warn!("a scan did not stop in time, so no data was deleted");
        return Err(ApiError::reset_failed());
    }
    state.monitor.stop(EndReason::Quit).await;
    let reset = state.store.reset_all().await;
    // `BackupNotRemoved` comes after the rows were deleted: memory must
    // follow the store either way.
    let cleared = matches!(reset, Ok(()) | Err(StoreError::BackupNotRemoved));
    if cleared {
        state.scans.forget_network();
        state.monitor.forget().await;
        state.updates.forget();
        state.diagnose.forget();
        state.wifi.forget();
    }
    state.monitor.start();
    if cleared {
        state.bus.publish(EventKind::DataReset {});
    }
    match reset {
        Ok(()) => {
            tracing::info!("all data deleted");
            Ok(())
        }
        Err(StoreError::BackupNotRemoved) => Err(ApiError::reset_backup_left()),
        Err(error) => {
            tracing::warn!(%error, "could not delete all data; nothing was deleted");
            Err(ApiError::reset_failed())
        }
    }
}

/// A known path with the wrong method. axum still adds the `Allow` header.
async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}
