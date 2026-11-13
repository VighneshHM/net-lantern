//! A rough OS guess from the TTL, open ports and a few mDNS and vendor hints.

use super::classify::Signals;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OsGuess {
    /// e.g. "Windows", "iOS or iPadOS", "Linux, Android or embedded".
    pub name: String,
    /// What it rests on, e.g. "TTL 128 and ports 135, 445".
    pub basis: String,
}

const WINDOWS_PORTS: [u16; 5] = [135, 139, 445, 3389, 5985];

fn guess(name: &str, basis: impl Into<String>) -> Option<OsGuess> {
    Some(OsGuess {
        name: name.into(),
        basis: basis.into(),
    })
}

fn open_windows_ports(s: &Signals) -> Vec<String> {
    WINDOWS_PORTS
        .iter()
        .filter(|p| s.open_ports.contains(p))
        .map(|p| p.to_string())
        .collect()
}

fn announces(s: &Signals, service_type: &str) -> bool {
    s.mdns
        .iter()
        .any(|m| m.service_type.eq_ignore_ascii_case(service_type))
}

fn apple_evidence(s: &Signals) -> Option<String> {
    if s.open_ports.contains(&548) {
        return Some("port 548".into());
    }
    for t in ["_companion-link._tcp", "_airplay._tcp", "_raop._tcp"] {
        if announces(s, t) {
            return Some(t.trim_end_matches("._tcp").into());
        }
    }
    let apple = s
        .vendor
        .as_deref()
        .is_some_and(|v| v.to_ascii_lowercase().contains("apple"));
    apple.then(|| "vendor Apple".into())
}

pub fn os_guess(s: &Signals) -> Option<OsGuess> {
    if s.is_self {
        return guess("Windows", "this laptop");
    }
    let start_ttl = s.ttl.map(|t| match t {
        0..=64 => 64,
        65..=128 => 128,
        _ => 255u16,
    });
    let win = open_windows_ports(s);
    match start_ttl {
        Some(128) if win.is_empty() => guess("Windows", "TTL 128"),
        Some(128) => guess("Windows", format!("TTL 128 and ports {}", win.join(", "))),
        Some(64) => {
            if s.open_ports.contains(&62078) {
                guess("iOS or iPadOS", "TTL 64 and port 62078")
            } else if announces(s, "_apple-mobdev2._tcp") {
                guess("iOS or iPadOS", "TTL 64 and _apple-mobdev2")
            } else if let Some(ev) = apple_evidence(s) {
                guess("macOS or iOS", format!("TTL 64 and {ev}"))
            } else if s.open_ports.contains(&5555) {
                guess("Android", "TTL 64 and port 5555")
            } else {
                guess("Linux, Android or embedded", "TTL 64")
            }
        }
        Some(_) => guess("Network equipment or embedded", "TTL 255"),
        None if s.open_ports.contains(&62078) => guess("iOS or iPadOS", "port 62078"),
        None if win.len() >= 2 => guess("Windows", format!("ports {}", win.join(", "))),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(s: &Signals) -> Option<(String, String)> {
        os_guess(s).map(|o| (o.name, o.basis))
    }
    fn sig(ttl: Option<u8>, ports: &[u16]) -> Signals {
        Signals {
            ttl,
            open_ports: ports.to_vec(),
            ..Default::default()
        }
    }
    fn pair(a: &str, b: &str) -> Option<(String, String)> {
        Some((a.into(), b.into()))
    }

    #[test]
    fn rules() {
        assert_eq!(
            g(&sig(Some(128), &[135, 445])),
            pair("Windows", "TTL 128 and ports 135, 445")
        );
        assert_eq!(g(&sig(Some(100), &[])), pair("Windows", "TTL 128"));
        assert_eq!(
            g(&sig(Some(57), &[62078])),
            pair("iOS or iPadOS", "TTL 64 and port 62078")
        );
        let mut a = sig(Some(64), &[]);
        a.vendor = Some("Apple".into());
        assert_eq!(g(&a), pair("macOS or iOS", "TTL 64 and vendor Apple"));
        assert_eq!(
            g(&sig(Some(64), &[5555])),
            pair("Android", "TTL 64 and port 5555")
        );
        assert_eq!(
            g(&sig(Some(63), &[])),
            pair("Linux, Android or embedded", "TTL 64")
        );
        assert_eq!(
            g(&sig(Some(250), &[])),
            pair("Network equipment or embedded", "TTL 255")
        );
        assert_eq!(g(&sig(None, &[62078])), pair("iOS or iPadOS", "port 62078"));
        assert_eq!(
            g(&sig(None, &[445, 3389])),
            pair("Windows", "ports 445, 3389")
        );
        assert_eq!(g(&sig(None, &[445])), None);
        let me = Signals {
            is_self: true,
            ..Default::default()
        };
        assert_eq!(g(&me), pair("Windows", "this laptop"));
    }
}
