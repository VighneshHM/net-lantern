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
    Scenario, ScenarioDevice, ScenarioHost, ScenarioHttp, ScenarioWifi, TimelineEntry,
    WifiPermission,
};
use tokio::time::Instant;

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
    timeline: Vec<TimelineEntry>,
    /// Construction time; scenario seconds are measured from here.
    started: Instant,
    /// Scenario seconds per real second.
    time_scale: u32,
}

/// A LAN device as it looks at one moment, after the timeline is applied.
struct EffectiveDevice<'a> {
    device: &'a ScenarioDevice,
    ping_ms: Option<u64>,
    arp: bool,
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
            timeline: scenario.timeline,
            started: Instant::now(),
            time_scale: 1,
        })
    }

    /// Run the timeline `factor` times faster than real time (default 1).
    pub fn with_time_scale(mut self, factor: u32) -> SimNetwork {
        self.time_scale = factor;
        self
    }

    /// Whole scenario seconds since construction.
    fn elapsed_s(&self) -> u64 {
        let ms = self.started.elapsed().as_millis() * u128::from(self.time_scale);
        (ms / 1000) as u64
    }

    /// The device at `ip` as it is at scenario second `t`, or `None` when absent.
    fn effective(&self, ip: Ipv4Addr, t: u64) -> Option<EffectiveDevice<'_>> {
        let device = self.devices.get(&ip)?;
        let mut e = EffectiveDevice {
            device,
            ping_ms: device.ping_ms,
            arp: device.arp,
        };
        let mut present = device.present;
        for entry in self
            .timeline
            .iter()
            .filter(|x| x.ip == ip && x.active_at(t))
        {
            if let Some(p) = entry.set.present {
                present = p;
            }
            if let Some(p) = entry.set.ping_ms {
                e.ping_ms = p;
            }
            if let Some(a) = entry.set.arp {
                e.arp = a;
            }
        }
        present.then_some(e)
    }

    /// Every present LAN device at the current scenario time, by address.
    fn present_devices(&self) -> Vec<EffectiveDevice<'_>> {
        let t = self.elapsed_s();
        self.devices
            .keys()
            .filter_map(|ip| self.effective(*ip, t))
            .collect()
    }

    /// True when the gateway is gone, so nothing outside the LAN answers.
    fn gateway_absent(&self) -> bool {
        let Some(iface) = &self.interface else {
            return false;
        };
        match self.effective(iface.gateway, self.elapsed_s()) {
            None => self.devices.contains_key(&iface.gateway),
            Some(g) => !g.arp && g.ping_ms.is_none(),
        }
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
        let (ping_ms, ttl) = if self.devices.contains_key(&ip) {
            let d = self.effective(ip, self.elapsed_s())?;
            (d.ping_ms, d.device.ttl)
        } else {
            if self.gateway_absent() {
                return None;
            }
            let h = self.internet.get(&ip)?;
            (h.ping_ms, h.ttl)
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
            .effective(ip, self.elapsed_s())
            .filter(|e| e.arp)
            .map(|e| e.device.mac))
    }

    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.connected()?;
        Ok(self
            .present_devices()
            .into_iter()
            .filter(|e| e.arp)
            .map(|e| ArpEntry {
                ip: e.device.ip,
                mac: e.device.mac,
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
        let open_ports = if self.devices.contains_key(&ip) {
            self.effective(ip, self.elapsed_s())
                .map(|e| &e.device.open_ports)
        } else if self.gateway_absent() {
            None
        } else {
            self.internet.get(&ip).map(|h| &h.open_ports)
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
        if self.dns_down.contains(&server) || self.gateway_absent() {
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
        if self.gateway_absent() {
            return Err(PlatformError::Io {
                message: "no route to the internet".into(),
            });
        }
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
        match self.effective(ip, self.elapsed_s()) {
            Some(e) => Ok(e.device.rdns.clone()),
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
            .present_devices()
            .into_iter()
            .map(|e| e.device)
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
            .present_devices()
            .into_iter()
            .map(|e| e.device)
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
        match self.effective(ip, self.elapsed_s()) {
            Some(e) => Ok(e.device.netbios.clone()),
            None => {
                tokio::time::sleep(timeout).await;
                Ok(None)
            }
        }
    }
}
