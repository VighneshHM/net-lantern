//! A simulated network driven by a JSON scenario file, used by tests and
//! `netlantern --demo` in place of the real Windows implementation.

mod scenario;

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    ArpEntry, HttpResponse, Interface, MacAddr, NeighborState, NetIo, PingReply, Platform,
    PlatformError, TcpProbe,
};
use scenario::{Scenario, ScenarioDevice, ScenarioHttp};

pub use scenario::ScenarioError;

/// An in-memory network implementing both [`Platform`] and [`NetIo`].
///
/// Delays use `tokio::time::sleep`, so tests running with a paused clock
/// (`start_paused = true`) get instant, deterministic "network latency".
#[derive(Debug)]
pub struct SimNetwork {
    name: String,
    interface: Option<Interface>,
    /// Ordered by address so the ARP table is deterministic.
    devices: BTreeMap<Ipv4Addr, ScenarioDevice>,
    dns: HashMap<String, Vec<Ipv4Addr>>,
    http: HashMap<String, ScenarioHttp>,
}

impl SimNetwork {
    /// Build a simulated network from scenario JSON text.
    pub fn from_json(json: &str) -> Result<SimNetwork, ScenarioError> {
        let scenario: Scenario = serde_json::from_str(json)?;
        Ok(SimNetwork {
            name: scenario.name,
            interface: scenario.interface.map(|i| Interface {
                ip: i.ip,
                prefix: i.prefix,
                gateway: i.gateway,
                dns_servers: i.dns,
                kind: i.kind,
                mac: i.mac,
            }),
            devices: scenario.devices.into_iter().map(|d| (d.ip, d)).collect(),
            dns: scenario.dns,
            http: scenario.http,
        })
    }

    /// Read and parse a scenario file.
    pub fn load(path: impl AsRef<Path>) -> Result<SimNetwork, ScenarioError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    /// The scenario's name (`"home"`, `"offline"`, ...).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Every `Platform` call fails when the scenario has no interface.
    fn connected(&self) -> Result<&Interface, PlatformError> {
        self.interface.as_ref().ok_or(PlatformError::NotConnected)
    }
}

#[async_trait]
impl Platform for SimNetwork {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.connected().cloned()
    }

    async fn ping(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<PingReply>, PlatformError> {
        self.connected()?;
        let answer = self.devices.get(&ip).and_then(|device| {
            let rtt = Duration::from_millis(device.ping_ms?);
            (rtt <= timeout).then_some(PingReply {
                rtt,
                ttl: device.ttl,
            })
        });
        match answer {
            Some(reply) => {
                tokio::time::sleep(reply.rtt).await;
                Ok(Some(reply))
            }
            None => {
                tokio::time::sleep(timeout).await;
                Ok(None)
            }
        }
    }

    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        _timeout: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.connected()?;
        Ok(self
            .devices
            .get(&ip)
            .filter(|device| device.arp)
            .map(|device| device.mac))
    }

    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.connected()?;
        Ok(self
            .devices
            .values()
            .filter(|device| device.arp)
            .map(|device| ArpEntry {
                ip: device.ip,
                mac: device.mac,
                state: NeighborState::Reachable,
            })
            .collect())
    }

    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        _timeout: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        self.connected()?;
        Ok(match self.devices.get(&ip) {
            Some(device) if device.open_ports.contains(&port) => TcpProbe::Open,
            Some(_) => TcpProbe::Closed,
            None => TcpProbe::Filtered,
        })
    }
}

#[async_trait]
impl NetIo for SimNetwork {
    async fn dns_resolve(
        &self,
        _server: Ipv4Addr,
        name: &str,
    ) -> Result<Vec<Ipv4Addr>, PlatformError> {
        self.dns
            .get(name)
            .cloned()
            .ok_or_else(|| PlatformError::Io {
                message: format!("DNS name not found: {name}"),
            })
    }

    async fn http_get(
        &self,
        url: &str,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PlatformError> {
        let canned = self.http.get(url).ok_or_else(|| PlatformError::Io {
            message: format!("no route to URL: {url}"),
        })?;
        Ok(HttpResponse {
            status: canned.status,
            body_prefix: canned.body.clone(),
        })
    }
}
