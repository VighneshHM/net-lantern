//! The `--scan-once` table.

use std::net::Ipv4Addr;
use std::time::Duration;

use netlantern_core::Device;

/// The header line of the table.
pub const HEADER: &str = "ip\tmac\tvendor\tname\ttype\tping";

/// Tabs and line breaks would break the table's columns.
fn cell(text: &str) -> String {
    text.replace(['\t', '\r', '\n'], " ")
}

/// The snake_case name of a device type, as the API spells it.
fn type_name(device: &Device) -> String {
    serde_json::to_value(device.device_type)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn row(device: &Device) -> String {
    let ping = match (device.answers_ping, device.rtt_ms) {
        (true, Some(rtt)) => format!("{rtt} ms"),
        (true, None) => "replied".to_string(),
        (false, _) => "arp-only".to_string(),
    };
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        device.ip.map(|ip| ip.to_string()).unwrap_or_default(),
        device.mac.map(|m| m.to_string()).unwrap_or_default(),
        cell(device.vendor.as_deref().unwrap_or("")),
        cell(&device.display_name),
        type_name(device),
        ping,
    )
}

/// The whole report: header, one row per device sorted by IP (devices
/// without an IP last), then the summary line.
pub fn render(devices: &[Device], elapsed: Duration) -> String {
    let mut sorted: Vec<&Device> = devices.iter().collect();
    sorted.sort_by_key(|d| (d.ip.is_none(), d.ip.as_ref().map(Ipv4Addr::octets)));
    let mut out = String::from(HEADER);
    out.push('\n');
    for d in sorted {
        out.push_str(&row(d));
        out.push('\n');
    }
    out.push_str(&format!(
        "Found {} devices in {:.1} s\n",
        devices.len(),
        elapsed.as_secs_f64()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(ip: Option<&str>, name: &str, answers_ping: bool, rtt: Option<u32>) -> Device {
        serde_json::from_value(serde_json::json!({
            "id": 1, "ip": ip, "mac": "00:11:32:AB:CD:70", "private_mac": false,
            "vendor": null, "hostname": null, "name": null, "display_name": name,
            "type": "nas", "type_confidence": 0.9, "starred": false, "is_new": false,
            "online": true, "answers_ping": answers_ping, "rtt_ms": rtt,
            "is_self": false, "is_gateway": false, "first_seen": 0, "last_seen": 0,
            "services": [], "open_ports": [],
        }))
        .unwrap()
    }

    #[test]
    fn rows_sort_by_ip_numerically_with_no_ip_last() {
        let devices = [
            device(None, "none", true, Some(1)),
            device(Some("192.168.1.100"), "b", true, Some(1)),
            device(Some("192.168.1.9"), "a", true, Some(1)),
        ];
        let out = render(&devices, Duration::from_millis(1234));
        let names: Vec<&str> = out
            .lines()
            .skip(1)
            .take(3)
            .map(|l| l.split('\t').nth(3).unwrap())
            .collect();
        assert_eq!(names, ["a", "b", "none"]);
        assert!(out.starts_with(HEADER));
        assert_eq!(out.lines().last().unwrap(), "Found 3 devices in 1.2 s");
    }

    #[test]
    fn ping_column_has_rtt_arp_only_or_replied() {
        let out = render(
            &[
                device(Some("10.0.0.1"), "x", true, Some(4)),
                device(Some("10.0.0.2"), "x", false, None),
                device(Some("10.0.0.3"), "x", true, None),
            ],
            Duration::ZERO,
        );
        let pings: Vec<&str> = out
            .lines()
            .skip(1)
            .take(3)
            .map(|l| l.rsplit('\t').next().unwrap())
            .collect();
        assert_eq!(pings, ["4 ms", "arp-only", "replied"]);
    }

    #[test]
    fn tabs_and_newlines_in_names_cannot_break_the_columns() {
        let out = render(
            &[device(Some("10.0.0.1"), "a\tb\nc", true, Some(1))],
            Duration::ZERO,
        );
        let row = out.lines().nth(1).unwrap();
        assert_eq!(row.split('\t').count(), 6);
        assert_eq!(row.split('\t').nth(3), Some("a b c"));
    }
}
