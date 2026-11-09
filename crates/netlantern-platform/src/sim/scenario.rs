use std::collections::HashMap;
use std::net::Ipv4Addr;

use serde::Deserialize;
use thiserror::Error;

use crate::{InterfaceKind, MacAddr, NearbyNetwork, NetbiosInfo, WifiConnection};

/// Why a scenario could not be loaded.
#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("cannot read scenario file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid scenario JSON: {0}")]
    Json(#[from] serde_json::Error),
}

/// On-disk scenario format. Kept separate from the public [`crate::Interface`]
/// type so the file format can differ (`dns` here, `dns_servers` there).
#[derive(Debug, Deserialize)]
pub(super) struct Scenario {
    pub name: String,
    /// `null` means the laptop is not connected to any network.
    pub interface: Option<ScenarioInterface>,
    pub devices: Vec<ScenarioDevice>,
    /// Hostname -> addresses.
    pub dns: HashMap<String, Vec<Ipv4Addr>>,
    /// URL -> canned response.
    pub http: HashMap<String, ScenarioHttp>,
    /// Wi-Fi data; absent means the scenario has none.
    pub wifi: Option<ScenarioWifi>,
    /// Outside hosts: they answer ping and TCP probes but are not on the LAN.
    #[serde(default)]
    pub internet: Vec<ScenarioHost>,
    /// DNS servers that never answer.
    #[serde(default)]
    pub dns_down: Vec<Ipv4Addr>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioInterface {
    pub ip: Ipv4Addr,
    pub prefix: u8,
    pub gateway: Ipv4Addr,
    pub dns: Vec<Ipv4Addr>,
    pub kind: InterfaceKind,
    pub mac: MacAddr,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioDevice {
    pub ip: Ipv4Addr,
    pub mac: MacAddr,
    /// Round-trip time in milliseconds; `null` means the device ignores ping.
    pub ping_ms: Option<u64>,
    #[serde(default = "default_ttl")]
    pub ttl: u8,
    /// Whether the device shows up in ARP (resolve and neighbor table).
    #[serde(default = "default_arp")]
    pub arp: bool,
    #[serde(default)]
    pub open_ports: Vec<u16>,
    #[serde(default)]
    pub mdns: Vec<ScenarioMdns>,
    pub ssdp: Option<ScenarioSsdp>,
    pub netbios: Option<NetbiosInfo>,
    /// Reverse-DNS (PTR) name.
    pub rdns: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioMdns {
    pub service: String,
    pub instance: String,
    pub hostname: Option<String>,
    pub friendly_name: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioSsdp {
    pub st: String,
    pub server: Option<String>,
    pub location: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioHttp {
    pub status: u16,
    #[serde(default)]
    pub body: String,
    pub location: Option<String>,
    /// When set, the request fails with a certificate error and `status` is ignored.
    pub tls_error: Option<String>,
}

fn default_ttl() -> u8 {
    64
}

fn default_arp() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioWifi {
    #[serde(default)]
    pub permission: WifiPermission,
    pub current: Option<WifiConnection>,
    #[serde(default)]
    pub nearby: Vec<NearbyNetwork>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum WifiPermission {
    #[default]
    Granted,
    Denied,
}

/// A host outside the LAN: it has no MAC and never appears in ARP or discovery.
#[derive(Debug, Deserialize)]
pub(super) struct ScenarioHost {
    pub ip: Ipv4Addr,
    /// Round-trip time in milliseconds; `null` means the host ignores ping.
    pub ping_ms: Option<u64>,
    #[serde(default = "default_ttl")]
    pub ttl: u8,
    #[serde(default)]
    pub open_ports: Vec<u16>,
}
