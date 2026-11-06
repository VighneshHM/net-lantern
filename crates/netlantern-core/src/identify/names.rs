//! Announced names: the best human name a device gave for itself.

use std::net::Ipv4Addr;

use super::classify::Signals;

const SUFFIXES: [&str; 6] = [
    ".local",
    ".lan",
    ".home",
    ".localdomain",
    ".home.arpa",
    ".internal",
];

/// Turn a DNS-style name into a short device name, or reject it when it is
/// an ISP/FQDN name or just encodes the device's own address.
pub fn clean_name(raw: &str, ip: Ipv4Addr) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('.').trim();
    let lower = trimmed.to_ascii_lowercase();
    let name = SUFFIXES
        .iter()
        .find(|suf| lower.ends_with(**suf))
        .map_or(trimmed, |suf| &trimmed[..trimmed.len() - suf.len()]);
    if name.is_empty() || name.contains('.') {
        return None;
    }
    let [a, b, c, d] = ip.octets();
    let dashed = format!("{a}-{b}-{c}-{d}");
    if name.contains(&dashed) {
        return None;
    }
    Some(name.to_string())
}

/// True for machine-generated names: a run of 8 or more hex digits
/// (dashes ignored), which also covers UUIDs.
pub fn looks_generated(name: &str) -> bool {
    let mut run = 0;
    for ch in name.chars() {
        if ch == '-' {
            continue;
        }
        if ch.is_ascii_hexdigit() {
            run += 1;
            if run >= 8 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fn nonempty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The best name the device announced for itself, in priority order: mDNS
/// friendly name, mDNS host name, mDNS instance, NetBIOS, UPnP, reverse DNS.
pub fn announced_name(s: &Signals, ip: Ipv4Addr) -> Option<String> {
    let usable = |n: String| (!looks_generated(&n)).then_some(n);
    s.mdns
        .iter()
        .find_map(|m| m.friendly_name.as_deref().and_then(nonempty))
        .or_else(|| {
            s.mdns.iter().find_map(|m| {
                m.hostname
                    .as_deref()
                    .and_then(|h| clean_name(h, ip))
                    .and_then(usable)
            })
        })
        .or_else(|| {
            s.mdns
                .iter()
                .find_map(|m| nonempty(&m.instance).and_then(usable))
        })
        .or_else(|| s.netbios.as_ref().and_then(|n| nonempty(&n.name)))
        .or_else(|| {
            s.ssdp.iter().find_map(|(_, d)| {
                d.as_ref()
                    .and_then(|d| d.friendly_name.as_deref())
                    .and_then(nonempty)
            })
        })
        .or_else(|| {
            s.rdns
                .as_deref()
                .and_then(|r| clean_name(r, ip))
                .and_then(usable)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identify::upnp::UpnpDescription;
    use netlantern_platform::{MdnsService, NetbiosInfo, SsdpReply};

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn svc(instance: &str, hostname: Option<&str>, friendly: Option<&str>) -> MdnsService {
        MdnsService {
            ip: ip("192.168.1.9"),
            service_type: "_http._tcp".into(),
            instance: instance.into(),
            hostname: hostname.map(Into::into),
            friendly_name: friendly.map(Into::into),
            model: None,
        }
    }

    #[test]
    fn clean_name_rules() {
        let ip = ip("192.168.1.61");
        assert_eq!(clean_name("echo-dot.lan.", ip).as_deref(), Some("echo-dot"));
        assert_eq!(
            clean_name("DiskStation.local", ip).as_deref(),
            Some("DiskStation")
        );
        assert_eq!(clean_name("192-168-1-61.isp.example.net", ip), None);
        assert_eq!(clean_name("host.corp.example.com", ip), None);
        assert_eq!(clean_name("  ", ip), None);
        assert_eq!(clean_name(".local", ip), None);
        assert_eq!(clean_name("pc.home.arpa", ip).as_deref(), Some("pc"));
        assert_eq!(clean_name("TV.LOCAL", ip).as_deref(), Some("TV"));
        assert_eq!(clean_name("host-192-168-1-61.lan", ip), None);
        assert_eq!(clean_name("192.168.1.61", ip), None);
    }

    #[test]
    fn generated_names() {
        assert!(looks_generated("3c1e9a7b-1f"));
        assert!(looks_generated("4f6c2a1e-9b7d-4c1a-8e2f-0a1b2c3d4e5f"));
        assert!(!looks_generated("Kitchen speaker"));
        assert!(!looks_generated("Pixel-7"));
        assert!(!looks_generated("Hue Bridge - 1A2B3C"));
    }

    #[test]
    fn announced_name_priority() {
        let a = ip("192.168.1.9");
        let mut s = Signals {
            mdns: vec![svc("Inst", Some("host.local"), Some("Kitchen speaker"))],
            netbios: Some(NetbiosInfo {
                name: "NB".into(),
                workgroup: None,
            }),
            rdns: Some("rdns.lan".into()),
            ..Default::default()
        };
        assert_eq!(announced_name(&s, a).as_deref(), Some("Kitchen speaker"));

        s.mdns = vec![svc("Inst", Some("3c1e9a7b-1f.local"), None)];
        assert_eq!(announced_name(&s, a).as_deref(), Some("Inst"));

        s.mdns = vec![svc("3c1e9a7b-1f", Some("3c1e9a7b-1f.local"), None)];
        assert_eq!(announced_name(&s, a).as_deref(), Some("NB"));

        s.netbios = None;
        assert_eq!(announced_name(&s, a).as_deref(), Some("rdns"));

        s.ssdp = vec![(
            SsdpReply {
                ip: a,
                st: "x".into(),
                server: None,
                location: None,
            },
            Some(UpnpDescription {
                friendly_name: Some(" [TV] Living Room ".into()),
                ..Default::default()
            }),
        )];
        assert_eq!(announced_name(&s, a).as_deref(), Some("[TV] Living Room"));

        s.ssdp.clear();
        s.rdns = Some("192-168-1-9.isp.net".into());
        assert_eq!(announced_name(&s, a), None);
    }

    #[test]
    fn mdns_hostname_beats_instance() {
        let a = ip("192.168.1.9");
        let s = Signals {
            mdns: vec![svc("Inst", Some("iPhone.local"), None)],
            ..Default::default()
        };
        assert_eq!(announced_name(&s, a).as_deref(), Some("iPhone"));
    }
}
