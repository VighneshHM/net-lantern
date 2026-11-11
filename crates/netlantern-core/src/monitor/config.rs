//! Monitoring settings. The defaults are the values in the project constraints.

use std::net::Ipv4Addr;
use std::time::Duration;

/// How long each kind of data is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    pub raw: Duration,
    pub rollups: Duration,
    pub presence: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            raw: Duration::from_secs(24 * 3600),
            rollups: Duration::from_secs(30 * 24 * 3600),
            presence: Duration::from_secs(90 * 24 * 3600),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorConfig {
    pub tick: Duration,
    pub dns_every: Duration,
    pub presence_every: Duration,
    pub rollup_every: Duration,
    pub ping_timeout: Duration,
    pub tcp_fallback_timeout: Duration,
    pub arp_timeout: Duration,
    pub internet_target: Ipv4Addr,
    pub dns_name: String,
    pub offline_grace: Duration,
    pub offline_grace_sleepy: Duration,
    pub latency_high_ms: f64,
    pub consecutive: u32,
    pub loss_window: u32,
    pub loss_threshold: u32,
    pub sleep_gap: Duration,
    pub retention: Retention,
    pub presence_ping_concurrency: usize,
    pub presence_arp_concurrency: usize,
    pub max_checks: usize,
    /// The presence refresh pings only devices seen within this long; older
    /// ones come back through the ARP table.
    pub sweep_max_unseen: Duration,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(30),
            dns_every: Duration::from_secs(60),
            presence_every: Duration::from_secs(300),
            rollup_every: Duration::from_secs(300),
            ping_timeout: Duration::from_secs(1),
            tcp_fallback_timeout: Duration::from_secs(2),
            arp_timeout: Duration::from_secs(3),
            internet_target: Ipv4Addr::new(1, 1, 1, 1),
            dns_name: "www.microsoft.com".into(),
            offline_grace: Duration::from_secs(120),
            offline_grace_sleepy: Duration::from_secs(600),
            latency_high_ms: 100.0,
            consecutive: 2,
            loss_window: 30,
            loss_threshold: 3,
            sleep_gap: Duration::from_secs(90),
            retention: Retention::default(),
            presence_ping_concurrency: 16,
            presence_arp_concurrency: 8,
            max_checks: 5,
            sweep_max_unseen: Duration::from_secs(7 * 24 * 3600),
        }
    }
}

impl MonitorConfig {
    /// Every duration divided by `factor` (at least 10 ms), for fast tests.
    /// Retention and `sweep_max_unseen` are left alone.
    pub fn scaled(&self, factor: u32) -> MonitorConfig {
        let f = factor.max(1);
        let s = |d: Duration| (d / f).max(Duration::from_millis(10));
        MonitorConfig {
            tick: s(self.tick),
            dns_every: s(self.dns_every),
            presence_every: s(self.presence_every),
            rollup_every: s(self.rollup_every),
            ping_timeout: s(self.ping_timeout),
            tcp_fallback_timeout: s(self.tcp_fallback_timeout),
            arp_timeout: s(self.arp_timeout),
            offline_grace: s(self.offline_grace),
            offline_grace_sleepy: s(self.offline_grace_sleepy),
            sleep_gap: s(self.sleep_gap),
            ..self.clone()
        }
    }
}
