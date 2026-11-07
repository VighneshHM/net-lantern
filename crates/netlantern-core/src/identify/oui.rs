//! MAC address vendor lookup against the embedded IEEE OUI table.
//!
//! The table is generated from the IEEE MA-L registry by
//! `cargo run -p xtask -- oui <oui.csv>`.

use std::sync::OnceLock;

use netlantern_platform::MacAddr;

static TABLE_TSV: &str = include_str!("../../data/oui.tsv");

fn table() -> &'static [(u32, &'static str)] {
    static TABLE: OnceLock<Vec<(u32, &'static str)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        TABLE_TSV
            .lines()
            .filter_map(|line| {
                let (prefix, name) = line.split_once('\t')?;
                Some((u32::from_str_radix(prefix, 16).ok()?, name))
            })
            .collect()
    })
}

/// Short vendor name for a MAC's OUI, or `None` for private (locally
/// administered) addresses and unknown prefixes.
pub fn vendor(mac: MacAddr) -> Option<&'static str> {
    if mac.is_locally_administered() {
        return None;
    }
    let key = u32::from_be_bytes([0, mac.0[0], mac.0[1], mac.0[2]]);
    let t = table();
    t.binary_search_by_key(&key, |(p, _)| *p)
        .ok()
        .map(|i| t[i].1)
}

#[cfg(test)]
fn oui_table_is_sorted_and_large() -> bool {
    let t = table();
    t.len() > 30_000 && t.windows(2).all(|w| w[0].0 < w[1].0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(s: &str) -> MacAddr {
        s.parse().unwrap()
    }

    #[test]
    fn looks_up_vendors() {
        assert_eq!(vendor(mac("00:11:32:AB:CD:70")), Some("Synology"));
        assert_eq!(vendor(mac("F0:18:98:7C:3D:5E")), Some("Apple"));
        assert_eq!(vendor(mac("24:6F:28:3A:9F:21")), Some("Espressif"));
        assert_eq!(vendor(mac("DA:A1:19:5C:2E:01")), None);
        // A block the IEEE sub-assigns (MA-M/MA-S) names no maker.
        assert_eq!(vendor(mac("B8:4C:87:00:00:01")), None);
        assert!(oui_table_is_sorted_and_large());
    }
}
