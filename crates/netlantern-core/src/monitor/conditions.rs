//! Check conditions: turn one tick's probe results into readings and samples.

use std::collections::{HashMap, VecDeque};
use std::net::Ipv4Addr;

use netlantern_platform::WifiConnection;

use crate::model::{Device, DeviceId};
use crate::monitor::{
    copy, presence, AlertKey, CheckOutcome, Metric, MonitorConfig, Reading, Sample, Target,
};

/// What one tick measured. `None` RTTs mean no reply.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickResult {
    pub at: i64,
    pub gateway_rtt: Option<f64>,
    /// Only tried when the gateway didn't answer ping; `None` when not tried.
    pub gateway_arp: Option<bool>,
    pub internet_rtt: Option<f64>,
    /// The TCP connect time to the internet target's port 443; only tried
    /// when the ping got no reply.
    pub internet_tcp_ms: Option<f64>,
    /// Outer `None`: not measured this tick. Inner `None`: no answer.
    pub dns_ms: Option<Option<f64>>,
    pub wifi: Option<WifiConnection>,
    pub devices: Vec<DeviceTick>,
}

/// One starred device's results in a tick.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceTick {
    pub id: DeviceId,
    pub rtt: Option<f64>,
    pub arp: Option<bool>,
    pub checks: Vec<CheckOutcome>,
}

/// The last `size` reply/no-reply results per target.
#[derive(Clone, Debug)]
pub struct LossWindow {
    size: u32,
    rings: HashMap<Target, VecDeque<bool>>,
}

impl LossWindow {
    pub fn new(size: u32) -> Self {
        Self {
            size: size.max(1),
            rings: HashMap::new(),
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn push(&mut self, target: Target, replied: bool) {
        let ring = self.rings.entry(target).or_default();
        ring.push_back(replied);
        while ring.len() > self.size as usize {
            ring.pop_front();
        }
    }

    /// How many of the results held for `target` got no reply.
    pub fn lost(&self, target: &Target) -> u32 {
        self.rings
            .get(target)
            .map_or(0, |r| r.iter().filter(|replied| !**replied).count() as u32)
    }

    /// Forget every target, e.g. when a new session starts.
    pub fn clear(&mut self) {
        self.rings.clear();
    }

    /// Forget one target, e.g. a device that was unstarred.
    pub fn remove(&mut self, target: &Target) {
        self.rings.remove(target);
    }
}

fn reading(key: AlertKey, failing: bool, ts: i64, title: String, detail: String) -> Reading {
    Reading {
        key,
        failing,
        ts,
        title,
        detail,
    }
}

/// The router is down when it answered neither ping nor ARP.
fn router_failing(t: &TickResult) -> bool {
    t.gateway_rtt.is_none() && t.gateway_arp == Some(false)
}

/// The internet's round trip: the ping, else the TCP connect time.
fn internet_ms(t: &TickResult) -> Option<f64> {
    t.internet_rtt.or(t.internet_tcp_ms)
}

/// Failing when the gateway answered neither ping nor ARP. A gateway that
/// ignores ping but answers ARP is up.
pub fn router_reading(t: &TickResult, gateway: Ipv4Addr) -> Reading {
    reading(
        AlertKey::RouterDown,
        router_failing(t),
        t.at,
        copy::ROUTER_DOWN_TITLE.to_string(),
        copy::router_down_detail(gateway),
    )
}

/// Failing when the router is up but the internet target answered neither
/// ping nor the TCP connect to port 443. While the router is down the
/// internet is not blamed.
pub fn internet_reading(t: &TickResult, cfg: &MonitorConfig) -> Reading {
    reading(
        AlertKey::InternetDown,
        !router_failing(t) && internet_ms(t).is_none(),
        t.at,
        copy::INTERNET_DOWN_TITLE.to_string(),
        copy::internet_down_detail(cfg.internet_target),
    )
}

/// Failing when the internet answered and its round trip is above
/// `latency_high_ms`. The engine asks for `consecutive` of these in a row.
pub fn latency_reading(t: &TickResult, cfg: &MonitorConfig) -> Reading {
    let ms = internet_ms(t);
    reading(
        AlertKey::LatencyHigh,
        ms.is_some_and(|ms| ms > cfg.latency_high_ms),
        t.at,
        copy::LATENCY_HIGH_TITLE.to_string(),
        copy::latency_high_detail(
            ms.unwrap_or(0.0),
            cfg.internet_target,
            cfg.consecutive,
            cfg.latency_high_ms,
        ),
    )
}

/// Failing when at least `loss_threshold` of the window's pings got no
/// reply. `name` is the device's display name (unused for the gateway);
/// `at` is the reading's time.
pub fn loss_reading(
    target: Target,
    window: &LossWindow,
    name: &str,
    at: i64,
    cfg: &MonitorConfig,
) -> Reading {
    let lost = window.lost(&target);
    let title = copy::loss_title(&target, name);
    reading(
        AlertKey::PacketLoss(target),
        lost >= cfg.loss_threshold,
        at,
        title,
        copy::loss_detail(lost, window.size()),
    )
}

/// Failing when a starred device has been unseen for longer than its grace,
/// counted from the later of `last_seen` and the session start.
pub fn offline_reading(
    device: &Device,
    session_started_at: i64,
    now: i64,
    cfg: &MonitorConfig,
) -> Reading {
    let grace = presence::grace(device.device_type, cfg);
    let online = presence::is_online(device.last_seen, session_started_at, now, grace);
    let unseen_ms = now.saturating_sub(device.last_seen.max(session_started_at));
    reading(
        AlertKey::StarredOffline(device.id),
        !online,
        now,
        copy::offline_title(&device.display_name),
        copy::offline_detail(unseen_ms.max(0) / 60_000),
    )
}

/// Failing while the device is still flagged new.
pub fn new_device_reading(device: &Device, now: i64) -> Reading {
    let ip = device
        .ip
        .map_or_else(|| "an unknown address".to_string(), |ip| ip.to_string());
    reading(
        AlertKey::NewDevice(device.id),
        device.is_new,
        now,
        copy::new_device_title(
            &device.display_name,
            copy::vendor_label(device.vendor.as_deref(), device.private_mac),
            copy::type_label(device.device_type),
        ),
        copy::new_device_detail(&ip),
    )
}

/// The samples to store for one tick, all at `t.at`:
/// - the gateway's RTT, only once it has answered a ping this session (a
///   gateway that ignores ping would otherwise look like 100% loss);
/// - the internet's RTT (the ping, else the TCP connect time, else no reply);
/// - the DNS lookup time, when measured this tick;
/// - Wi-Fi signal and receive rate, when connected;
/// - each starred device's RTT.
pub fn samples(t: &TickResult, gateway_has_answered_ping: bool) -> Vec<Sample> {
    let sample = |target: Target, metric: Metric, value: Option<f64>| Sample {
        target,
        metric,
        ts: t.at,
        value,
    };
    let mut out = Vec::with_capacity(5 + t.devices.len());
    if gateway_has_answered_ping {
        out.push(sample(Target::Gateway, Metric::RttMs, t.gateway_rtt));
    }
    out.push(sample(Target::Internet, Metric::RttMs, internet_ms(t)));
    if let Some(dns) = t.dns_ms {
        out.push(sample(Target::Dns, Metric::LookupMs, dns));
    }
    if let Some(w) = &t.wifi {
        out.push(sample(
            Target::Wifi,
            Metric::RssiDbm,
            Some(f64::from(w.rssi_dbm)),
        ));
        if let Some(rx) = w.rx_rate_mbps {
            out.push(sample(
                Target::Wifi,
                Metric::LinkRateMbps,
                Some(f64::from(rx)),
            ));
        }
    }
    for d in &t.devices {
        out.push(sample(Target::Device(d.id), Metric::RttMs, d.rtt));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceType;

    const GW: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

    fn tick() -> TickResult {
        TickResult {
            at: 30_000,
            gateway_rtt: Some(2.0),
            internet_rtt: Some(20.0),
            ..TickResult::default()
        }
    }

    #[test]
    fn a_router_answering_arp_is_not_down() {
        let t = TickResult {
            gateway_rtt: None,
            gateway_arp: Some(true),
            ..tick()
        };
        let r = router_reading(&t, GW);
        assert_eq!(r.key, AlertKey::RouterDown);
        assert!(!r.failing);
        assert_eq!(r.ts, 30_000);

        let down = TickResult {
            gateway_rtt: None,
            gateway_arp: Some(false),
            ..tick()
        };
        let r = router_reading(&down, GW);
        assert!(r.failing);
        assert_eq!(r.title, "Your router isn't answering");
        assert_eq!(r.detail, "No reply from 192.168.1.1 to ping or ARP.");
    }

    #[test]
    fn tcp_443_counts_as_internet_up() {
        let cfg = MonitorConfig::default();
        let t = TickResult {
            internet_rtt: None,
            internet_tcp_ms: Some(30.0),
            ..tick()
        };
        let r = internet_reading(&t, &cfg);
        assert_eq!(r.key, AlertKey::InternetDown);
        assert!(!r.failing);

        let down = TickResult {
            internet_rtt: None,
            internet_tcp_ms: None,
            ..tick()
        };
        let r = internet_reading(&down, &cfg);
        assert!(r.failing);
        assert_eq!(r.title, "The internet is down");
        assert_eq!(
            r.detail,
            "Your router answers, but 1.1.1.1 doesn't (ping or HTTPS port)."
        );
    }

    #[test]
    fn internet_is_not_blamed_while_the_router_is_down() {
        let cfg = MonitorConfig::default();
        let t = TickResult {
            gateway_rtt: None,
            gateway_arp: Some(false),
            internet_rtt: None,
            internet_tcp_ms: None,
            ..tick()
        };
        assert!(router_reading(&t, GW).failing);
        assert!(!internet_reading(&t, &cfg).failing);
    }

    #[test]
    fn latency_uses_the_ping_else_the_tcp_time() {
        let cfg = MonitorConfig::default();
        let slow = TickResult {
            internet_rtt: Some(140.4),
            ..tick()
        };
        let r = latency_reading(&slow, &cfg);
        assert_eq!(r.key, AlertKey::LatencyHigh);
        assert!(r.failing);
        assert_eq!(r.title, "Internet latency is high");
        assert_eq!(
            r.detail,
            "140 ms to 1.1.1.1 on the last 2 checks (limit 100 ms)."
        );
        let slow_tcp = TickResult {
            internet_rtt: None,
            internet_tcp_ms: Some(150.0),
            ..tick()
        };
        assert!(latency_reading(&slow_tcp, &cfg).failing);
        assert!(!latency_reading(&tick(), &cfg).failing);
        let down = TickResult {
            internet_rtt: None,
            ..tick()
        };
        assert!(!latency_reading(&down, &cfg).failing);
    }

    #[test]
    fn a_silent_gateway_records_no_loss_samples() {
        let t = TickResult {
            gateway_rtt: None,
            gateway_arp: Some(true),
            ..tick()
        };
        let s = samples(&t, false);
        assert!(!s.iter().any(|s| s.target == Target::Gateway));
        let s = samples(&t, true);
        assert!(s.iter().any(|s| s.target == Target::Gateway
            && s.metric == Metric::RttMs
            && s.value.is_none()));
    }

    #[test]
    fn samples_cover_internet_dns_wifi_and_devices() {
        let t = TickResult {
            internet_rtt: None,
            internet_tcp_ms: Some(31.0),
            dns_ms: Some(None),
            devices: vec![DeviceTick {
                id: DeviceId(4),
                rtt: Some(3.0),
                arp: None,
                checks: vec![],
            }],
            ..tick()
        };
        let s = samples(&t, true);
        let find = |target: Target, metric: Metric| {
            s.iter()
                .find(|s| s.target == target && s.metric == metric)
                .map(|s| s.value)
        };
        assert_eq!(find(Target::Gateway, Metric::RttMs), Some(Some(2.0)));
        assert_eq!(find(Target::Internet, Metric::RttMs), Some(Some(31.0)));
        assert_eq!(find(Target::Dns, Metric::LookupMs), Some(None));
        assert_eq!(
            find(Target::Device(DeviceId(4)), Metric::RttMs),
            Some(Some(3.0))
        );
        assert_eq!(find(Target::Wifi, Metric::RssiDbm), None);
        assert!(s.iter().all(|s| s.ts == 30_000));

        let no_dns = samples(&tick(), true);
        assert!(!no_dns.iter().any(|s| s.target == Target::Dns));
    }

    #[test]
    fn loss_window_counts_the_last_thirty_only() {
        let mut w = LossWindow::new(30);
        for _ in 0..3 {
            w.push(Target::Gateway, false);
        }
        assert_eq!(w.lost(&Target::Gateway), 3);
        for _ in 0..30 {
            w.push(Target::Gateway, true);
        }
        assert_eq!(w.lost(&Target::Gateway), 0);
        assert_eq!(w.lost(&Target::Device(DeviceId(1))), 0);
    }

    #[test]
    fn three_lost_of_thirty_is_failing() {
        let cfg = MonitorConfig::default();
        let mut w = LossWindow::new(cfg.loss_window);
        for i in 0..30 {
            w.push(Target::Gateway, i % 10 != 0);
        }
        let r = loss_reading(Target::Gateway, &w, "Router", 900_000, &cfg);
        assert_eq!(r.key, AlertKey::PacketLoss(Target::Gateway));
        assert!(r.failing);
        assert_eq!(r.ts, 900_000);
        assert_eq!(r.title, "Packet loss to your router");
        assert_eq!(r.detail, "3 of the last 30 pings got no reply.");

        let dev = Target::Device(DeviceId(7));
        w.push(dev.clone(), false);
        w.push(dev.clone(), false);
        let r = loss_reading(dev.clone(), &w, "Office printer", 900_000, &cfg);
        assert!(!r.failing);
        assert_eq!(r.title, "Packet loss to Office printer");
        w.push(dev.clone(), false);
        assert!(loss_reading(dev, &w, "Office printer", 900_000, &cfg).failing);
    }

    fn device(id: i64, device_type: DeviceType, last_seen: i64) -> Device {
        Device {
            id: DeviceId(id),
            ip: Some(Ipv4Addr::new(192, 168, 1, 57)),
            mac: None,
            private_mac: false,
            vendor: Some("Espressif".into()),
            hostname: None,
            name: None,
            display_name: "192.168.1.57".into(),
            device_type,
            type_confidence: 0.5,
            starred: true,
            is_new: true,
            online: true,
            answers_ping: true,
            rtt_ms: None,
            is_self: false,
            is_gateway: false,
            first_seen: 0,
            last_seen,
            services: vec![],
            open_ports: vec![],
        }
    }

    #[test]
    fn offline_counts_from_the_session_start() {
        let cfg = MonitorConfig::default();
        let now = 10 * 3_600_000;
        let printer = device(4, DeviceType::Printer, now - 8 * 3_600_000);
        // Session started 1 min ago: still within the 2 min grace.
        assert!(!offline_reading(&printer, now - 60_000, now, &cfg).failing);
        let r = offline_reading(&printer, now - 3 * 60_000, now, &cfg);
        assert!(r.failing);
        assert_eq!(r.key, AlertKey::StarredOffline(DeviceId(4)));
        assert_eq!(r.title, "192.168.1.57 is offline");
        assert_eq!(r.detail, "Not seen for 3 min.");
        let phone = device(5, DeviceType::Phone, now - 8 * 60_000);
        assert!(!offline_reading(&phone, 0, now, &cfg).failing);
    }

    #[test]
    fn new_device_reading_fails_while_flagged_new() {
        let mut d = device(9, DeviceType::Iot, 0);
        let r = new_device_reading(&d, 5);
        assert!(r.failing);
        assert_eq!(r.key, AlertKey::NewDevice(DeviceId(9)));
        assert_eq!(r.title, "New device: 192.168.1.57 — Espressif, IoT");
        assert_eq!(r.detail, "First seen at 192.168.1.57.");
        d.is_new = false;
        assert!(!new_device_reading(&d, 5).failing);
    }
}
