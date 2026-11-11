//! Monitoring data types.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::DeviceId;

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
