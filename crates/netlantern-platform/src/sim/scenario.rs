use std::collections::HashMap;
use std::net::Ipv4Addr;

use serde::Deserialize;
use thiserror::Error;

use crate::{InterfaceKind, MacAddr};

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
}

#[derive(Debug, Deserialize)]
pub(super) struct ScenarioHttp {
    pub status: u16,
    #[serde(default)]
    pub body: String,
}

fn default_ttl() -> u8 {
    64
}

fn default_arp() -> bool {
    true
}
