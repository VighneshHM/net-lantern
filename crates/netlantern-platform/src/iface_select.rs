//! Portable interface selection: pure logic over adapter data read by the
//! Windows readers, so it is testable everywhere.

use std::net::Ipv4Addr;

use crate::{Interface, InterfaceKind, MacAddr, NeighborState};

/// `IF_TYPE_IEEE80211`.
const IF_TYPE_WIFI: u32 = 71;
/// `IF_TYPE_ETHERNET_CSMACD`.
const IF_TYPE_ETHERNET: u32 = 6;

/// One IPv4-enabled network adapter as reported by the OS.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AdapterInfo {
    pub if_index: u32,
    /// The IANA `ifType` (71 Wi-Fi, 6 Ethernet, anything else is "other").
    pub if_type: u32,
    pub up: bool,
    /// Unicast IPv4 addresses with their prefix lengths.
    pub ipv4: Vec<(Ipv4Addr, u8)>,
    pub gateways: Vec<Ipv4Addr>,
    pub dns: Vec<Ipv4Addr>,
    /// `None` when the adapter has no 6-byte hardware address.
    pub mac: Option<MacAddr>,
    /// The IPv4 interface metric; lower is preferred.
    pub metric: u32,
}

fn is_apipa(ip: Ipv4Addr) -> bool {
    ip.octets()[..2] == [169, 254]
}

fn kind_of(if_type: u32) -> InterfaceKind {
    match if_type {
        IF_TYPE_WIFI => InterfaceKind::Wifi,
        IF_TYPE_ETHERNET => InterfaceKind::Ethernet,
        _ => InterfaceKind::Other,
    }
}

/// Pick the adapter the laptop is really using for the LAN.
///
/// Candidates must be up, have a MAC, a non-APIPA IPv4 address and a real
/// gateway. Wi-Fi and Ethernet outrank everything else (VPNs, tunnels), then
/// the lowest metric wins, then the lowest interface index. Returns the
/// [`Interface`] and its interface index.
pub fn choose_interface(adapters: &[AdapterInfo]) -> Option<(Interface, u32)> {
    adapters
        .iter()
        .filter_map(|a| {
            let mac = a.mac?;
            let (ip, prefix) = a.ipv4.iter().copied().find(|(ip, _)| !is_apipa(*ip))?;
            let gateway = a.gateways.iter().copied().find(|g| !g.is_unspecified())?;
            let kind = kind_of(a.if_type);
            let rank = (kind == InterfaceKind::Other, a.metric, a.if_index);
            a.up.then(|| {
                let iface = Interface {
                    ip,
                    prefix,
                    gateway,
                    dns_servers: a.dns.clone(),
                    kind,
                    mac,
                };
                (rank, iface, a.if_index)
            })
        })
        .min_by_key(|(rank, _, _)| *rank)
        .map(|(_, iface, idx)| (iface, idx))
}

/// Map a raw `NL_NEIGHBOR_STATE` to a [`NeighborState`]; unknown values give `None`.
pub fn neighbor_state(raw: i32) -> Option<NeighborState> {
    Some(match raw {
        0 => NeighborState::Unreachable,
        1 => NeighborState::Incomplete,
        2 => NeighborState::Probe,
        3 => NeighborState::Delay,
        4 => NeighborState::Stale,
        5 => NeighborState::Reachable,
        6 => NeighborState::Permanent,
        _ => return None,
    })
}

/// The MAC to report for a neighbour-table row, or `None` to skip the row.
///
/// Incomplete and Unreachable rows are kept even without a MAC (they get an
/// all-zero one): they tell the scan the OS already tried that address, so it
/// need not ARP it again. In every other state a missing, odd-sized or
/// all-zero MAC means a broken row, which is skipped.
pub fn neighbor_mac(len: u32, bytes: &[u8], state: NeighborState) -> Option<MacAddr> {
    let mut mac = [0u8; 6];
    if len == 6 && bytes.len() >= 6 {
        mac.copy_from_slice(&bytes[..6]);
    }
    if mac == [0u8; 6] && state.is_present() {
        return None;
    }
    Some(MacAddr(mac))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn adapter(if_type: u32, addr: &str, prefix: u8, gw: Option<&str>, metric: u32) -> AdapterInfo {
        // Distinct index and MAC per adapter, derived from the address.
        let o = ip(addr).octets();
        AdapterInfo {
            if_index: u32::from(o[2]) * 256 + u32::from(o[3]) + u32::from(o[0]),
            if_type,
            up: true,
            ipv4: vec![(ip(addr), prefix)],
            gateways: gw.map(ip).into_iter().collect(),
            dns: vec![],
            mac: Some(MacAddr([0x02, o[0], o[1], o[2], o[3], if_type as u8])),
            metric,
        }
    }

    #[test]
    fn prefers_wifi_with_gateway_over_vpn_and_hyperv() {
        let vpn = adapter(53, "10.8.0.6", 24, Some("10.8.0.1"), 1);
        let vswitch = adapter(6, "172.20.0.1", 20, None, 5);
        let wifi = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 35);
        let (iface, idx) = choose_interface(&[vpn, vswitch, wifi.clone()]).unwrap();
        assert_eq!(
            (iface.ip, iface.prefix, iface.kind, idx),
            (ip("192.168.1.23"), 24, InterfaceKind::Wifi, wifi.if_index)
        );
        assert_eq!(iface.gateway, ip("192.168.1.1"));
    }

    #[test]
    fn skips_apipa_and_down_adapters() {
        let apipa = adapter(6, "169.254.10.2", 16, Some("169.254.0.1"), 1);
        let mut down = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 1);
        down.up = false;
        assert!(choose_interface(&[apipa, down]).is_none());
    }

    #[test]
    fn skips_adapters_without_a_mac_or_with_a_zero_gateway() {
        let mut no_mac = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 1);
        no_mac.mac = None;
        let zero_gw = adapter(6, "192.168.2.23", 24, Some("0.0.0.0"), 1);
        assert!(choose_interface(&[no_mac, zero_gw]).is_none());
    }

    #[test]
    fn falls_back_to_other_when_only_a_vpn_has_a_gateway() {
        let (iface, _) =
            choose_interface(&[adapter(53, "10.8.0.6", 24, Some("10.8.0.1"), 1)]).unwrap();
        assert_eq!(iface.kind, InterfaceKind::Other);
    }

    #[test]
    fn lower_metric_wins_between_physical_adapters() {
        let eth = adapter(6, "192.168.1.50", 24, Some("192.168.1.1"), 25);
        let wifi = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 35);
        let (iface, idx) = choose_interface(&[wifi, eth.clone()]).unwrap();
        assert_eq!((iface.kind, idx), (InterfaceKind::Ethernet, eth.if_index));
    }

    #[test]
    fn lowest_index_breaks_a_metric_tie() {
        let a = adapter(6, "192.168.1.50", 24, Some("192.168.1.1"), 25);
        let mut b = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 25);
        b.if_index = a.if_index + 1;
        let (_, idx) = choose_interface(&[b, a.clone()]).unwrap();
        assert_eq!(idx, a.if_index);
    }

    #[test]
    fn uses_the_first_non_apipa_address() {
        let mut a = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 35);
        a.ipv4 = vec![(ip("169.254.9.9"), 16), (ip("192.168.1.23"), 24)];
        let (iface, _) = choose_interface(&[a]).unwrap();
        assert_eq!((iface.ip, iface.prefix), (ip("192.168.1.23"), 24));
    }

    #[test]
    fn maps_neighbor_states() {
        assert_eq!(neighbor_state(5), Some(NeighborState::Reachable));
        assert_eq!(neighbor_state(1), Some(NeighborState::Incomplete));
        assert_eq!(neighbor_state(6), Some(NeighborState::Permanent));
        assert_eq!(neighbor_state(42), None);
        assert_eq!(neighbor_state(-1), None);
    }

    #[test]
    fn rows_without_a_mac_are_kept_only_when_absent() {
        use NeighborState::*;
        let zero = [0u8; 6];
        let real = [1, 2, 3, 4, 5, 6, 0, 0];
        for state in [Incomplete, Unreachable] {
            assert_eq!(neighbor_mac(6, &zero, state), Some(MacAddr(zero)));
            assert_eq!(neighbor_mac(0, &[], state), Some(MacAddr(zero)));
            assert_eq!(neighbor_mac(8, &real, state), Some(MacAddr(zero)));
        }
        for state in [Reachable, Stale, Delay, Probe, Permanent] {
            assert_eq!(neighbor_mac(6, &zero, state), None);
            assert_eq!(neighbor_mac(0, &[], state), None);
            assert_eq!(
                neighbor_mac(6, &real, state),
                Some(MacAddr([1, 2, 3, 4, 5, 6]))
            );
        }
    }
}
