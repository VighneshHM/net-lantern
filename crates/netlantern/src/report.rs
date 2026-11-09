//! The terminal reports: the `--scan-once` table, `--diagnose-once` and
//! `--wifi-once`.

use std::net::Ipv4Addr;
use std::time::Duration;

use netlantern_core::{CoreError, Device, DiagnosisRun, LayerStatus, NearbyScan, WifiStatus};
use netlantern_platform::{InterfaceKind, PlatformError, LOCATION_PERMISSION};

// The same words the API uses; the other copy is `location_permission` in
// crates/netlantern-server/src/error.rs. This crate does not depend on that
// module, so keep the two in step.
const LOCATION_MESSAGE: &str = "Windows is hiding Wi-Fi details from Net Lantern.";
const LOCATION_HINT: &str = r#"Turn on Location services and "Let desktop apps access your location" in Settings > Privacy & security > Location."#;

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

fn status_word(status: LayerStatus) -> &'static str {
    match status {
        LayerStatus::Pass => "PASSED",
        LayerStatus::Warn => "WARNING",
        LayerStatus::Fail => "FAILED",
        LayerStatus::Skipped => "SKIPPED",
    }
}

/// A finished diagnosis: one block per step, then the verdict.
pub fn diagnosis_report(run: &DiagnosisRun) -> String {
    let mut out = String::new();
    for step in &run.steps {
        out.push_str(&format!(
            "{:<13}{:<9}{}\n",
            step.title,
            status_word(step.status),
            step.explanation
        ));
        for line in &step.evidence {
            out.push_str(&format!("  - {line}\n"));
        }
        if let Some(fix) = &step.fix {
            out.push_str(&format!("  Try this: {fix}\n"));
        }
    }
    if let Some(verdict) = &run.verdict {
        out.push_str(&format!("\nVerdict: {}\n", verdict.headline));
        if let Some(fix) = &verdict.fix {
            out.push_str(&format!("Try this: {fix}\n"));
        }
    }
    out
}

fn signal_label(rssi_dbm: i32) -> &'static str {
    match rssi_dbm {
        r if r >= -50 => "Excellent",
        r if r >= -60 => "Good",
        r if r >= -70 => "Fair",
        _ => "Weak",
    }
}

fn is_location(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Platform(PlatformError::PermissionNeeded { what }) if what == LOCATION_PERMISSION
    )
}

fn location_lines() -> String {
    format!("{LOCATION_MESSAGE}\n{LOCATION_HINT}\n")
}

fn connection_lines(status: &WifiStatus) -> String {
    if status.interface_kind == Some(InterfaceKind::Ethernet) {
        return "Connected by cable; Wi-Fi details don't apply.\n".to_string();
    }
    let Some(c) = &status.connection else {
        return "Not connected to Wi-Fi.\n".to_string();
    };
    let width = c
        .width_mhz
        .map(|w| format!(", {w} MHz wide"))
        .unwrap_or_default();
    let link = match (c.rx_rate_mbps, c.tx_rate_mbps) {
        (None, None) => String::new(),
        (rx, tx) => {
            let show = |r: Option<u32>| r.map_or("?".to_string(), |v| v.to_string());
            format!(", link {}/{} Mbps", show(rx), show(tx))
        }
    };
    format!(
        "Connected to \"{}\" ({})\n  {} channel {}{}, signal {} dBm ({}), {}{}\n  BSSID {}\n",
        cell(&c.ssid),
        c.security,
        c.band.label(),
        c.channel,
        width,
        c.rssi_dbm,
        signal_label(c.rssi_dbm),
        c.phy,
        link,
        c.bssid
    )
}

/// The `--wifi-once` report: the current connection, a blank line, then the
/// nearby networks. A permission problem is a normal report, not a failure.
pub fn wifi_report(
    status: &Result<WifiStatus, CoreError>,
    nearby: &Result<NearbyScan, CoreError>,
) -> String {
    let mut out = match status {
        Ok(status) => connection_lines(status),
        Err(e) if is_location(e) => location_lines(),
        Err(CoreError::NotConnected) => "Not connected to Wi-Fi.\n".to_string(),
        Err(e) => format!("{e}\n"),
    };
    out.push('\n');
    match nearby {
        Ok(scan) => {
            out.push_str("SSID\tBSSID\tBAND\tCHANNEL\tSIGNAL\n");
            for n in &scan.networks {
                let ssid = if n.ssid.is_empty() {
                    "(hidden)".to_string()
                } else {
                    cell(&n.ssid)
                };
                out.push_str(&format!(
                    "{ssid}\t{}\t{}\t{}\t{} dBm ({})\n",
                    n.bssid,
                    n.band.label(),
                    n.channel,
                    n.rssi_dbm,
                    signal_label(n.rssi_dbm)
                ));
            }
            out.push_str(&format!("Advice: {}\n", scan.congestion.advice));
        }
        Err(e) if is_location(e) => out.push_str(&location_lines()),
        Err(CoreError::Platform(PlatformError::Unavailable { reason })) => {
            out.push_str(&format!("Wi-Fi details aren't available: {reason}.\n"));
        }
        Err(e) => out.push_str(&format!("{e}\n")),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlantern_core::{
        Congestion, DiagnosisId, Finding, Layer, LayerResult, RunState, Verdict,
    };
    use netlantern_platform::{NearbyNetwork, WifiBand, WifiConnection};

    fn step(title: &str, status: LayerStatus, evidence: &[&str], fix: Option<&str>) -> LayerResult {
        LayerResult {
            layer: Layer::Link,
            title: title.into(),
            status,
            finding: Finding::WeakAndCrowded,
            evidence: evidence.iter().map(|e| e.to_string()).collect(),
            explanation: "Signal is weak.".into(),
            fix: fix.map(str::to_owned),
        }
    }

    fn run(steps: Vec<LayerResult>, verdict: Option<Verdict>) -> DiagnosisRun {
        DiagnosisRun {
            id: DiagnosisId(1),
            state: RunState::Done,
            started_at: 0,
            finished_at: Some(1),
            steps,
            verdict,
            error: None,
        }
    }

    #[test]
    fn diagnosis_report_lists_steps_and_verdict() {
        let verdict = Verdict {
            finding: Finding::WeakAndCrowded,
            status: LayerStatus::Warn,
            layer: Some(Layer::Link),
            headline: "Your Wi-Fi signal is weak and the channel is crowded.".into(),
            fix: Some("Move closer to the router.".into()),
        };
        let out = diagnosis_report(&run(
            vec![
                step(
                    "Wi-Fi link",
                    LayerStatus::Warn,
                    &["Signal -75 dBm"],
                    Some("Move closer."),
                ),
                step("Router", LayerStatus::Skipped, &[], None),
            ],
            Some(verdict),
        ));
        assert!(
            out.contains("Wi-Fi link   WARNING  Signal is weak.\n"),
            "{out}"
        );
        assert!(
            out.contains("  - Signal -75 dBm\n  Try this: Move closer.\n"),
            "{out}"
        );
        assert!(out.contains("Router       SKIPPED  "), "{out}");
        assert!(
            out.contains("\nVerdict: Your Wi-Fi signal is weak and the channel is crowded.\nTry this: Move closer to the router.\n"),
            "{out}"
        );
    }

    #[test]
    fn wifi_report_explains_location_permission() {
        let denied = || {
            CoreError::Platform(PlatformError::PermissionNeeded {
                what: LOCATION_PERMISSION.into(),
            })
        };
        let out = wifi_report(&Err(denied()), &Err(denied()));
        assert!(
            out.contains("Windows is hiding Wi-Fi details from Net Lantern."),
            "{out}"
        );
        assert!(out.contains("Location services"), "{out}");
    }

    fn connection(width: Option<u32>, rx: Option<u32>, tx: Option<u32>) -> WifiStatus {
        WifiStatus {
            interface_kind: Some(InterfaceKind::Wifi),
            connection: Some(WifiConnection {
                ssid: "Home".into(),
                bssid: "00:11:22:33:44:55".parse().unwrap(),
                rssi_dbm: -61,
                channel: 36,
                band: WifiBand::Ghz5,
                width_mhz: width,
                phy: "Wi-Fi 6 (802.11ax)".into(),
                rx_rate_mbps: rx,
                tx_rate_mbps: tx,
                security: "WPA2-Personal".into(),
            }),
        }
    }

    fn nearby_err() -> Result<NearbyScan, CoreError> {
        Err(CoreError::Platform(PlatformError::Unavailable {
            reason: "no adapter".into(),
        }))
    }

    #[test]
    fn wifi_report_connection_lines() {
        let out = wifi_report(
            &Ok(connection(Some(80), Some(866), Some(780))),
            &nearby_err(),
        );
        assert!(
            out.starts_with("Connected to \"Home\" (WPA2-Personal)\n  5 GHz channel 36, 80 MHz wide, signal -61 dBm (Fair), Wi-Fi 6 (802.11ax), link 866/780 Mbps\n  BSSID 00:11:22:33:44:55\n"),
            "{out}"
        );
        assert!(
            out.contains("Wi-Fi details aren't available: no adapter."),
            "{out}"
        );
        let out = wifi_report(&Ok(connection(None, Some(866), None)), &nearby_err());
        assert!(out.contains("channel 36, signal"), "{out}");
        assert!(out.contains("link 866/? Mbps"), "{out}");
    }

    #[test]
    fn wifi_report_lists_nearby_with_hidden_ssids_and_advice() {
        let scan = NearbyScan {
            scanned_at: 0,
            networks: vec![NearbyNetwork {
                ssid: String::new(),
                bssid: "AA:BB:CC:DD:EE:FF".parse().unwrap(),
                rssi_dbm: -45,
                channel: 6,
                band: WifiBand::Ghz2_4,
                width_mhz: None,
            }],
            congestion: Congestion {
                channels: vec![],
                current: None,
                advice: "All quiet.".into(),
            },
        };
        let out = wifi_report(&Err(CoreError::NotConnected), &Ok(scan));
        assert!(out.starts_with("Not connected to Wi-Fi.\n"), "{out}");
        assert!(
            out.contains("SSID\tBSSID\tBAND\tCHANNEL\tSIGNAL\n(hidden)\tAA:BB:CC:DD:EE:FF\t2.4 GHz\t6\t-45 dBm (Excellent)\n"),
            "{out}"
        );
        assert!(out.contains("Advice: All quiet.\n"), "{out}");
    }

    #[test]
    fn signal_labels_follow_the_thresholds() {
        let labels: Vec<&str> = [-50, -51, -60, -61, -70, -71]
            .into_iter()
            .map(signal_label)
            .collect();
        assert_eq!(
            labels,
            ["Excellent", "Good", "Good", "Fair", "Fair", "Weak"]
        );
    }

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
