//! A simulated network driven by a JSON scenario file, used by tests and
//! `netlantern --demo` in place of the real Windows implementation.

mod scenario;

use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    ArpEntry, HttpResponse, Interface, InterfaceKind, MacAddr, MdnsService, NearbyNetwork,
    NeighborState, NetIo, NetbiosInfo, PingReply, Platform, PlatformError, SsdpReply, TcpProbe,
    WifiConnection, LOCATION_PERMISSION,
};
use scenario::{
    Scenario, ScenarioDevice, ScenarioHost, ScenarioHttp, ScenarioWifi, WifiPermission,
};

pub use scenario::ScenarioError;

/// How long a reverse lookup for an unknown address takes to give up.
const DNS_WAIT: Duration = Duration::from_secs(1);

/// How long a Wi-Fi radio scan takes.
const WIFI_SCAN_WAIT: Duration = Duration::from_secs(1);

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
    wifi: Option<ScenarioWifi>,
    /// Outside hosts, ordered by address; never in ARP or discovery.
    internet: BTreeMap<Ipv4Addr, ScenarioHost>,
    dns_down: Vec<Ipv4Addr>,
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
            wifi: scenario.wifi,
            internet: scenario.internet.into_iter().map(|h| (h.ip, h)).collect(),
            dns_down: scenario.dns_down,
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

    /// The Wi-Fi block, or the error every Wi-Fi call returns without usable data.
    fn wifi_data(&self) -> Result<&ScenarioWifi, PlatformError> {
        let wifi = self
            .wifi
            .as_ref()
            .ok_or_else(|| PlatformError::Unavailable {
                reason: "no Wi-Fi data in this scenario".into(),
            })?;
        match wifi.permission {
            WifiPermission::Granted => Ok(wifi),
            WifiPermission::Denied => Err(PlatformError::PermissionNeeded {
                what: LOCATION_PERMISSION.into(),
            }),
        }
    }

    /// Ping answer (round-trip time and TTL) for a LAN device or outside host.
    fn ping_answer(&self, ip: Ipv4Addr) -> Option<(Duration, u8)> {
        let (ping_ms, ttl) = match self.devices.get(&ip) {
            Some(d) => (d.ping_ms, d.ttl),
            None => {
                let h = self.internet.get(&ip)?;
                (h.ping_ms, h.ttl)
            }
        };
        Some((Duration::from_millis(ping_ms?), ttl))
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
        let answer = self
            .ping_answer(ip)
            .and_then(|(rtt, ttl)| (rtt <= timeout).then_some(PingReply { rtt, ttl }));
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
        let open_ports = match (self.devices.get(&ip), self.internet.get(&ip)) {
            (Some(device), _) => Some(&device.open_ports),
            (None, Some(host)) => Some(&host.open_ports),
            (None, None) => None,
        };
        Ok(match open_ports {
            Some(ports) if ports.contains(&port) => TcpProbe::Open,
            Some(_) => TcpProbe::Closed,
            None => TcpProbe::Filtered,
        })
    }

    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        let iface = self.connected()?;
        if iface.kind != InterfaceKind::Wifi {
            return Ok(None);
        }
        Ok(self.wifi_data()?.current.clone())
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        tokio::time::sleep(WIFI_SCAN_WAIT).await;
        Ok(self.wifi_data()?.nearby.clone())
    }
}

#[async_trait]
impl NetIo for SimNetwork {
    async fn dns_resolve(
        &self,
        server: Ipv4Addr,
        name: &str,
    ) -> Result<Vec<Ipv4Addr>, PlatformError> {
        self.connected()?;
        if self.dns_down.contains(&server) {
            tokio::time::sleep(DNS_WAIT).await;
            return Err(PlatformError::Io {
                message: format!("DNS query to {server} timed out"),
            });
        }
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
        if let Some(message) = &canned.tls_error {
            return Err(PlatformError::TlsCertificate {
                message: message.clone(),
            });
        }
        Ok(HttpResponse {
            status: canned.status,
            location: canned.location.clone(),
            body_prefix: canned.body.clone(),
        })
    }

    async fn reverse_dns(
        &self,
        _server: Ipv4Addr,
        ip: Ipv4Addr,
    ) -> Result<Option<String>, PlatformError> {
        self.connected()?;
        match self.devices.get(&ip) {
            Some(device) => Ok(device.rdns.clone()),
            None => {
                tokio::time::sleep(DNS_WAIT).await;
                Ok(None)
            }
        }
    }

    async fn mdns_browse(
        &self,
        _local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        self.connected()?;
        tokio::time::sleep(window).await;
        Ok(self
            .devices
            .values()
            .flat_map(|device| {
                device.mdns.iter().map(|m| MdnsService {
                    ip: device.ip,
                    service_type: m.service.clone(),
                    instance: m.instance.clone(),
                    hostname: m.hostname.clone(),
                    friendly_name: m.friendly_name.clone(),
                    model: m.model.clone(),
                })
            })
            .collect())
    }

    async fn ssdp_search(
        &self,
        _local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<SsdpReply>, PlatformError> {
        self.connected()?;
        tokio::time::sleep(window).await;
        Ok(self
            .devices
            .values()
            .filter_map(|device| {
                let s = device.ssdp.as_ref()?;
                Some(SsdpReply {
                    ip: device.ip,
                    st: s.st.clone(),
                    server: s.server.clone(),
                    location: s.location.clone(),
                })
            })
            .collect())
    }

    async fn netbios_status(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        self.connected()?;
        match self.devices.get(&ip) {
            Some(device) => Ok(device.netbios.clone()),
            None => {
                tokio::time::sleep(timeout).await;
                Ok(None)
            }
        }
    }
}
