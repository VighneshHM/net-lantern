//! The REST API and the router that puts it, the event stream, and the UI
//! fallback behind the guard.
//!
//! Every error is an [`ApiError`] JSON body: extractor rejections (bad JSON, a
//! non-numeric id) are mapped rather than passed through as axum's plain-text
//! responses.

use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use netlantern_core::{
    AckOutcome, Alert, AlertFilter, AlertId, Check, CoreError, Device, DeviceId, DevicePatch,
    DiagnoseStart, DiagnosisId, DiagnosisRun, EventKind, Incident, IncidentId, Metric,
    MetricSeries, MonitorStatus, NearbyScan, Resolution, RuleSettings, ScanId, ScanMode,
    ScanSnapshot, StartOutcome, Target, WifiStatus,
};
use netlantern_platform::Interface;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::assets;
use crate::error::ApiError;
use crate::guard::{guard, GuardState, SESSION_COOKIE};
use crate::state::AppState;
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
    };
    Router::new()
        .route("/api/session", post(create_session))
        .route("/api/network", get(network))
        .route("/api/scans", post(start_scan))
        .route("/api/scans/latest", get(latest_scan))
        .route("/api/scans/{id}", get(scan))
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
        .route("/api/events", get(ws::events))
        // Applies only to the routes above, so it stays after the last one.
        .method_not_allowed_fallback(method_not_allowed)
        .fallback(assets::fallback)
        .with_state(state)
        .layer(from_fn_with_state(guard_state, guard))
}

/// `POST /api/session` body. No `Debug`, so the token cannot end up in a log.
#[derive(Deserialize)]
struct SessionRequest {
    token: String,
}

/// Exchanges a one-time launch token for the session cookie.
async fn create_session(
    State(state): State<AppState>,
    body: Result<Json<SessionRequest>, JsonRejection>,
) -> Result<impl IntoResponse, ApiError> {
    let Json(request) =
        body.map_err(|_| ApiError::bad_request(r#"Expected a JSON body {"token": "..."}."#))?;
    let session = state
        .auth
        .redeem(&request.token)
        .ok_or_else(ApiError::unauthorized)?;
    let cookie = format!("{SESSION_COOKIE}={session}; HttpOnly; SameSite=Strict; Path=/");
    Ok((StatusCode::NO_CONTENT, [(SET_COOKIE, cookie)]))
}

async fn network(State(state): State<AppState>) -> Result<Json<Interface>, ApiError> {
    let interface = state
        .platform
        .active_interface()
        .await
        .map_err(CoreError::from)?;
    Ok(Json(interface))
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

/// A known path with the wrong method. axum still adds the `Allow` header.
async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}
