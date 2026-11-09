//! Wire types, probe types and the diagnosis settings.

use std::net::Ipv4Addr;
use std::time::Duration;

use netlantern_platform::{InterfaceKind, WifiConnection};
use serde::{Deserialize, Serialize};

use crate::wifi::CurrentLoad;

/// One step of the diagnosis, from the laptop outward.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Link,
    Router,
    Internet,
    Dns,
    Website,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerStatus {
    Pass,
    Warn,
    Fail,
    Skipped,
}

/// What a judge concluded about a layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finding {
    EthernetOk,
    LinkOk,
    SignalHidden,
    /// Connected, but Windows gave no signal details (not location access).
    LinkDetailsUnavailable,
    NotConnected,
    WeakSignal,
    CrowdedChannel,
    WeakAndCrowded,
    RouterOk,
    RouterIgnoresPing,
    RouterUnreachable,
    RouterLossy,
    RouterSlow,
    InternetOk,
    CaptivePortal,
    InternetDown,
    InternetLossy,
    InternetSlow,
    DnsOk,
    NoDnsServer,
    DnsServerProblem,
    DnsUnreachable,
    DnsPartial,
    WebsiteOk,
    TlsIntercepted,
    WebsiteUnreachable,
    Healthy,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerResult {
    pub layer: Layer,
    pub title: String,
    pub status: LayerStatus,
    pub finding: Finding,
    pub evidence: Vec<String>,
    pub explanation: String,
    pub fix: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub finding: Finding,
    pub status: LayerStatus,
    /// The layer the verdict comes from; `None` when everything is healthy.
    pub layer: Option<Layer>,
    pub headline: String,
    pub fix: Option<String>,
}

/// Identifier of one diagnosis run; serialized as a bare number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosisId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosisRun {
    pub id: DiagnosisId,
    pub state: RunState,
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub steps: Vec<LayerResult>,
    pub verdict: Option<Verdict>,
    pub error: Option<String>,
}

/// What the Wi-Fi radio reported.
#[derive(Clone, Debug, PartialEq)]
pub enum WifiReading {
    NotWifi,
    Connected(WifiConnection),
    Disconnected,
    PermissionNeeded,
    Unavailable(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinkProbe {
    /// `None` means not connected.
    pub interface: Option<InterfaceKind>,
    pub wifi: WifiReading,
    pub congestion: Option<CurrentLoad>,
    pub congestion_advice: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PingStats {
    pub target: Ipv4Addr,
    pub sent: u32,
    pub rtts_ms: Vec<u32>,
}

impl PingStats {
    pub fn replies(&self) -> u32 {
        self.rtts_ms.len() as u32
    }

    /// Average round trip in whole milliseconds; `None` when nothing replied.
    pub fn avg_ms(&self) -> Option<u32> {
        if self.rtts_ms.is_empty() {
            return None;
        }
        let sum: u64 = self.rtts_ms.iter().map(|&r| u64::from(r)).sum();
        Some((sum as f64 / self.rtts_ms.len() as f64).round() as u32)
    }

    pub fn lost(&self) -> u32 {
        self.sent.saturating_sub(self.replies())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RouterProbe {
    pub pings: PingStats,
    /// Asked only when no ping replied.
    pub answers_arp: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PortalCheck {
    Expected,
    Redirect {
        status: u16,
        location: Option<String>,
    },
    UnexpectedAnswer {
        status: u16,
    },
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TcpCheck {
    pub target: Ipv4Addr,
    pub port: u16,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InternetProbe {
    pub pings: Vec<PingStats>,
    pub tcp: TcpCheck,
    pub portal: PortalCheck,
}

/// One DNS query. An `Ok` with no addresses counts as failed.
#[derive(Clone, Debug, PartialEq)]
pub struct DnsLookup {
    pub name: String,
    pub server: Ipv4Addr,
    pub result: Result<Vec<Ipv4Addr>, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DnsProbe {
    pub system_servers: Vec<Ipv4Addr>,
    pub direct_server: Ipv4Addr,
    /// One lookup per test name, sent to the system servers.
    pub system: Vec<DnsLookup>,
    pub direct: Vec<DnsLookup>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WebsiteOutcome {
    Status(u16),
    Certificate(String),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WebsiteProbe {
    pub url: String,
    pub outcome: WebsiteOutcome,
}

/// Settings for a diagnosis run.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnoseConfig {
    pub wifi_warn_dbm: i32,
    pub gateway_pings: u32,
    pub gateway_ping_timeout: Duration,
    pub router_warn_rtt_ms: u32,
    pub arp_timeout: Duration,
    pub internet_targets: Vec<Ipv4Addr>,
    pub internet_pings: u32,
    pub internet_ping_timeout: Duration,
    pub internet_warn_rtt_ms: u32,
    pub tcp_target: (Ipv4Addr, u16),
    pub tcp_timeout: Duration,
    pub portal_url: String,
    pub portal_expected_body: String,
    pub dns_names: Vec<String>,
    pub direct_dns: Ipv4Addr,
    pub website_url: String,
    pub layer_timeout: Duration,
    pub nearby_max_age: Duration,
    /// The wait before the router layer when this diagnosis ran a fresh radio
    /// scan, so the scan's radio time doesn't read as router packet loss.
    pub post_scan_settle: Duration,
}

impl Default for DiagnoseConfig {
    fn default() -> Self {
        Self {
            wifi_warn_dbm: -70,
            gateway_pings: 5,
            gateway_ping_timeout: Duration::from_secs(1),
            router_warn_rtt_ms: 50,
            arp_timeout: Duration::from_secs(3),
            internet_targets: vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(8, 8, 8, 8)],
            internet_pings: 3,
            internet_ping_timeout: Duration::from_secs(1),
            internet_warn_rtt_ms: 100,
            tcp_target: (Ipv4Addr::new(1, 1, 1, 1), 443),
            tcp_timeout: Duration::from_secs(2),
            portal_url: "http://www.msftconnecttest.com/connecttest.txt".into(),
            portal_expected_body: "Microsoft Connect Test".into(),
            dns_names: vec!["www.microsoft.com".into(), "www.cloudflare.com".into()],
            direct_dns: Ipv4Addr::new(1, 1, 1, 1),
            website_url: "https://www.microsoft.com/".into(),
            layer_timeout: Duration::from_secs(15),
            nearby_max_age: Duration::from_secs(60),
            post_scan_settle: Duration::from_secs(2),
        }
    }
}
