//! The one error type every route returns: a status plus a JSON body of
//! `{"code": ..., "message": ..., "hint": ...|null}` (and `scan_id` on
//! `scan_in_progress`).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use netbot_core::CoreError;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
    pub scan_id: Option<u64>,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            hint: None,
            scan_id: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "This dashboard session has expired.",
        )
    }

    pub fn forbidden_host() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden_host",
            "Request blocked: unexpected Host header.",
        )
    }

    pub fn forbidden_origin() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden_origin",
            "Request blocked: cross-site request.",
        )
    }

    pub fn not_connected() -> Self {
        Self {
            hint: Some("Connect to a Wi-Fi or Ethernet network, then try again.".to_string()),
            ..Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "not_connected",
                "This computer is not connected to a network.",
            )
        }
    }

    pub fn scan_in_progress(id: u64) -> Self {
        Self {
            scan_id: Some(id),
            ..Self::new(
                StatusCode::CONFLICT,
                "scan_in_progress",
                "A scan is already running.",
            )
        }
    }

    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, "unsupported", msg)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "Not found.")
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", msg)
    }

    /// The OS refused or failed a request for some reason other than being
    /// offline (permissions, a missing service, I/O).
    pub fn platform_error(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "platform_error", msg)
    }
}

impl From<CoreError> for ApiError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::NotConnected => Self::not_connected(),
            CoreError::Unsupported(msg) => Self::unsupported(msg),
            CoreError::Platform(err) => Self::platform_error(err.to_string()),
        }
    }
}

/// The wire shape. `hint` is always serialized (as `null` when absent);
/// `scan_id` only when set.
#[derive(Serialize)]
struct Body<'a> {
    code: &'a str,
    message: &'a str,
    hint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scan_id: Option<u64>,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = Body {
            code: self.code,
            message: &self.message,
            hint: self.hint.as_deref(),
            scan_id: self.scan_id,
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use netbot_platform::PlatformError;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn platform_error_is_500_with_a_null_hint() {
        let err = ApiError::platform_error("permission needed: administrator rights");
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.code, "platform_error");

        let res = err.into_response();
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body,
            json!({
                "code": "platform_error",
                "message": "permission needed: administrator rights",
                "hint": null,
            })
        );
    }

    #[test]
    fn core_errors_map_to_their_api_errors() {
        assert_eq!(
            ApiError::from(CoreError::NotConnected),
            ApiError::not_connected()
        );
        assert_eq!(
            ApiError::from(CoreError::Unsupported("deep scan arrives in phase 4")),
            ApiError::unsupported("deep scan arrives in phase 4")
        );
        let denied = PlatformError::PermissionNeeded {
            what: "administrator rights".into(),
        };
        assert_eq!(
            ApiError::from(CoreError::Platform(denied)),
            ApiError::platform_error("permission needed: administrator rights")
        );
    }
}
