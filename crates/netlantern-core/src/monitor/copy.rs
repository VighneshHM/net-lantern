//! The exact alert titles and details, and the labels they use.

use std::net::Ipv4Addr;

use crate::model::DeviceType;
use crate::monitor::Target;

pub const ROUTER_DOWN_TITLE: &str = "Your router isn't answering";
pub const INTERNET_DOWN_TITLE: &str = "The internet is down";
pub const LATENCY_HIGH_TITLE: &str = "Internet latency is high";

/// "Router", "Laptop", … or "unknown type".
pub fn type_label(t: DeviceType) -> &'static str {
    match t {
        DeviceType::Router => "Router",
        DeviceType::Laptop => "Laptop",
        DeviceType::Phone => "Phone",
        DeviceType::Tablet => "Tablet",
        DeviceType::Printer => "Printer",
        DeviceType::Tv => "TV",
        DeviceType::Speaker => "Speaker",
        DeviceType::Camera => "Camera",
        DeviceType::Nas => "NAS",
        DeviceType::Console => "Console",
        DeviceType::Iot => "IoT",
        DeviceType::Unknown => "unknown type",
    }
}

/// The vendor, "private address" for a private MAC, or "unknown vendor".
pub fn vendor_label(vendor: Option<&str>, private_mac: bool) -> &str {
    match vendor.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => v,
        None if private_mac => "private address",
        None => "unknown vendor",
    }
}

pub fn router_down_detail(gateway: Ipv4Addr) -> String {
    format!("No reply from {gateway} to ping or ARP.")
}

pub fn internet_down_detail(target: Ipv4Addr) -> String {
    format!("Your router answers, but {target} doesn't (ping or HTTPS port).")
}

/// `ms` is rounded to a whole number.
pub fn latency_high_detail(ms: f64, target: Ipv4Addr, checks: u32, limit_ms: f64) -> String {
    format!("{ms:.0} ms to {target} on the last {checks} checks (limit {limit_ms:.0} ms).")
}

/// "Packet loss to your router" for the gateway, else "Packet loss to {name}".
pub fn loss_title(target: &Target, name: &str) -> String {
    match target {
        Target::Gateway => "Packet loss to your router".to_string(),
        _ => format!("Packet loss to {name}"),
    }
}

pub fn loss_detail(lost: u32, window: u32) -> String {
    format!("{lost} of the last {window} pings got no reply.")
}

pub fn offline_title(name: &str) -> String {
    format!("{name} is offline")
}

pub fn offline_detail(minutes: i64) -> String {
    format!("Not seen for {minutes} min.")
}

pub fn new_device_title(name: &str, vendor: &str, type_label: &str) -> String {
    format!("New device: {name} — {vendor}, {type_label}")
}

pub fn new_device_detail(ip: &str) -> String {
    format!("First seen at {ip}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_device_title_matches_journey_b() {
        assert_eq!(
            new_device_title(
                "192.168.1.57",
                vendor_label(Some("Espressif"), false),
                type_label(DeviceType::Iot)
            ),
            "New device: 192.168.1.57 — Espressif, IoT"
        );
    }
}
