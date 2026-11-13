//! Monitoring data types.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::{DeviceId, NetworkId};

/// What a sample measures: the gateway, the internet, DNS, Wi-Fi or a device.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Gateway,
    Internet,
    Dns,
    Wifi,
    Device(DeviceId),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Target::Gateway => f.write_str("gateway"),
            Target::Internet => f.write_str("internet"),
            Target::Dns => f.write_str("dns"),
            Target::Wifi => f.write_str("wifi"),
            Target::Device(id) => write!(f, "device:{}", id.0),
        }
    }
}

impl FromStr for Target {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "gateway" => Ok(Target::Gateway),
            "internet" => Ok(Target::Internet),
            "dns" => Ok(Target::Dns),
            "wifi" => Ok(Target::Wifi),
            _ => s
                .strip_prefix("device:")
                .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|n| n.parse::<i64>().ok())
                .map(|n| Target::Device(DeviceId(n)))
                .ok_or_else(|| format!("unknown target {s:?}")),
        }
    }
}

impl Serialize for Target {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    RttMs,
    LookupMs,
    RssiDbm,
    LinkRateMbps,
}

/// One measurement. For `RttMs` and `LookupMs`, `None` means no reply.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub target: Target,
    pub metric: Metric,
    pub ts: i64,
    pub value: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Sleep,
    NetworkLeft,
    Quit,
    Crash,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSpan {
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub end_reason: Option<EndReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricPoint {
    pub ts: i64,
    pub min: Option<f64>,
    pub avg: Option<f64>,
    pub max: Option<f64>,
    pub count: u32,
    pub lost: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Raw,
    FiveMin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSeries {
    pub resolution: Resolution,
    pub points: Vec<MetricPoint>,
    pub sessions: Vec<SessionSpan>,
}

/// A service check on a starred device. Ping is implicit and never stored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Check {
    Tcp {
        port: u16,
    },
    Http {
        port: u16,
        path: String,
        https: bool,
    },
}

/// The result of running one check. `detail` is "open", "no answer",
/// "HTTP {status}", or "error: {msg}" (message cut to 100 characters).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub check: Check,
    pub ok: bool,
    pub at: i64,
    pub detail: String,
}

/// What an alert is about. Each key has at most one active alert per network.
///
/// The text form (`Display` / `FromStr`, and JSON) is `router_down`, `internet_down`,
/// `latency_high`, `packet_loss:gateway`, `packet_loss:device:12`,
/// `starred_offline:12` or `new_device:12`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AlertKey {
    RouterDown,
    InternetDown,
    LatencyHigh,
    /// Only `Target::Gateway` or `Target::Device`.
    PacketLoss(Target),
    StarredOffline(DeviceId),
    NewDevice(DeviceId),
}

impl AlertKey {
    /// The rule switch that covers this key. The "internet down" switch covers
    /// both `RouterDown` and `InternetDown`.
    pub fn rule(&self) -> Rule {
        match self {
            AlertKey::RouterDown | AlertKey::InternetDown => Rule::InternetDown,
            AlertKey::LatencyHigh => Rule::LatencyHigh,
            AlertKey::PacketLoss(_) => Rule::PacketLoss,
            AlertKey::StarredOffline(_) => Rule::StarredOffline,
            AlertKey::NewDevice(_) => Rule::NewDevice,
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            AlertKey::RouterDown | AlertKey::InternetDown | AlertKey::StarredOffline(_) => {
                Severity::Critical
            }
            AlertKey::LatencyHigh | AlertKey::PacketLoss(_) => Severity::Warning,
            AlertKey::NewDevice(_) => Severity::Info,
        }
    }

    /// Whether this key is held back (suppressed) while the router is down.
    pub fn depends_on_router(&self) -> bool {
        !matches!(
            self,
            AlertKey::RouterDown | AlertKey::PacketLoss(Target::Gateway) | AlertKey::NewDevice(_)
        )
    }
}

impl fmt::Display for AlertKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlertKey::RouterDown => f.write_str("router_down"),
            AlertKey::InternetDown => f.write_str("internet_down"),
            AlertKey::LatencyHigh => f.write_str("latency_high"),
            AlertKey::PacketLoss(t) => write!(f, "packet_loss:{t}"),
            AlertKey::StarredOffline(id) => write!(f, "starred_offline:{}", id.0),
            AlertKey::NewDevice(id) => write!(f, "new_device:{}", id.0),
        }
    }
}

/// A device id written as unsigned digits only.
fn parse_device_id(n: &str) -> Option<DeviceId> {
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse::<i64>().ok().map(DeviceId)
}

impl FromStr for AlertKey {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let key = match s {
            "router_down" => Some(AlertKey::RouterDown),
            "internet_down" => Some(AlertKey::InternetDown),
            "latency_high" => Some(AlertKey::LatencyHigh),
            _ => {
                if let Some(t) = s.strip_prefix("packet_loss:") {
                    match t.parse::<Target>() {
                        Ok(t @ (Target::Gateway | Target::Device(_))) => {
                            Some(AlertKey::PacketLoss(t))
                        }
                        _ => None,
                    }
                } else if let Some(n) = s.strip_prefix("starred_offline:") {
                    parse_device_id(n).map(AlertKey::StarredOffline)
                } else if let Some(n) = s.strip_prefix("new_device:") {
                    parse_device_id(n).map(AlertKey::NewDevice)
                } else {
                    None
                }
            }
        };
        key.ok_or_else(|| format!("unknown alert key {s:?}"))
    }
}

impl Serialize for AlertKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AlertKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// An alert rule the user can switch on or off per network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rule {
    StarredOffline,
    InternetDown,
    LatencyHigh,
    PacketLoss,
    NewDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

/// Which rules are on, plus whether desktop notifications are sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleSettings {
    pub starred_offline: bool,
    pub internet_down: bool,
    pub latency_high: bool,
    pub packet_loss: bool,
    pub new_device: bool,
    pub notifications: bool,
}

impl Default for RuleSettings {
    fn default() -> Self {
        Self {
            starred_offline: true,
            internet_down: true,
            latency_high: true,
            packet_loss: true,
            new_device: true,
            notifications: true,
        }
    }
}

impl RuleSettings {
    pub fn enabled(&self, rule: Rule) -> bool {
        match rule {
            Rule::StarredOffline => self.starred_offline,
            Rule::InternetDown => self.internet_down,
            Rule::LatencyHigh => self.latency_high,
            Rule::PacketLoss => self.packet_loss,
            Rule::NewDevice => self.new_device,
        }
    }
}

/// One evaluation of one key: whether it is failing at `ts`, with the copy
/// to use if it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub key: AlertKey,
    pub failing: bool,
    pub ts: i64,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AlertId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IncidentId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Open,
    Acked,
    Resolved,
}

/// A stored alert. `key` is the text form of its [`AlertKey`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alert {
    pub id: AlertId,
    pub rule: Rule,
    pub key: String,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub suppressed: bool,
    pub state: AlertState,
    pub incident_id: IncidentId,
    pub opened_at: i64,
    pub acked_at: Option<i64>,
    pub resolved_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    LastGood,
    Detected,
    Alerted,
    Affected,
    Diagnosed,
    Acknowledged,
    Recovered,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub ts: i64,
    pub kind: TimelineKind,
    pub text: String,
    pub alert_id: Option<AlertId>,
}

/// A problem and everything it set off. `alerts` holds all of them, the root
/// included, oldest first; `timeline` is ordered by time, then insertion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    pub id: IncidentId,
    pub opened_at: i64,
    pub resolved_at: Option<i64>,
    pub root: Alert,
    pub alerts: Vec<Alert>,
    pub timeline: Vec<TimelineEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertFilter {
    All,
    /// Open or acknowledged.
    Active,
    Open,
    Acked,
    Resolved,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AckOutcome {
    Acked(Alert),
    AlreadyResolved(Alert),
    NotFound,
}

/// What the monitor is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitorState {
    Monitoring,
    /// Connected to a network no scan has identified yet.
    WaitingForScan,
    NotConnected,
}

/// One tick's headline numbers, as published on `monitor.tick`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorTick {
    pub ts: i64,
    pub network_id: NetworkId,
    pub gateway_rtt_ms: Option<f64>,
    /// The ping, else the TCP connect time to port 443.
    pub internet_rtt_ms: Option<f64>,
    /// Lost pings to the router in the loss window.
    pub router_lost: u32,
    pub router_window: u32,
    pub dns_ms: Option<f64>,
    pub wifi_rssi_dbm: Option<i32>,
}

/// The latest check results for one starred device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceChecks {
    pub device_id: DeviceId,
    pub outcomes: Vec<CheckOutcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorStatus {
    pub state: MonitorState,
    pub network_id: Option<NetworkId>,
    pub session_started_at: Option<i64>,
    pub last: Option<MonitorTick>,
    pub checks: Vec<DeviceChecks>,
}
