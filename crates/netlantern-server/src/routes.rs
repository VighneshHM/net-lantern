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
use axum::routing::{get, post};
use axum::{Json, Router};
use netlantern_core::{CoreError, DeviceSummary, ScanId, ScanMode, ScanSnapshot, StartOutcome};
use netlantern_platform::Interface;
use serde::{Deserialize, Serialize};

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
    };
    Router::new()
        .route("/api/session", post(create_session))
        .route("/api/network", get(network))
        .route("/api/scans", post(start_scan))
        .route("/api/scans/{id}", get(scan))
        .route("/api/devices", get(devices))
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
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn devices(State(state): State<AppState>) -> Json<Vec<DeviceSummary>> {
    Json(state.devices.list())
}

/// A known path with the wrong method. axum still adds the `Allow` header.
async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}
