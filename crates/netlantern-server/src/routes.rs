//! The REST API and the router that puts it, the event stream, and the UI
//! fallback behind the guard.
//!
//! Every error is an [`ApiError`] JSON body: extractor rejections (bad JSON, a
//! non-numeric id) are mapped rather than passed through as axum's plain-text
//! responses.

use std::sync::Arc;

use axum::extract::rejection::{JsonRejection, PathRejection};
use axum::extract::{Path, State};
use axum::http::header::SET_COOKIE;
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use netlantern_core::{
    CoreError, Device, DeviceId, DevicePatch, DiagnoseStart, DiagnosisId, DiagnosisRun, NearbyScan,
    ScanId, ScanMode, ScanSnapshot, StartOutcome, WifiStatus,
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
            r#"Expected a JSON body with any of "name" (text or null), "starred" (true or false), "is_new" (false)."#,
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
    state
        .store
        .update_device(network, DeviceId(id), patch)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
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

/// A known path with the wrong method. axum still adds the `Allow` header.
async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}
