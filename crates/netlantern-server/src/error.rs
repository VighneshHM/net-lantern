//! The one error type every route returns: a status plus a JSON body of
//! `{"code": ..., "message": ..., "hint": ...|null}` (and `scan_id` on
//! `scan_in_progress`).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use netlantern_core::{CoreError, StoreError};
use netlantern_platform::{PlatformError, LOCATION_PERMISSION};
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

    pub fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "This method isn't allowed here.",
        )
    }

    /// The database failed. The raw error goes to the log, never to the client.
    pub fn store(err: &StoreError) -> Self {
        tracing::warn!(error = %err, "storage error");
        Self {
            hint: Some(
                "Restart Net Lantern. If this keeps happening, copy the log into a bug report."
                    .to_string(),
            ),
            ..Self::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "Net Lantern couldn't read or write its data.",
            )
        }
    }

    /// Windows won't reveal Wi-Fi details until Location access is on.
    pub fn location_permission() -> Self {
        Self {
            hint: Some(
                r#"Turn on Location services and "Let desktop apps access your location" in Settings > Privacy & security > Location."#
                    .to_string(),
            ),
            ..Self::new(
                StatusCode::FORBIDDEN,
                "location_permission",
                "Windows is hiding Wi-Fi details from Net Lantern.",
            )
        }
    }

    /// There is no Wi-Fi to read (no adapter, radio off, service stopped).
    pub fn wifi_unavailable(reason: &str) -> Self {
        Self {
            hint: Some("Check that Wi-Fi is turned on.".to_string()),
            ..Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "wifi_unavailable",
                format!("Wi-Fi details aren't available: {reason}."),
            )
        }
    }

    /// Like `From<CoreError>`, but an unavailable platform means no Wi-Fi.
    pub fn from_wifi(err: CoreError) -> Self {
        match err {
            CoreError::Platform(PlatformError::Unavailable { reason }) => {
                Self::wifi_unavailable(&reason)
            }
            other => other.into(),
        }
    }

    /// The OS refused or failed a request for some reason other than being
    /// offline (permissions, a missing service, I/O).
    pub fn platform_error(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "platform_error", msg)
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        Self::store(&err)
    }
}

impl From<CoreError> for ApiError {
    fn from(err: CoreError) -> Self {
        match err {
            CoreError::NotConnected => Self::not_connected(),
            CoreError::Unsupported(msg) => Self::unsupported(msg),
            CoreError::Platform(PlatformError::PermissionNeeded { what })
                if what == LOCATION_PERMISSION =>
            {
                Self::location_permission()
            }
            CoreError::Platform(err) => Self::platform_error(err.to_string()),
            CoreError::Store(err) => Self::store(&err),
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

    #[tokio::test]
    async fn method_not_allowed_is_405_with_a_null_hint() {
        let err = ApiError::method_not_allowed();
        assert_eq!(err.status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(err.code, "method_not_allowed");

        let res = err.into_response();
        assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
        let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body,
            json!({
                "code": "method_not_allowed",
                "message": "This method isn't allowed here.",
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
        let location = PlatformError::PermissionNeeded {
            what: LOCATION_PERMISSION.into(),
        };
        assert_eq!(
            ApiError::from(CoreError::Platform(location)),
            ApiError::location_permission()
        );
    }

    #[test]
    fn wifi_routes_map_an_unavailable_platform_to_wifi_unavailable() {
        let err = CoreError::Platform(PlatformError::Unavailable {
            reason: "no Wi-Fi adapter".into(),
        });
        let api = ApiError::from_wifi(err);
        assert_eq!(api.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(api.code, "wifi_unavailable");
        assert_eq!(
            api.message,
            "Wi-Fi details aren't available: no Wi-Fi adapter."
        );
        assert_eq!(api.hint.as_deref(), Some("Check that Wi-Fi is turned on."));
    }
}
