use std::net::Ipv4Addr;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::MacAddr;

/// The kind of network adapter an [`Interface`] runs over.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceKind {
    Wifi,
    Ethernet,
    Other,
}

/// The laptop's active network interface.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Interface {
    pub ip: Ipv4Addr,
    /// Subnet prefix length (`24` for a `/24`).
    pub prefix: u8,
    pub gateway: Ipv4Addr,
    pub dns_servers: Vec<Ipv4Addr>,
    pub kind: InterfaceKind,
    pub mac: MacAddr,
}

/// A successful ICMP echo reply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PingReply {
    pub rtt: Duration,
    pub ttl: u8,
}

/// Windows neighbor-cache states for an ARP entry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NeighborState {
    Reachable,
    Stale,
    Delay,
    Probe,
    Incomplete,
    Unreachable,
    Permanent,
}

impl NeighborState {
    /// Whether the neighbor is believed to be on the network: false only for
    /// `Incomplete` (resolution never finished) and `Unreachable`.
    pub fn is_present(&self) -> bool {
        !matches!(self, NeighborState::Incomplete | NeighborState::Unreachable)
    }
}

/// One row of the OS ARP/neighbor table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ArpEntry {
    pub ip: Ipv4Addr,
    pub mac: MacAddr,
    pub state: NeighborState,
}

/// Outcome of a TCP connect probe.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TcpProbe {
    /// The connection was accepted.
    Open,
    /// The host actively refused the connection.
    Closed,
    /// No answer at all (dropped by a firewall, or no such host).
    Filtered,
}

#[derive(Clone, PartialEq, Eq, Debug, Error)]
pub enum PlatformError {
    #[error("not connected to a network")]
    NotConnected,
    #[error("permission needed: {what}")]
    PermissionNeeded { what: String },
    #[error("unavailable: {reason}")]
    Unavailable { reason: String },
    #[error("I/O error: {message}")]
    Io { message: String },
}

/// The OS seam for layer 2-4 probing. The Windows implementation arrives in
/// phase 1; [`crate::sim::SimNetwork`] stands in for tests and `--demo`.
#[async_trait]
pub trait Platform: Send + Sync {
    /// The interface the laptop is currently using, or `NotConnected`.
    async fn active_interface(&self) -> Result<Interface, PlatformError>;

    /// ICMP echo. `Ok(None)` means no reply within `timeout`.
    async fn ping(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<PingReply>, PlatformError>;

    /// Actively resolve `ip` to a MAC address. `Ok(None)` means nobody answered.
    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<MacAddr>, PlatformError>;

    /// A snapshot of the OS neighbor table.
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError>;

    /// TCP connect probe against `ip:port`.
    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpProbe, PlatformError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_incomplete_and_unreachable_are_absent() {
        use NeighborState::*;
        for state in [Reachable, Stale, Delay, Probe, Permanent] {
            assert!(state.is_present(), "{state:?} should be present");
        }
        for state in [Incomplete, Unreachable] {
            assert!(!state.is_present(), "{state:?} should be absent");
        }
    }

    #[test]
    fn interface_serializes_with_the_api_field_names() {
        let iface = Interface {
            ip: "192.168.1.23".parse().unwrap(),
            prefix: 24,
            gateway: "192.168.1.1".parse().unwrap(),
            dns_servers: vec!["192.168.1.1".parse().unwrap()],
            kind: InterfaceKind::Wifi,
            mac: "F4:8E:38:9A:20:7C".parse().unwrap(),
        };
        let json = serde_json::to_value(&iface).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "ip": "192.168.1.23",
                "prefix": 24,
                "gateway": "192.168.1.1",
                "dns_servers": ["192.168.1.1"],
                "kind": "wifi",
                "mac": "F4:8E:38:9A:20:7C",
            })
        );
        assert_eq!(serde_json::from_value::<Interface>(json).unwrap(), iface);
    }

    #[test]
    fn enums_serialize_as_snake_case() {
        fn json<T: Serialize>(value: T) -> String {
            serde_json::to_string(&value).unwrap()
        }
        assert_eq!(json(InterfaceKind::Ethernet), "\"ethernet\"");
        assert_eq!(json(InterfaceKind::Other), "\"other\"");
        assert_eq!(json(NeighborState::Incomplete), "\"incomplete\"");
        assert_eq!(json(TcpProbe::Filtered), "\"filtered\"");
    }

    #[test]
    fn errors_have_readable_messages() {
        assert_eq!(
            PlatformError::NotConnected.to_string(),
            "not connected to a network"
        );
        assert_eq!(
            PlatformError::PermissionNeeded {
                what: "administrator rights".into()
            }
            .to_string(),
            "permission needed: administrator rights"
        );
    }
}
