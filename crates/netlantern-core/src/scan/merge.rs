//! Pure helpers that turn ping replies, ARP entries, open ports and discovery
//! signals into the sightings a scan records.

use std::collections::HashMap;
use std::net::Ipv4Addr;

use netlantern_platform::{ArpEntry, Interface, MacAddr, PingReply};

use crate::identify::classify::Signals;
use crate::model::Sighting;

/// The swept addresses that are present, in address order: they replied to a
/// ping or have a live ARP entry. Entries outside `targets` (broadcast,
/// multicast, other subnets) are ignored.
pub(super) fn present_addresses(
    targets: &[Ipv4Addr],
    replies: &HashMap<Ipv4Addr, PingReply>,
    arp: &HashMap<Ipv4Addr, ArpEntry>,
) -> Vec<Ipv4Addr> {
    targets
        .iter()
        .copied()
        .filter(|ip| replies.contains_key(ip) || live_mac(arp, *ip).is_some())
        .collect()
}

/// The MAC of a live ARP entry for `ip`.
pub(super) fn live_mac(arp: &HashMap<Ipv4Addr, ArpEntry>, ip: Ipv4Addr) -> Option<MacAddr> {
    arp.get(&ip)
        .filter(|entry| entry.state.is_present())
        .map(|entry| entry.mac)
}

/// Adds `entry` to `arp` unless a live entry is already there: a dead entry
/// (incomplete, unreachable) is replaced, a live one is kept.
pub(super) fn keep_live(arp: &mut HashMap<Ipv4Addr, ArpEntry>, entry: ArpEntry) {
    if !entry.state.is_present() {
        arp.entry(entry.ip).or_insert(entry);
        return;
    }
    match arp.get(&entry.ip) {
        Some(existing) if existing.state.is_present() => {}
        _ => {
            arp.insert(entry.ip, entry);
        }
    }
}

/// What a scan saw of one address. This laptop's MAC comes from its interface,
/// everyone else's from ARP. The flags, TTL and ports are copied into the
/// signals so the classifier sees them.
pub(super) fn sighting(
    iface: &Interface,
    ip: Ipv4Addr,
    reply: Option<&PingReply>,
    arp: &HashMap<Ipv4Addr, ArpEntry>,
    open_ports: Vec<u16>,
    mut signals: Signals,
) -> Sighting {
    let is_self = ip == iface.ip;
    let is_gateway = ip == iface.gateway;
    let mac = if is_self {
        Some(iface.mac)
    } else {
        live_mac(arp, ip)
    };
    let ttl = reply.map(|r| r.ttl);
    signals.ttl = ttl;
    signals.open_ports = open_ports.clone();
    signals.private_mac = mac.is_some_and(|m| m.is_locally_administered());
    signals.is_gateway = is_gateway;
    signals.is_self = is_self;
    Sighting {
        ip,
        mac,
        answers_ping: reply.is_some(),
        rtt_ms: reply.map(|r| u32::try_from(r.rtt.as_millis()).unwrap_or(u32::MAX)),
        ttl,
        is_self,
        is_gateway,
        open_ports,
        signals,
    }
}

/// One sighting per present address, in address order, combining everything
/// the scan learned.
pub(super) fn merge_sightings(
    iface: &Interface,
    present: &[Ipv4Addr],
    replies: &HashMap<Ipv4Addr, PingReply>,
    arp: &HashMap<Ipv4Addr, ArpEntry>,
    mut ports: HashMap<Ipv4Addr, Vec<u16>>,
    mut signals: HashMap<Ipv4Addr, Signals>,
) -> Vec<Sighting> {
    present
        .iter()
        .map(|&ip| {
            sighting(
                iface,
                ip,
                replies.get(&ip),
                arp,
                ports.remove(&ip).unwrap_or_default(),
                signals.remove(&ip).unwrap_or_default(),
            )
        })
        .collect()
}

/// True when, apart from this laptop, only the gateway is present: the network
/// likely keeps its clients apart (guest Wi-Fi, client isolation).
pub(super) fn is_isolated(iface: &Interface, present: &[Ipv4Addr]) -> bool {
    let mut others = present.iter().filter(|ip| **ip != iface.ip);
    others.next() == Some(&iface.gateway) && others.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use netlantern_platform::{InterfaceKind, NeighborState};

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn iface() -> Interface {
        Interface {
            ip: ip("10.0.0.5"),
            prefix: 24,
            gateway: ip("10.0.0.1"),
            dns_servers: vec![],
            kind: InterfaceKind::Wifi,
            mac: MacAddr([0xF4, 0x8E, 0x38, 0x9A, 0x20, 0x7C]),
        }
    }

    fn reply() -> PingReply {
        PingReply {
            rtt: Duration::from_millis(3),
            ttl: 64,
        }
    }

    fn entry(addr: &str, state: NeighborState) -> ArpEntry {
        ArpEntry {
            ip: ip(addr),
            mac: MacAddr([0x50, 0xC7, 0xBF, 0x3A, 0x11, 0x02]),
            state,
        }
    }

    fn neighbor(addr: &str, state: NeighborState) -> (Ipv4Addr, ArpEntry) {
        (ip(addr), entry(addr, state))
    }

    fn merge(
        targets: &[Ipv4Addr],
        replies: &HashMap<Ipv4Addr, PingReply>,
        arp: &HashMap<Ipv4Addr, ArpEntry>,
    ) -> Vec<Sighting> {
        let present = present_addresses(targets, replies, arp);
        merge_sightings(
            &iface(),
            &present,
            replies,
            arp,
            HashMap::new(),
            HashMap::new(),
        )
    }

    #[test]
    fn neighbors_that_are_not_present_add_no_device_and_no_mac() {
        let targets = [ip("10.0.0.2"), ip("10.0.0.3"), ip("10.0.0.4")];
        // .2 is stale (still believed present); .3 answered ping but its ARP
        // entry never completed; .4 is marked unreachable and did not answer.
        let arp = HashMap::from([
            neighbor("10.0.0.2", NeighborState::Stale),
            neighbor("10.0.0.3", NeighborState::Incomplete),
            neighbor("10.0.0.4", NeighborState::Unreachable),
        ]);
        let replies = HashMap::from([(ip("10.0.0.3"), reply())]);

        let sightings = merge(&targets, &replies, &arp);

        let summary: Vec<_> = sightings
            .iter()
            .map(|d| (d.ip, d.mac.is_some(), d.answers_ping))
            .collect();
        assert_eq!(
            summary,
            [(ip("10.0.0.2"), true, false), (ip("10.0.0.3"), false, true)]
        );
    }

    #[test]
    fn arp_entries_outside_the_swept_addresses_are_ignored() {
        let targets = [ip("10.0.0.2")];
        let arp = HashMap::from([
            neighbor("10.0.0.2", NeighborState::Reachable),
            neighbor("10.0.0.255", NeighborState::Permanent),
            neighbor("224.0.0.22", NeighborState::Permanent),
        ]);

        let sightings = merge(&targets, &HashMap::new(), &arp);

        assert_eq!(sightings.len(), 1);
        assert_eq!(sightings[0].ip, ip("10.0.0.2"));
    }

    #[test]
    fn this_laptop_uses_its_interface_mac() {
        let targets = [ip("10.0.0.5")];
        let replies = HashMap::from([(ip("10.0.0.5"), reply())]);

        let sightings = merge(&targets, &replies, &HashMap::new());

        assert_eq!(sightings[0].mac, Some(iface().mac));
        assert!(sightings[0].is_self && !sightings[0].is_gateway);
        assert!(sightings[0].signals.is_self);
    }

    #[test]
    fn sub_millisecond_rtts_round_down_to_zero() {
        let fast = PingReply {
            rtt: Duration::from_micros(400),
            ttl: 128,
        };
        let replies = HashMap::from([(ip("10.0.0.2"), fast)]);

        let sightings = merge(&[ip("10.0.0.2")], &replies, &HashMap::new());

        assert_eq!(sightings[0].rtt_ms, Some(0));
        assert_eq!(sightings[0].ttl, Some(128));
    }

    #[test]
    fn ports_signals_and_flags_reach_the_classifier() {
        let arp = HashMap::from([neighbor("10.0.0.1", NeighborState::Reachable)]);
        let replies = HashMap::from([(ip("10.0.0.1"), reply())]);
        let signals = Signals {
            rdns: Some("router.lan".into()),
            ..Signals::default()
        };
        let sightings = merge_sightings(
            &iface(),
            &[ip("10.0.0.1")],
            &replies,
            &arp,
            HashMap::from([(ip("10.0.0.1"), vec![80, 443])]),
            HashMap::from([(ip("10.0.0.1"), signals)]),
        );
        let s = &sightings[0];
        assert_eq!(s.open_ports, vec![80, 443]);
        assert_eq!(s.signals.open_ports, vec![80, 443]);
        assert_eq!(s.signals.rdns.as_deref(), Some("router.lan"));
        assert_eq!(s.signals.ttl, Some(64));
        assert!(s.is_gateway && s.signals.is_gateway && !s.signals.private_mac);
    }

    #[test]
    fn a_private_mac_is_flagged() {
        let mut private = entry("10.0.0.7", NeighborState::Reachable);
        private.mac = MacAddr([0xDA, 0xA1, 0x19, 0x5C, 0x2E, 0x01]);
        let arp = HashMap::from([(private.ip, private)]);
        let sightings = merge(&[ip("10.0.0.7")], &HashMap::new(), &arp);
        assert!(sightings[0].signals.private_mac);
    }

    #[test]
    fn keep_live_never_replaces_a_live_entry() {
        let mut arp = HashMap::from([neighbor("10.0.0.2", NeighborState::Unreachable)]);
        keep_live(&mut arp, entry("10.0.0.2", NeighborState::Reachable));
        assert_eq!(arp[&ip("10.0.0.2")].state, NeighborState::Reachable);
        keep_live(&mut arp, entry("10.0.0.2", NeighborState::Incomplete));
        keep_live(&mut arp, entry("10.0.0.2", NeighborState::Stale));
        assert_eq!(arp[&ip("10.0.0.2")].state, NeighborState::Reachable);
        keep_live(&mut arp, entry("10.0.0.3", NeighborState::Incomplete));
        assert_eq!(arp[&ip("10.0.0.3")].state, NeighborState::Incomplete);
    }

    #[test]
    fn isolation_means_only_the_gateway_besides_this_laptop() {
        let i = iface();
        assert!(is_isolated(&i, &[ip("10.0.0.1"), ip("10.0.0.5")]));
        assert!(is_isolated(&i, &[ip("10.0.0.1")]));
        assert!(!is_isolated(&i, &[ip("10.0.0.5")]));
        assert!(!is_isolated(&i, &[]));
        assert!(!is_isolated(
            &i,
            &[ip("10.0.0.1"), ip("10.0.0.5"), ip("10.0.0.9")]
        ));
    }
}
