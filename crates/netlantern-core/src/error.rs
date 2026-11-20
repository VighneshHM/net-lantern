use netlantern_platform::PlatformError;
use thiserror::Error;

use crate::model::NetworkId;
use crate::store::StoreError;

/// What a scan or a consent request reports while a known network's router
/// looks replaced and the user has not said whether to keep the network.
pub const NEW_ROUTER_TEXT: &str = "This looks like a network you know with a new router.";

/// Why an answer about a replaced router was refused: nothing is pending.
pub const NO_DECISION_TEXT: &str = "There's no question about a new router to answer.";

/// Why a scan could not be started.
#[derive(Debug, PartialEq, Error)]
pub enum CoreError {
    #[error("not connected to a network")]
    NotConnected,
    /// The request is understood but this build cannot do it yet.
    // Kept for requests this build cannot serve.
    #[error("not supported: {0}")]
    Unsupported(&'static str),
    /// A deep scan was asked for on a network the user has not allowed it on.
    #[error("deep scan needs permission for this network")]
    ConsentNeeded,
    /// The network could not be identified; the reason is in words for the user.
    #[error("{0}")]
    Unidentified(String),
    /// The router is new, but a known network has this SSID and subnet: the
    /// user must say whether it is that network (`candidate`, known as
    /// `label`) before it is scanned.
    #[error("{}", NEW_ROUTER_TEXT)]
    NetworkDecisionNeeded { candidate: NetworkId, label: String },
    #[error(transparent)]
    Platform(PlatformError),
    #[error(transparent)]
    Store(StoreError),
}

impl From<PlatformError> for CoreError {
    fn from(err: PlatformError) -> Self {
        match err {
            PlatformError::NotConnected => CoreError::NotConnected,
            other => CoreError::Platform(other),
        }
    }
}

impl From<StoreError> for CoreError {
    fn from(err: StoreError) -> Self {
        CoreError::Store(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_connected_maps_to_not_connected() {
        assert_eq!(
            CoreError::from(PlatformError::NotConnected),
            CoreError::NotConnected
        );
    }

    #[test]
    fn other_platform_errors_are_wrapped() {
        let err = PlatformError::PermissionNeeded {
            what: "administrator rights".into(),
        };
        assert_eq!(
            CoreError::from(err.clone()),
            CoreError::Platform(err.clone())
        );
        assert_eq!(
            CoreError::Platform(err).to_string(),
            "permission needed: administrator rights"
        );
    }

    #[test]
    fn messages_are_readable() {
        assert_eq!(
            CoreError::NotConnected.to_string(),
            "not connected to a network"
        );
        assert_eq!(
            CoreError::Unsupported("not yet").to_string(),
            "not supported: not yet"
        );
        assert_eq!(
            CoreError::ConsentNeeded.to_string(),
            "deep scan needs permission for this network"
        );
        assert_eq!(
            CoreError::Unidentified("Your router didn't answer.".into()).to_string(),
            "Your router didn't answer."
        );
        assert_eq!(
            CoreError::NetworkDecisionNeeded {
                candidate: NetworkId(1),
                label: "Home".into(),
            }
            .to_string(),
            NEW_ROUTER_TEXT
        );
    }
}
