use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Mutex, PoisonError};

use netlantern_platform::MacAddr;

use crate::model::DeviceSummary;

/// What identifies a device: its MAC address when known, else its IP address.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DeviceKey {
    Mac(MacAddr),
    Ip(Ipv4Addr),
}

impl DeviceKey {
    fn of(device: &DeviceSummary) -> Self {
        match device.mac {
            Some(mac) => DeviceKey::Mac(mac),
            None => DeviceKey::Ip(device.ip),
        }
    }
}

/// The devices seen so far, shared between scans and the API.
#[derive(Default)]
pub struct DeviceStore {
    devices: Mutex<HashMap<DeviceKey, DeviceSummary>>,
}

impl DeviceStore {
    /// Adds `device`, replacing any earlier record of the same device.
    pub fn upsert(&self, device: DeviceSummary) {
        // A panic cannot leave the map half-updated, so a poisoned lock is safe to reuse.
        let mut devices = self.devices.lock().unwrap_or_else(PoisonError::into_inner);
        devices.insert(DeviceKey::of(&device), device);
    }

    /// Every device, sorted by IP address.
    pub fn list(&self) -> Vec<DeviceSummary> {
        let devices = self.devices.lock().unwrap_or_else(PoisonError::into_inner);
        let mut list: Vec<DeviceSummary> = devices.values().cloned().collect();
        list.sort_by_key(|device| device.ip);
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(ip: &str, mac: Option<[u8; 6]>) -> DeviceSummary {
        DeviceSummary {
            ip: ip.parse().unwrap(),
            mac: mac.map(MacAddr),
            private_mac: false,
            answers_ping: true,
            rtt_ms: Some(5),
            is_self: false,
            is_gateway: false,
        }
    }

    const MAC_A: [u8; 6] = [0x50, 0xC7, 0xBF, 0x3A, 0x11, 0x02];
    const MAC_B: [u8; 6] = [0x3C, 0x28, 0x6D, 0x91, 0x4E, 0xB0];

    #[test]
    fn list_is_sorted_by_numeric_ip() {
        let store = DeviceStore::default();
        store.upsert(device("192.168.1.102", Some(MAC_A)));
        store.upsert(device("192.168.1.9", Some(MAC_B)));
        store.upsert(device("192.168.1.30", None));
        let ips: Vec<String> = store.list().iter().map(|d| d.ip.to_string()).collect();
        assert_eq!(ips, ["192.168.1.9", "192.168.1.30", "192.168.1.102"]);
    }

    #[test]
    fn a_device_is_keyed_by_mac_so_it_follows_its_address() {
        let store = DeviceStore::default();
        store.upsert(device("192.168.1.50", Some(MAC_A)));
        store.upsert(device("192.168.1.51", Some(MAC_A)));
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].ip, "192.168.1.51".parse::<Ipv4Addr>().unwrap());
    }

    #[test]
    fn devices_without_a_mac_are_keyed_by_ip() {
        let store = DeviceStore::default();
        store.upsert(device("192.168.1.50", None));
        let mut again = device("192.168.1.50", None);
        again.rtt_ms = Some(9);
        store.upsert(again);
        store.upsert(device("192.168.1.51", None));
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].rtt_ms, Some(9));
    }

    #[test]
    fn upsert_replaces_the_previous_record() {
        let store = DeviceStore::default();
        store.upsert(device("192.168.1.50", Some(MAC_A)));
        let mut gone_quiet = device("192.168.1.50", Some(MAC_A));
        gone_quiet.answers_ping = false;
        gone_quiet.rtt_ms = None;
        store.upsert(gone_quiet.clone());
        assert_eq!(store.list(), vec![gone_quiet]);
    }
}
