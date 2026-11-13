use std::net::Ipv4Addr;

use netlantern_platform::MacAddr;
use serde::{Deserialize, Serialize};

use crate::identify::classify::Signals;
use crate::identify::os::OsGuess;
use crate::monitor::Check;

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
    Ports,
    /// The deep scan's port probes; `done`/`total` count probes.
    PortScan,
    /// The deep scan's banner and page-title reads; `done`/`total` count reads.
    Banners,
    Discover,
    Merge,
}

/// What kind of device something is, as guessed by the classifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Router,
    Laptop,
    Phone,
    Tablet,
    Printer,
    Tv,
    Speaker,
    Camera,
    Nas,
    Console,
    Iot,
    Unknown,
}

/// Where a scan is at, as returned by the API.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScanSnapshot {
    pub id: ScanId,
    pub mode: ScanMode,
    pub state: ScanState,
    pub stage: ScanStage,
    /// Addresses swept so far (equal to `total` once the sweep is over); in a
    /// deep scan's `port_scan` and `banners` stages, probes done so far.
    pub done: u32,
    pub total: u32,
    pub devices_found: u32,
    /// Why the scan failed, in words for the user.
    pub error: Option<String>,
    /// Only the gateway (and this laptop) answered: the network likely keeps
    /// clients apart.
    pub isolated: bool,
}

/// Identifier of a network (a gateway MAC plus a subnet), serialized as a bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NetworkId(pub i64);

/// Identifier of a stored device, serialized as a bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub i64);

/// A service a device announced: `source` is `mdns`, `ssdp`, `netbios` or `dns`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub source: String,
    pub value: String,
}

/// An open port with what is known about the service behind it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortInfo {
    pub port: u16,
    pub service: Option<String>,
    pub banner: Option<String>,
}

/// What a deep scan learned about one device's ports.
#[derive(Clone, Debug)]
pub struct DeepFindings {
    /// Every port that was probed on this device.
    pub scanned: Vec<u16>,
    /// The open ones, with what they said.
    pub ports: Vec<PortInfo>,
}

/// Everything one scan saw of one address, handed to the store to merge.
#[derive(Clone, Debug)]
pub struct Sighting {
    pub ip: Ipv4Addr,
    pub mac: Option<MacAddr>,
    pub answers_ping: bool,
    pub rtt_ms: Option<u32>,
    pub ttl: Option<u8>,
    pub is_self: bool,
    pub is_gateway: bool,
    pub open_ports: Vec<u16>,
    /// Set by a deep scan: which ports it probed and what the open ones said.
    pub deep: Option<DeepFindings>,
    /// `signals.vendor` is filled in by the store from the OUI table.
    pub signals: Signals,
}

/// A stored device in the shape served to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub ip: Option<Ipv4Addr>,
    /// The MAC seen most recently.
    pub mac: Option<MacAddr>,
    pub private_mac: bool,
    pub vendor: Option<String>,
    pub hostname: Option<String>,
    pub name: Option<String>,
    pub display_name: String,
    #[serde(rename = "type")]
    pub device_type: DeviceType,
    pub type_confidence: f32,
    pub starred: bool,
    pub is_new: bool,
    pub online: bool,
    pub answers_ping: bool,
    pub rtt_ms: Option<u32>,
    pub is_self: bool,
    pub is_gateway: bool,
    pub first_seen: i64,
    pub last_seen: i64,
    pub services: Vec<Service>,
    pub open_ports: Vec<u16>,
    /// Open ports with their service names and banners, in port order.
    #[serde(default)]
    pub ports: Vec<PortInfo>,
    /// A rough guess at the operating system, from the stored evidence.
    #[serde(default)]
    pub os_guess: Option<OsGuess>,
    /// When a deep scan last checked this device's ports.
    #[serde(default)]
    pub deep_scanned_at: Option<i64>,
    /// Service checks on a starred device, in the order they run.
    #[serde(default)]
    pub checks: Vec<Check>,
    /// When `online` last flipped, once monitoring has recorded presence.
    #[serde(default)]
    pub presence_changed_at: Option<i64>,
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
        let v = serde_json::to_value((ScanStage::Ports, ScanStage::Discover)).unwrap();
        assert_eq!(v, serde_json::json!(["ports", "discover"]));
        let v = serde_json::to_value((ScanStage::PortScan, ScanStage::Banners)).unwrap();
        assert_eq!(v, serde_json::json!(["port_scan", "banners"]));
    }
}
