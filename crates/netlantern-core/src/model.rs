use std::net::Ipv4Addr;

use netlantern_platform::MacAddr;
use serde::{Deserialize, Serialize};

/// Version of the HTTP/WebSocket API carried on every event.
pub const API_VERSION: u32 = 1;

/// Identifier of one scan run; serialized as a bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScanId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanMode {
    Quick,
    Deep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanState {
    Running,
    Done,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStage {
    Sweep,
    Arp,
    Merge,
}

/// One device as seen by a scan, in the shape streamed to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceSummary {
    pub ip: Ipv4Addr,
    pub mac: Option<MacAddr>,
    pub private_mac: bool,
    pub answers_ping: bool,
    pub rtt_ms: Option<u32>,
    pub is_self: bool,
    pub is_gateway: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_id_is_a_bare_number() {
        assert_eq!(
            serde_json::to_value(ScanId(7)).unwrap(),
            serde_json::json!(7)
        );
        assert_eq!(serde_json::from_str::<ScanId>("7").unwrap(), ScanId(7));
    }

    #[test]
    fn enums_serialize_snake_case() {
        let v =
            serde_json::to_value((ScanMode::Quick, ScanState::Cancelled, ScanStage::Arp)).unwrap();
        assert_eq!(v, serde_json::json!(["quick", "cancelled", "arp"]));
    }

    #[test]
    fn device_summary_json_shape() {
        let d = DeviceSummary {
            ip: "192.168.1.5".parse().unwrap(),
            mac: Some(MacAddr([0x50, 0xC7, 0xBF, 0x3A, 0x11, 0x02])),
            private_mac: false,
            answers_ping: true,
            rtt_ms: Some(4),
            is_self: false,
            is_gateway: true,
        };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            serde_json::json!({"ip":"192.168.1.5","mac":"50:C7:BF:3A:11:02","private_mac":false,
                "answers_ping":true,"rtt_ms":4,"is_self":false,"is_gateway":true})
        );
    }
}
