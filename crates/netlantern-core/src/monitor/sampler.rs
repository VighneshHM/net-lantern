//! The monitor's probes: I/O only. Every failure becomes "no answer" (or
//! "unknown"), and is logged once per kind through [`Faults`].

use std::collections::HashSet;
use std::fmt::Display;
use std::net::Ipv4Addr;
use std::sync::{Mutex, PoisonError};

use futures::stream::{self, StreamExt};
use netlantern_platform::{Interface, MacAddr, NetIo, Platform, TcpProbe};
use tokio::time::Instant;

use crate::model::DeviceId;
use crate::monitor::{Check, CheckOutcome, DeviceTick, MonitorConfig, TickResult};

/// Longest network-supplied text kept in a check's detail.
const MAX_TEXT: usize = 100;

/// Logs each kind of failure the first time it happens, so a broken store or
/// adapter does not flood the log every 30 s.
#[derive(Default)]
pub(crate) struct Faults(Mutex<HashSet<&'static str>>);

impl Faults {
    pub(crate) fn note(&self, kind: &'static str, error: &dyn Display) {
        let first = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(kind);
        if first {
            tracing::warn!(kind, error = %error, "monitoring step failed; it will keep trying");
        }
    }

    /// `Some(value)`, or `None` after noting the error.
    pub(crate) fn ok<T, E: Display>(&self, kind: &'static str, r: Result<T, E>) -> Option<T> {
        r.map_err(|e| self.note(kind, &e)).ok()
    }
}

/// A starred device to probe this tick.
pub(crate) struct StarredProbe {
    pub id: DeviceId,
    pub ip: Ipv4Addr,
    /// The device's stored MAC: another MAC answering ARP for `ip` is another
    /// device, not this one.
    pub mac: Option<MacAddr>,
    pub checks: Vec<Check>,
}

/// What one tick needs to know.
pub(crate) struct TickPlan<'a> {
    pub at: i64,
    pub iface: &'a Interface,
    pub devices: Vec<StarredProbe>,
    pub dns_due: bool,
}

fn ms_since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// Run one tick's probes concurrently.
pub(crate) async fn tick(
    platform: &dyn Platform,
    netio: &dyn NetIo,
    cfg: &MonitorConfig,
    faults: &Faults,
    plan: TickPlan<'_>,
) -> TickResult {
    let gateway = async {
        let rtt = ping(platform, faults, plan.iface.gateway, cfg).await;
        let arp = match rtt {
            Some(_) => None,
            None => arp(platform, faults, plan.iface.gateway, cfg).await,
        };
        (rtt, arp)
    };
    let internet = async {
        if let Some(rtt) = ping(platform, faults, cfg.internet_target, cfg).await {
            return (Some(rtt), None);
        }
        let start = Instant::now();
        let probe = platform
            .tcp_probe(cfg.internet_target, 443, cfg.tcp_fallback_timeout)
            .await;
        let tcp = match faults.ok("internet tcp probe", probe) {
            Some(TcpProbe::Open) => Some(ms_since(start)),
            _ => None,
        };
        (None, tcp)
    };
    let dns = async {
        let server = plan.iface.dns_servers.first().copied();
        match server {
            Some(server) if plan.dns_due => {
                let start = Instant::now();
                let r = netio.dns_resolve(server, &cfg.dns_name).await;
                Some(r.ok().map(|_| ms_since(start)))
            }
            _ => None,
        }
    };
    let wifi = async { platform.wifi_current().await.ok().flatten() };
    let devices = futures::future::join_all(
        plan.devices
            .iter()
            .map(|d| device(platform, netio, cfg, faults, d, plan.at)),
    );
    let ((gateway_rtt, gateway_arp), (internet_rtt, internet_tcp_ms), dns_ms, wifi, devices) =
        tokio::join!(gateway, internet, dns, wifi, devices);
    TickResult {
        at: plan.at,
        gateway_rtt,
        gateway_arp,
        internet_rtt,
        internet_tcp_ms,
        dns_ms,
        wifi,
        devices,
    }
}

/// Round trip in ms, or `None` for no reply (or a failed ping).
async fn ping(
    platform: &dyn Platform,
    faults: &Faults,
    ip: Ipv4Addr,
    cfg: &MonitorConfig,
) -> Option<f64> {
    let reply = platform.ping(ip, cfg.ping_timeout).await;
    faults
        .ok("ping", reply)
        .flatten()
        .map(|r| r.rtt.as_secs_f64() * 1000.0)
}

/// Whether `ip` answered ARP; `None` when the request itself failed.
async fn arp(
    platform: &dyn Platform,
    faults: &Faults,
    ip: Ipv4Addr,
    cfg: &MonitorConfig,
) -> Option<bool> {
    arp_mac(platform, faults, ip, cfg)
        .await
        .map(|m| m.is_some())
}

/// The MAC that answered ARP for `ip`; outer `None` when the request failed.
async fn arp_mac(
    platform: &dyn Platform,
    faults: &Faults,
    ip: Ipv4Addr,
    cfg: &MonitorConfig,
) -> Option<Option<MacAddr>> {
    let mac = platform.arp_resolve(ip, cfg.arp_timeout).await;
    faults.ok("arp resolve", mac)
}

async fn device(
    platform: &dyn Platform,
    netio: &dyn NetIo,
    cfg: &MonitorConfig,
    faults: &Faults,
    d: &StarredProbe,
    at: i64,
) -> DeviceTick {
    let rtt = ping(platform, faults, d.ip, cfg).await;
    // Only the device's own MAC counts: where it has none stored, any reply.
    let arp = match rtt {
        Some(_) => None,
        None => arp_mac(platform, faults, d.ip, cfg)
            .await
            .map(|m| m.is_some_and(|m| d.mac.is_none_or(|own| own == m))),
    };
    let mut checks = Vec::new();
    for check in d.checks.iter().take(cfg.max_checks) {
        checks.push(run_check(platform, netio, cfg, d.ip, check, at).await);
    }
    DeviceTick {
        id: d.id,
        rtt,
        arp,
        checks,
    }
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

async fn run_check(
    platform: &dyn Platform,
    netio: &dyn NetIo,
    cfg: &MonitorConfig,
    ip: Ipv4Addr,
    check: &Check,
    at: i64,
) -> CheckOutcome {
    let (ok, detail) = match check {
        Check::Tcp { port } => {
            match platform
                .tcp_probe(ip, *port, cfg.tcp_fallback_timeout)
                .await
            {
                Ok(TcpProbe::Open) => (true, "open".to_string()),
                Ok(_) => (false, "no answer".to_string()),
                Err(e) => (false, format!("error: {}", cut(&e.to_string()))),
            }
        }
        Check::Http { port, path, https } => {
            let scheme = if *https { "https" } else { "http" };
            let url = format!("{scheme}://{ip}:{port}{path}");
            match netio.http_get(&url, true).await {
                Ok(r) => (r.status == 200, format!("HTTP {}", r.status)),
                Err(e) => (false, format!("error: {}", cut(&e.to_string()))),
            }
        }
    };
    CheckOutcome {
        check: check.clone(),
        ok,
        at,
        detail,
    }
}

/// Ping every address (`ping_concurrency` at a time), then ARP-resolve the
/// ones that did not answer (`arp_concurrency` at a time). Returns the ids
/// that answered either.
pub(crate) async fn sweep(
    platform: &dyn Platform,
    cfg: &MonitorConfig,
    faults: &Faults,
    targets: Vec<(DeviceId, Ipv4Addr)>,
) -> Vec<(DeviceId, Ipv4Addr)> {
    let pinged: Vec<((DeviceId, Ipv4Addr), bool)> = stream::iter(targets)
        .map(|t| async move { (t, ping(platform, faults, t.1, cfg).await.is_some()) })
        .buffer_unordered(cfg.presence_ping_concurrency.max(1))
        .collect()
        .await;
    let (mut seen, silent): (Vec<_>, Vec<_>) = pinged.into_iter().partition(|(_, ok)| *ok);
    let resolved: Vec<((DeviceId, Ipv4Addr), bool)> = stream::iter(silent)
        .map(|(t, _)| async move { (t, arp(platform, faults, t.1, cfg).await == Some(true)) })
        .buffer_unordered(cfg.presence_arp_concurrency.max(1))
        .collect()
        .await;
    seen.extend(resolved.into_iter().filter(|(_, ok)| *ok));
    seen.into_iter().map(|(t, _)| t).collect()
}
