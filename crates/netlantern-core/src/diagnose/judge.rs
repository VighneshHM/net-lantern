//! Pure judges: turn what the probes saw into a finding per layer, and the
//! layers into one verdict.

use netlantern_platform::{HttpResponse, PlatformError};

use super::model::*;
use netlantern_platform::InterfaceKind;

use super::copy;

/// Longest piece of network text shown in evidence, in characters.
const MAX_NETWORK_TEXT: usize = 100;

fn cut(s: &str) -> String {
    s.chars().take(MAX_NETWORK_TEXT).collect()
}

fn result(
    layer: Layer,
    title: &str,
    status: LayerStatus,
    finding: Finding,
    evidence: Vec<String>,
) -> LayerResult {
    LayerResult {
        layer,
        title: title.into(),
        status,
        finding,
        evidence,
        explanation: copy::explanation(finding).into(),
        fix: copy::fix(finding).map(String::from),
    }
}

fn ping_line(p: &PingStats) -> String {
    match p.avg_ms() {
        Some(avg) => format!(
            "{}: {} of {} replies, average {} ms",
            p.target,
            p.replies(),
            p.sent,
            avg
        ),
        None => format!("{}: no replies", p.target),
    }
}

pub fn classify_portal(
    res: &Result<HttpResponse, PlatformError>,
    expected_body: &str,
) -> PortalCheck {
    match res {
        Ok(r) if r.status == 200 && r.body_prefix.trim() == expected_body => PortalCheck::Expected,
        Ok(r) if (300..400).contains(&r.status) || r.status == 511 => PortalCheck::Redirect {
            status: r.status,
            location: r.location.clone(),
        },
        Ok(r) => PortalCheck::UnexpectedAnswer { status: r.status },
        Err(e) => PortalCheck::Failed(e.to_string()),
    }
}

pub fn judge_link(p: &LinkProbe, cfg: &DiagnoseConfig) -> LayerResult {
    let title = match p.interface {
        Some(InterfaceKind::Wifi) => "Wi-Fi link",
        Some(InterfaceKind::Ethernet) => "Cable link",
        _ => "Network link",
    };
    let link = |status, finding, evidence| result(Layer::Link, title, status, finding, evidence);

    let Some(kind) = p.interface else {
        return link(
            LayerStatus::Fail,
            Finding::NotConnected,
            vec!["Not connected to any network".into()],
        );
    };
    if kind != InterfaceKind::Wifi {
        return link(
            LayerStatus::Pass,
            Finding::EthernetOk,
            vec!["Connected by cable".into()],
        );
    }
    // The interface being up proves the link; the reading is only detail.
    let conn = match &p.wifi {
        WifiReading::Connected(conn) => conn,
        WifiReading::PermissionNeeded => {
            return link(
                LayerStatus::Pass,
                Finding::SignalHidden,
                vec!["Signal details hidden: location access is off".into()],
            );
        }
        other => {
            let reason = match other {
                WifiReading::Unavailable(reason) => cut(reason),
                _ => "not reported".into(),
            };
            return link(
                LayerStatus::Pass,
                Finding::LinkDetailsUnavailable,
                vec![format!("Signal details unavailable ({reason})")],
            );
        }
    };

    let mut evidence = vec![format!(
        "Wi-Fi \"{}\", {} channel {}, signal {} dBm",
        conn.ssid,
        conn.band.label(),
        conn.channel,
        conn.rssi_dbm
    )];
    if let Some(load) = &p.congestion {
        evidence.push(format!(
            "Channel {} is shared with {} other networks",
            load.channel, load.overlapping
        ));
    }
    let weak = conn.rssi_dbm < cfg.wifi_warn_dbm;
    let crowded = p.congestion.as_ref().is_some_and(|c| c.crowded);
    let own_5ghz = p.congestion.as_ref().is_some_and(|c| c.own_5ghz_visible);
    let (status, finding) = match (weak, crowded) {
        (true, true) => (LayerStatus::Warn, Finding::WeakAndCrowded),
        (true, false) => (LayerStatus::Warn, Finding::WeakSignal),
        (false, true) => (LayerStatus::Warn, Finding::CrowdedChannel),
        (false, false) => (LayerStatus::Pass, Finding::LinkOk),
    };
    let mut r = link(status, finding, evidence);
    match finding {
        Finding::CrowdedChannel => r.fix = p.congestion_advice.clone(),
        Finding::WeakAndCrowded if !own_5ghz => {
            r.fix = Some(
                "Move closer to the router, and change the router's channel (see Wi-Fi quality below)."
                    .into(),
            );
        }
        _ => {}
    }
    r
}

pub fn judge_router(p: &RouterProbe, cfg: &DiagnoseConfig) -> LayerResult {
    let gw = p.pings.target;
    let mut evidence = vec![ping_line(&p.pings)];
    let (status, finding) = if p.pings.replies() == 0 {
        match p.answers_arp {
            Some(true) => {
                evidence.push(format!("{gw} answered at the network level (ARP)"));
                (LayerStatus::Warn, Finding::RouterIgnoresPing)
            }
            Some(false) => {
                evidence.push(format!("{gw} didn't answer ARP either"));
                (LayerStatus::Fail, Finding::RouterUnreachable)
            }
            None => (LayerStatus::Fail, Finding::RouterUnreachable),
        }
    } else if p.pings.lost() > 0 {
        (LayerStatus::Warn, Finding::RouterLossy)
    } else if p.pings.avg_ms().is_some_and(|a| a > cfg.router_warn_rtt_ms) {
        (LayerStatus::Warn, Finding::RouterSlow)
    } else {
        (LayerStatus::Pass, Finding::RouterOk)
    };
    result(Layer::Router, "Router", status, finding, evidence)
}

pub fn judge_internet(p: &InternetProbe, cfg: &DiagnoseConfig) -> LayerResult {
    let mut evidence: Vec<String> = p.pings.iter().map(ping_line).collect();
    evidence.push(format!(
        "TCP {} to {}: {}",
        p.tcp.port,
        p.tcp.target,
        if p.tcp.open { "open" } else { "no answer" }
    ));
    evidence.push(match &p.portal {
        PortalCheck::Expected => "Connectivity check: answered as expected".into(),
        PortalCheck::Redirect { location, .. } => format!(
            "Connectivity check: redirected to {}",
            location.as_deref().map_or("(no location)".into(), cut)
        ),
        PortalCheck::UnexpectedAnswer { status } => {
            format!("Connectivity check: unexpected answer (HTTP {status})")
        }
        PortalCheck::Failed(e) => format!("Connectivity check: failed ({})", cut(e)),
    });

    let internet =
        |status, finding, evidence| result(Layer::Internet, "Internet", status, finding, evidence);
    // Portals block ping and outside DNS, so the portal check comes first.
    if matches!(
        p.portal,
        PortalCheck::Redirect { .. } | PortalCheck::UnexpectedAnswer { .. }
    ) {
        return internet(LayerStatus::Fail, Finding::CaptivePortal, evidence);
    }
    let any_ping = p.pings.iter().any(|s| s.replies() > 0);
    let web_worked = p.tcp.open || p.portal == PortalCheck::Expected;
    if !any_ping && !web_worked {
        return internet(LayerStatus::Fail, Finding::InternetDown, evidence);
    }
    if !any_ping {
        evidence.push("Pings are blocked on this network; TCP and web checks worked.".into());
        return internet(LayerStatus::Pass, Finding::InternetOk, evidence);
    }
    if p.pings.iter().any(|s| s.lost() > 0) {
        return internet(LayerStatus::Warn, Finding::InternetLossy, evidence);
    }
    let best = p.pings.iter().filter_map(PingStats::avg_ms).min();
    if best.is_some_and(|b| b > cfg.internet_warn_rtt_ms) {
        return internet(LayerStatus::Warn, Finding::InternetSlow, evidence);
    }
    internet(LayerStatus::Pass, Finding::InternetOk, evidence)
}

fn lookup_succeeded(l: &DnsLookup) -> bool {
    l.result.as_ref().is_ok_and(|addrs| !addrs.is_empty())
}

fn lookup_line(l: &DnsLookup) -> String {
    match &l.result {
        Ok(addrs) if !addrs.is_empty() => format!("{} via {}: {}", l.name, l.server, addrs[0]),
        Ok(_) => format!("{} via {}: failed (no addresses)", l.name, l.server),
        Err(e) => format!("{} via {}: failed ({})", l.name, l.server, cut(e)),
    }
}

pub fn judge_dns(p: &DnsProbe) -> LayerResult {
    let evidence: Vec<String> = p.system.iter().chain(&p.direct).map(lookup_line).collect();
    let ok = p.system.iter().filter(|l| lookup_succeeded(l)).count();
    let finding = if p.system_servers.is_empty() {
        Finding::NoDnsServer
    } else if ok > 0 && ok == p.system.len() {
        Finding::DnsOk
    } else if ok > 0 {
        Finding::DnsPartial
    } else if p.direct.iter().any(lookup_succeeded) {
        Finding::DnsServerProblem
    } else {
        Finding::DnsUnreachable
    };
    let status = match finding {
        Finding::DnsOk => LayerStatus::Pass,
        Finding::DnsPartial => LayerStatus::Warn,
        _ => LayerStatus::Fail,
    };
    result(Layer::Dns, "DNS", status, finding, evidence)
}

pub fn judge_website(p: &WebsiteProbe) -> LayerResult {
    let (status, finding, line) = match &p.outcome {
        WebsiteOutcome::Status(code) => (
            LayerStatus::Pass,
            Finding::WebsiteOk,
            format!("{}: HTTP {code}", p.url),
        ),
        WebsiteOutcome::Certificate(m) => (
            LayerStatus::Fail,
            Finding::TlsIntercepted,
            format!("{}: certificate rejected ({})", p.url, cut(m)),
        ),
        WebsiteOutcome::Failed(m) => (
            LayerStatus::Fail,
            Finding::WebsiteUnreachable,
            format!("{}: failed ({})", p.url, cut(m)),
        ),
    };
    result(Layer::Website, "Website", status, finding, vec![line])
}

pub fn skipped(layer: Layer) -> LayerResult {
    let title = match layer {
        Layer::Link => "Network link",
        Layer::Router => "Router",
        Layer::Internet => "Internet",
        Layer::Dns => "DNS",
        Layer::Website => "Website",
    };
    result(layer, title, LayerStatus::Skipped, Finding::Skipped, vec![])
}

pub fn verdict(steps: &[LayerResult]) -> Verdict {
    let decisive = [LayerStatus::Fail, LayerStatus::Warn]
        .into_iter()
        .find_map(|s| steps.iter().find(|r| r.status == s));
    match decisive {
        Some(step) => Verdict {
            finding: step.finding,
            status: step.status,
            layer: Some(step.layer),
            headline: copy::headline(step.finding).into(),
            fix: step.fix.clone(),
        },
        None => Verdict {
            finding: Finding::Healthy,
            status: LayerStatus::Pass,
            layer: None,
            headline: copy::headline(Finding::Healthy).into(),
            fix: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use netlantern_platform::{InterfaceKind, MacAddr, WifiBand, WifiConnection};

    use super::super::copy;
    use super::*;
    use crate::wifi::CurrentLoad;

    fn cfg() -> DiagnoseConfig {
        DiagnoseConfig::default()
    }

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn stats(target: &str, sent: u32, rtts: &[u32]) -> PingStats {
        PingStats {
            target: ip(target),
            sent,
            rtts_ms: rtts.to_vec(),
        }
    }

    fn tcp(open: bool) -> TcpCheck {
        TcpCheck {
            target: ip("1.1.1.1"),
            port: 443,
            open,
        }
    }

    fn pass(layer: Layer) -> LayerResult {
        LayerResult {
            layer,
            title: "x".into(),
            status: LayerStatus::Pass,
            finding: Finding::Healthy,
            evidence: vec![],
            explanation: String::new(),
            fix: None,
        }
    }

    fn kind(r: &LayerResult) -> (LayerStatus, Finding) {
        (r.status, r.finding)
    }

    fn connection(rssi: i32) -> WifiConnection {
        WifiConnection {
            ssid: "Home".into(),
            bssid: MacAddr([1, 2, 3, 4, 5, 6]),
            rssi_dbm: rssi,
            channel: 6,
            band: WifiBand::Ghz2_4,
            width_mhz: Some(20),
            phy: "Wi-Fi 6 (802.11ax)".into(),
            rx_rate_mbps: None,
            tx_rate_mbps: None,
            security: "WPA2-Personal".into(),
        }
    }

    fn load(crowded: bool, own_5ghz_visible: bool) -> CurrentLoad {
        CurrentLoad {
            band: WifiBand::Ghz2_4,
            channel: 6,
            overlapping: 7,
            crowded,
            own_5ghz_visible,
            suggested_channel: None,
        }
    }

    fn wifi_probe(rssi: i32, congestion: Option<CurrentLoad>) -> LinkProbe {
        LinkProbe {
            interface: Some(InterfaceKind::Wifi),
            wifi: WifiReading::Connected(connection(rssi)),
            congestion,
            congestion_advice: Some("Switch to channel 11.".into()),
        }
    }

    fn weak_crowded_probe(own_5ghz: bool) -> LinkProbe {
        wifi_probe(-74, Some(load(true, own_5ghz)))
    }

    fn lookup(name: &str, server: &str, result: Result<Vec<Ipv4Addr>, String>) -> DnsLookup {
        DnsLookup {
            name: name.into(),
            server: ip(server),
            result,
        }
    }

    fn good(name: &str, server: &str) -> DnsLookup {
        lookup(name, server, Ok(vec![ip("20.1.2.3")]))
    }

    fn bad(name: &str, server: &str) -> DnsLookup {
        lookup(name, server, Err("timed out".into()))
    }

    fn dns_probe(system: Vec<DnsLookup>, direct: Vec<DnsLookup>) -> DnsProbe {
        DnsProbe {
            system_servers: vec![ip("192.168.1.1")],
            direct_server: ip("1.1.1.1"),
            system,
            direct,
        }
    }

    // Link

    #[test]
    fn link_without_an_interface_fails_not_connected() {
        let p = LinkProbe {
            interface: None,
            wifi: WifiReading::NotWifi,
            congestion: None,
            congestion_advice: None,
        };
        let r = judge_link(&p, &cfg());
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::NotConnected));
        assert_eq!(r.title, "Network link");
        assert_eq!(r.evidence, vec!["Not connected to any network"]);
        assert_eq!(r.layer, Layer::Link);
    }

    #[test]
    fn cable_passes_as_ethernet_ok() {
        let p = LinkProbe {
            interface: Some(InterfaceKind::Ethernet),
            wifi: WifiReading::NotWifi,
            congestion: None,
            congestion_advice: None,
        };
        let r = judge_link(&p, &cfg());
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::EthernetOk));
        assert_eq!(r.title, "Cable link");
        assert_eq!(r.evidence, vec!["Connected by cable"]);
        assert_eq!(r.fix, None);
    }

    #[test]
    fn other_interface_passes_with_the_network_title() {
        let p = LinkProbe {
            interface: Some(InterfaceKind::Other),
            wifi: WifiReading::NotWifi,
            congestion: None,
            congestion_advice: None,
        };
        let r = judge_link(&p, &cfg());
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::EthernetOk));
        assert_eq!(r.title, "Network link");
    }

    fn wifi_reading_probe(wifi: WifiReading) -> LinkProbe {
        LinkProbe {
            interface: Some(InterfaceKind::Wifi),
            wifi,
            congestion: None,
            congestion_advice: None,
        }
    }

    #[test]
    fn unavailable_evidence_has_no_doubled_prefix() {
        let r = judge_link(
            &wifi_reading_probe(WifiReading::Unavailable("radio off".into())),
            &cfg(),
        );
        assert_eq!(r.evidence, vec!["Signal details unavailable (radio off)"]);
    }

    #[test]
    fn location_permission_off_still_passes_the_link() {
        let r = judge_link(&wifi_reading_probe(WifiReading::PermissionNeeded), &cfg());
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::SignalHidden));
        assert_eq!(r.title, "Wi-Fi link");
        assert_eq!(
            r.evidence,
            vec!["Signal details hidden: location access is off"]
        );
        assert_eq!(r.fix.as_deref(), copy::fix(Finding::SignalHidden));
    }

    #[test]
    fn missing_signal_details_pass_the_link_without_location_advice() {
        for (wifi, evidence) in [
            (
                WifiReading::Disconnected,
                "Signal details unavailable (not reported)",
            ),
            (
                WifiReading::Unavailable("timed out".into()),
                "Signal details unavailable (timed out)",
            ),
        ] {
            let r = judge_link(&wifi_reading_probe(wifi), &cfg());
            assert_eq!(
                kind(&r),
                (LayerStatus::Pass, Finding::LinkDetailsUnavailable)
            );
            assert_eq!(r.title, "Wi-Fi link");
            assert_eq!(r.evidence, vec![evidence]);
            assert_eq!(
                r.explanation,
                "Connected. Windows didn't report signal details this time."
            );
            assert_eq!(r.fix, None);
        }
        let long = "x".repeat(150);
        let r = judge_link(&wifi_reading_probe(WifiReading::Unavailable(long)), &cfg());
        assert_eq!(
            r.evidence,
            vec![format!("Signal details unavailable ({})", "x".repeat(100))]
        );
        assert_eq!(
            copy::headline(Finding::LinkDetailsUnavailable),
            "Your Wi-Fi connection is up."
        );
        assert_eq!(copy::fix(Finding::LinkDetailsUnavailable), None);
    }

    #[test]
    fn healthy_wifi_passes_with_evidence() {
        let r = judge_link(&wifi_probe(-55, Some(load(false, false))), &cfg());
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::LinkOk));
        assert_eq!(
            r.evidence,
            vec![
                "Wi-Fi \"Home\", 2.4 GHz channel 6, signal -55 dBm".to_string(),
                "Channel 6 is shared with 7 other networks".to_string(),
            ]
        );
        assert_eq!(r.explanation, copy::explanation(Finding::LinkOk));
        assert_eq!(r.fix, None);
    }

    #[test]
    fn wifi_without_congestion_has_only_the_signal_line() {
        let r = judge_link(&wifi_probe(-55, None), &cfg());
        assert_eq!(r.evidence.len(), 1);
    }

    #[test]
    fn weak_signal_alone_warns() {
        let r = judge_link(&wifi_probe(-74, Some(load(false, false))), &cfg());
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::WeakSignal));
        assert_eq!(r.fix.as_deref(), copy::fix(Finding::WeakSignal));
    }

    #[test]
    fn signal_exactly_at_the_threshold_is_not_weak() {
        let r = judge_link(&wifi_probe(-70, None), &cfg());
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::LinkOk));
    }

    #[test]
    fn crowded_alone_warns_with_the_congestion_advice_as_fix() {
        let r = judge_link(&wifi_probe(-55, Some(load(true, false))), &cfg());
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::CrowdedChannel));
        assert_eq!(r.fix.as_deref(), Some("Switch to channel 11."));
    }

    #[test]
    fn weak_and_crowded_with_own_5ghz_gives_journey_a_fix() {
        let v = verdict(&[
            judge_link(&weak_crowded_probe(true), &cfg()),
            pass(Layer::Router),
            pass(Layer::Internet),
            pass(Layer::Dns),
            pass(Layer::Website),
        ]);
        assert_eq!(
            v.headline,
            "Your Wi-Fi signal is weak and the channel is crowded."
        );
        assert_eq!(
            v.fix.as_deref(),
            Some("Move closer to the router, or join the 5 GHz network shown in the nearby list.")
        );
    }

    #[test]
    fn weak_and_crowded_without_own_5ghz_changes_the_channel() {
        let r = judge_link(&weak_crowded_probe(false), &cfg());
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::WeakAndCrowded));
        assert_eq!(
            r.fix.as_deref(),
            Some("Move closer to the router, and change the router's channel (see Wi-Fi quality below).")
        );
    }

    // Router

    #[test]
    fn healthy_router_passes_with_evidence() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[2, 3, 3, 4, 3]),
                answers_arp: None,
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::RouterOk));
        assert_eq!(r.title, "Router");
        assert_eq!(
            r.evidence,
            vec!["192.168.1.1: 5 of 5 replies, average 3 ms"]
        );
    }

    #[test]
    fn gateway_that_ignores_ping_but_answers_arp_warns_and_continues() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[]),
                answers_arp: Some(true),
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::RouterIgnoresPing));
        assert_eq!(
            r.evidence,
            vec![
                "192.168.1.1: no replies".to_string(),
                "192.168.1.1 answered at the network level (ARP)".to_string(),
            ]
        );
    }

    #[test]
    fn silent_gateway_without_arp_is_unreachable() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[]),
                answers_arp: Some(false),
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::RouterUnreachable));
        assert_eq!(
            r.evidence,
            vec![
                "192.168.1.1: no replies".to_string(),
                "192.168.1.1 didn't answer ARP either".to_string(),
            ]
        );
    }

    #[test]
    fn silent_gateway_with_unasked_arp_is_unreachable() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[]),
                answers_arp: None,
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::RouterUnreachable));
    }

    #[test]
    fn one_lost_gateway_ping_warns_lossy() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[3, 3, 3, 3]),
                answers_arp: None,
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::RouterLossy));
        assert_eq!(
            r.evidence,
            vec!["192.168.1.1: 4 of 5 replies, average 3 ms"]
        );
    }

    #[test]
    fn slow_router_warns_and_the_average_rounds() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[60, 61, 60, 60, 60]),
                answers_arp: None,
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::RouterSlow));
        assert_eq!(
            r.evidence,
            vec!["192.168.1.1: 5 of 5 replies, average 60 ms"]
        );
    }

    #[test]
    fn router_average_at_the_limit_is_fine() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[50; 5]),
                answers_arp: None,
            },
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::RouterOk));
    }

    #[test]
    fn loss_wins_over_slowness_on_the_router() {
        let r = judge_router(
            &RouterProbe {
                pings: stats("192.168.1.1", 5, &[90, 90, 90]),
                answers_arp: None,
            },
            &cfg(),
        );
        assert_eq!(r.finding, Finding::RouterLossy);
    }

    // Internet

    fn net(pings: Vec<PingStats>, open: bool, portal: PortalCheck) -> InternetProbe {
        InternetProbe {
            pings,
            tcp: tcp(open),
            portal,
        }
    }

    #[test]
    fn healthy_internet_passes_with_evidence() {
        let r = judge_internet(
            &net(
                vec![
                    stats("1.1.1.1", 3, &[10, 12, 11]),
                    stats("8.8.8.8", 3, &[20, 20, 20]),
                ],
                true,
                PortalCheck::Expected,
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::InternetOk));
        assert_eq!(r.title, "Internet");
        assert_eq!(
            r.evidence,
            vec![
                "1.1.1.1: 3 of 3 replies, average 11 ms".to_string(),
                "8.8.8.8: 3 of 3 replies, average 20 ms".to_string(),
                "TCP 443 to 1.1.1.1: open".to_string(),
                "Connectivity check: answered as expected".to_string(),
            ]
        );
    }

    #[test]
    fn blocked_icmp_with_working_tcp_passes() {
        let p = InternetProbe {
            pings: vec![stats("1.1.1.1", 3, &[]), stats("8.8.8.8", 3, &[])],
            tcp: tcp(true),
            portal: PortalCheck::Expected,
        };
        let r = judge_internet(&p, &cfg());
        assert_eq!(
            (r.status, r.finding),
            (LayerStatus::Pass, Finding::InternetOk)
        );
        assert!(r.evidence.contains(
            &"Pings are blocked on this network; TCP and web checks worked.".to_string()
        ));
        assert!(r.evidence.contains(&"1.1.1.1: no replies".to_string()));
    }

    #[test]
    fn portal_redirect_beats_everything_failing() {
        let r = judge_internet(
            &net(
                vec![stats("1.1.1.1", 3, &[]), stats("8.8.8.8", 3, &[])],
                false,
                PortalCheck::Redirect {
                    status: 302,
                    location: Some("http://portal/".into()),
                },
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::CaptivePortal));
        assert!(r
            .evidence
            .contains(&"Connectivity check: redirected to http://portal/".to_string()));
        assert!(r
            .evidence
            .contains(&"TCP 443 to 1.1.1.1: no answer".to_string()));
    }

    #[test]
    fn redirect_without_location_says_so() {
        let r = judge_internet(
            &net(
                vec![],
                false,
                PortalCheck::Redirect {
                    status: 511,
                    location: None,
                },
            ),
            &cfg(),
        );
        assert!(r
            .evidence
            .contains(&"Connectivity check: redirected to (no location)".to_string()));
    }

    #[test]
    fn unexpected_portal_answer_is_a_captive_portal() {
        let r = judge_internet(
            &net(
                vec![stats("1.1.1.1", 3, &[10; 3])],
                true,
                PortalCheck::UnexpectedAnswer { status: 200 },
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::CaptivePortal));
        assert!(r
            .evidence
            .contains(&"Connectivity check: unexpected answer (HTTP 200)".to_string()));
    }

    #[test]
    fn portal_failure_from_broken_dns_is_not_a_portal() {
        let r = judge_internet(
            &net(
                vec![stats("1.1.1.1", 3, &[10; 3]), stats("8.8.8.8", 3, &[10; 3])],
                true,
                PortalCheck::Failed("dns error".into()),
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::InternetOk));
        assert!(r
            .evidence
            .contains(&"Connectivity check: failed (dns error)".to_string()));
    }

    #[test]
    fn nothing_answering_is_internet_down() {
        let r = judge_internet(
            &net(
                vec![stats("1.1.1.1", 3, &[]), stats("8.8.8.8", 3, &[])],
                false,
                PortalCheck::Failed("timed out".into()),
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::InternetDown));
    }

    #[test]
    fn a_working_portal_alone_proves_reachability() {
        let r = judge_internet(
            &net(
                vec![stats("1.1.1.1", 3, &[]), stats("8.8.8.8", 3, &[])],
                false,
                PortalCheck::Expected,
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::InternetOk));
    }

    #[test]
    fn partial_ping_loss_warns_lossy() {
        let r = judge_internet(
            &net(
                vec![
                    stats("1.1.1.1", 3, &[10, 10]),
                    stats("8.8.8.8", 3, &[10; 3]),
                ],
                true,
                PortalCheck::Expected,
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::InternetLossy));
    }

    #[test]
    fn one_silent_target_among_replying_ones_is_lossy() {
        let r = judge_internet(
            &net(
                vec![stats("1.1.1.1", 3, &[10; 3]), stats("8.8.8.8", 3, &[])],
                true,
                PortalCheck::Expected,
            ),
            &cfg(),
        );
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::InternetLossy));
    }

    #[test]
    fn slow_internet_uses_the_best_replying_target() {
        let slow = net(
            vec![
                stats("1.1.1.1", 3, &[150; 3]),
                stats("8.8.8.8", 3, &[160; 3]),
            ],
            true,
            PortalCheck::Expected,
        );
        assert_eq!(
            kind(&judge_internet(&slow, &cfg())),
            (LayerStatus::Warn, Finding::InternetSlow)
        );
        let one_fast = net(
            vec![
                stats("1.1.1.1", 3, &[150; 3]),
                stats("8.8.8.8", 3, &[40; 3]),
            ],
            true,
            PortalCheck::Expected,
        );
        assert_eq!(
            kind(&judge_internet(&one_fast, &cfg())),
            (LayerStatus::Pass, Finding::InternetOk)
        );
    }

    // classify_portal

    fn resp(
        status: u16,
        body: &str,
        location: Option<&str>,
    ) -> Result<HttpResponse, PlatformError> {
        Ok(HttpResponse {
            status,
            body_prefix: body.into(),
            location: location.map(String::from),
        })
    }

    #[test]
    fn classify_portal_cases() {
        let expected = "Microsoft Connect Test";
        assert_eq!(
            classify_portal(&resp(200, "Microsoft Connect Test\n", None), expected),
            PortalCheck::Expected
        );
        assert_eq!(
            classify_portal(&resp(302, "", Some("http://portal/")), expected),
            PortalCheck::Redirect {
                status: 302,
                location: Some("http://portal/".into())
            }
        );
        assert_eq!(
            classify_portal(&resp(511, "", None), expected),
            PortalCheck::Redirect {
                status: 511,
                location: None
            }
        );
        assert_eq!(
            classify_portal(&resp(200, "<html>login", None), expected),
            PortalCheck::UnexpectedAnswer { status: 200 }
        );
        assert_eq!(
            classify_portal(&resp(404, "", None), expected),
            PortalCheck::UnexpectedAnswer { status: 404 }
        );
        assert_eq!(
            classify_portal(
                &Err(PlatformError::Io {
                    message: "dns error".into()
                }),
                expected
            ),
            PortalCheck::Failed("I/O error: dns error".into())
        );
    }

    #[test]
    fn network_text_is_cut_to_100_characters_not_bytes() {
        let long = "é".repeat(150);
        let r = judge_internet(
            &net(
                vec![],
                false,
                PortalCheck::Redirect {
                    status: 302,
                    location: Some(long),
                },
            ),
            &cfg(),
        );
        let line = r
            .evidence
            .iter()
            .find(|l| l.starts_with("Connectivity check: redirected to "))
            .unwrap();
        let shown = line.trim_start_matches("Connectivity check: redirected to ");
        assert_eq!(shown.chars().count(), 100);
    }

    // DNS

    #[test]
    fn working_dns_passes_with_evidence() {
        let r = judge_dns(&dns_probe(
            vec![
                good("www.microsoft.com", "192.168.1.1"),
                good("www.cloudflare.com", "192.168.1.1"),
            ],
            vec![good("www.microsoft.com", "1.1.1.1")],
        ));
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::DnsOk));
        assert_eq!(r.title, "DNS");
        assert_eq!(
            r.evidence,
            vec![
                "www.microsoft.com via 192.168.1.1: 20.1.2.3".to_string(),
                "www.cloudflare.com via 192.168.1.1: 20.1.2.3".to_string(),
                "www.microsoft.com via 1.1.1.1: 20.1.2.3".to_string(),
            ]
        );
    }

    #[test]
    fn no_system_servers_fails_no_dns_server() {
        let mut p = dns_probe(vec![], vec![good("www.microsoft.com", "1.1.1.1")]);
        p.system_servers.clear();
        assert_eq!(
            kind(&judge_dns(&p)),
            (LayerStatus::Fail, Finding::NoDnsServer)
        );
    }

    #[test]
    fn some_failed_lookups_warn_partial() {
        let r = judge_dns(&dns_probe(
            vec![
                good("www.microsoft.com", "192.168.1.1"),
                bad("www.cloudflare.com", "192.168.1.1"),
            ],
            vec![],
        ));
        assert_eq!(kind(&r), (LayerStatus::Warn, Finding::DnsPartial));
        assert!(r
            .evidence
            .contains(&"www.cloudflare.com via 192.168.1.1: failed (timed out)".to_string()));
    }

    #[test]
    fn system_dns_down_but_direct_works_is_server_problem() {
        let r = judge_dns(&dns_probe(
            vec![
                bad("www.microsoft.com", "192.168.1.1"),
                bad("www.cloudflare.com", "192.168.1.1"),
            ],
            vec![
                good("www.microsoft.com", "1.1.1.1"),
                bad("www.cloudflare.com", "1.1.1.1"),
            ],
        ));
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::DnsServerProblem));
    }

    #[test]
    fn both_dns_paths_down_is_unreachable() {
        let r = judge_dns(&dns_probe(
            vec![bad("www.microsoft.com", "192.168.1.1")],
            vec![bad("www.microsoft.com", "1.1.1.1")],
        ));
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::DnsUnreachable));
    }

    #[test]
    fn empty_answer_counts_as_failed_lookup() {
        let r = judge_dns(&dns_probe(
            vec![lookup("www.microsoft.com", "192.168.1.1", Ok(vec![]))],
            vec![good("www.microsoft.com", "1.1.1.1")],
        ));
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::DnsServerProblem));
        assert!(r
            .evidence
            .contains(&"www.microsoft.com via 192.168.1.1: failed (no addresses)".to_string()));
    }

    // Website

    fn site(outcome: WebsiteOutcome) -> LayerResult {
        judge_website(&WebsiteProbe {
            url: "https://www.microsoft.com/".into(),
            outcome,
        })
    }

    #[test]
    fn website_answer_passes() {
        let r = site(WebsiteOutcome::Status(200));
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::WebsiteOk));
        assert_eq!(r.title, "Website");
        assert_eq!(r.evidence, vec!["https://www.microsoft.com/: HTTP 200"]);
    }

    #[test]
    fn redirect_from_website_passes() {
        let r = site(WebsiteOutcome::Status(302));
        assert_eq!(kind(&r), (LayerStatus::Pass, Finding::WebsiteOk));
    }

    #[test]
    fn certificate_error_is_tls_intercepted() {
        let r = site(WebsiteOutcome::Certificate("unknown issuer".into()));
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::TlsIntercepted));
        assert_eq!(
            r.evidence,
            vec!["https://www.microsoft.com/: certificate rejected (unknown issuer)"]
        );
    }

    #[test]
    fn website_failure_is_unreachable() {
        let r = site(WebsiteOutcome::Failed("timed out".into()));
        assert_eq!(kind(&r), (LayerStatus::Fail, Finding::WebsiteUnreachable));
        assert_eq!(
            r.evidence,
            vec!["https://www.microsoft.com/: failed (timed out)"]
        );
    }

    // skipped and verdict

    #[test]
    fn skipped_layers_carry_the_standard_text() {
        let r = skipped(Layer::Link);
        assert_eq!(kind(&r), (LayerStatus::Skipped, Finding::Skipped));
        assert_eq!(r.title, "Network link");
        assert_eq!(
            r.explanation,
            "Not checked because an earlier layer failed."
        );
        assert!(r.evidence.is_empty());
        assert_eq!(r.fix, None);
        assert_eq!(skipped(Layer::Dns).title, "DNS");
        assert_eq!(skipped(Layer::Router).title, "Router");
        assert_eq!(skipped(Layer::Internet).title, "Internet");
        assert_eq!(skipped(Layer::Website).title, "Website");
    }

    fn step(layer: Layer, status: LayerStatus, finding: Finding, fix: Option<&str>) -> LayerResult {
        LayerResult {
            layer,
            title: "t".into(),
            status,
            finding,
            evidence: vec![],
            explanation: String::new(),
            fix: fix.map(String::from),
        }
    }

    #[test]
    fn verdict_prefers_first_fail_then_first_warn_then_healthy() {
        let steps = vec![
            step(
                Layer::Link,
                LayerStatus::Warn,
                Finding::WeakSignal,
                Some("closer"),
            ),
            step(
                Layer::Router,
                LayerStatus::Fail,
                Finding::RouterUnreachable,
                Some("restart"),
            ),
            step(
                Layer::Internet,
                LayerStatus::Fail,
                Finding::InternetDown,
                None,
            ),
        ];
        let v = verdict(&steps);
        assert_eq!(v.finding, Finding::RouterUnreachable);
        assert_eq!(v.status, LayerStatus::Fail);
        assert_eq!(v.layer, Some(Layer::Router));
        assert_eq!(v.headline, "Your laptop can't reach the router.");
        assert_eq!(v.fix.as_deref(), Some("restart"));

        let warns = vec![
            pass(Layer::Link),
            step(
                Layer::Router,
                LayerStatus::Warn,
                Finding::RouterLossy,
                Some("move"),
            ),
            step(
                Layer::Internet,
                LayerStatus::Warn,
                Finding::InternetSlow,
                None,
            ),
        ];
        let v = verdict(&warns);
        assert_eq!(
            (v.finding, v.status),
            (Finding::RouterLossy, LayerStatus::Warn)
        );
        assert_eq!(v.fix.as_deref(), Some("move"));

        let v = verdict(&[pass(Layer::Link), skipped(Layer::Router)]);
        assert_eq!(v.finding, Finding::Healthy);
        assert_eq!(v.status, LayerStatus::Pass);
        assert_eq!(v.layer, None);
        assert_eq!(v.headline, "Everything looks fine.");
        assert_eq!(v.fix, None);
    }

    #[test]
    fn verdict_of_no_steps_is_healthy() {
        assert_eq!(verdict(&[]).finding, Finding::Healthy);
    }

    // Copy and wire shapes

    const ALL_FINDINGS: [Finding; 28] = [
        Finding::EthernetOk,
        Finding::LinkOk,
        Finding::SignalHidden,
        Finding::LinkDetailsUnavailable,
        Finding::NotConnected,
        Finding::WeakSignal,
        Finding::CrowdedChannel,
        Finding::WeakAndCrowded,
        Finding::RouterOk,
        Finding::RouterIgnoresPing,
        Finding::RouterUnreachable,
        Finding::RouterLossy,
        Finding::RouterSlow,
        Finding::InternetOk,
        Finding::CaptivePortal,
        Finding::InternetDown,
        Finding::InternetLossy,
        Finding::InternetSlow,
        Finding::DnsOk,
        Finding::NoDnsServer,
        Finding::DnsServerProblem,
        Finding::DnsUnreachable,
        Finding::DnsPartial,
        Finding::WebsiteOk,
        Finding::TlsIntercepted,
        Finding::WebsiteUnreachable,
        Finding::Healthy,
        Finding::Skipped,
    ];

    #[test]
    fn every_finding_has_copy() {
        for f in ALL_FINDINGS {
            if f != Finding::Healthy {
                assert!(!copy::explanation(f).is_empty(), "{f:?} explanation");
            }
            if f != Finding::Skipped {
                assert!(!copy::headline(f).is_empty(), "{f:?} headline");
            }
        }
        assert_eq!(copy::explanation(Finding::Healthy), "");
        assert_eq!(copy::fix(Finding::CrowdedChannel), None);
    }

    #[test]
    fn wire_shapes() {
        let r = judge_link(&weak_crowded_probe(true), &cfg());
        let json = serde_json::to_value(&r).unwrap();
        let mut keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "evidence",
                "explanation",
                "finding",
                "fix",
                "layer",
                "status",
                "title"
            ]
        );
        assert_eq!(json["finding"], "weak_and_crowded");
        assert_eq!(
            serde_json::to_value(Finding::LinkDetailsUnavailable).unwrap(),
            "link_details_unavailable"
        );
        assert_eq!(json["layer"], "link");
        assert_eq!(json["status"], "warn");

        let run = DiagnosisRun {
            id: DiagnosisId(7),
            state: RunState::Running,
            started_at: 1,
            finished_at: None,
            steps: vec![],
            verdict: None,
            error: None,
        };
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["id"], 7);
        assert_eq!(json["state"], "running");
        let back: DiagnosisRun = serde_json::from_value(json).unwrap();
        assert_eq!(back, run);
    }

    #[test]
    fn default_config_matches_the_constraints() {
        let c = cfg();
        assert_eq!(c.wifi_warn_dbm, -70);
        assert_eq!(c.gateway_pings, 5);
        assert_eq!(c.router_warn_rtt_ms, 50);
        assert_eq!(c.internet_pings, 3);
        assert_eq!(c.internet_warn_rtt_ms, 100);
        assert_eq!(c.tcp_target, (ip("1.1.1.1"), 443));
        assert_eq!(c.layer_timeout.as_secs(), 15);
        assert_eq!(c.nearby_max_age.as_secs(), 60);
        assert_eq!(c.post_scan_settle.as_secs(), 2);
    }
}
