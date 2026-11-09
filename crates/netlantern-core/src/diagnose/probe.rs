//! The probes: one gather function per layer, asking the platform and the
//! network what the judges need.
//!
//! Each layer's work fills a [`Progress`] as checks finish. The progress
//! starts as the all-failed probe (no ping replies, ports closed, requests
//! "timed out"), so when the runner cuts a layer off at its time limit, what
//! is left in it is the layer judged with every unfinished check failed.

use std::net::Ipv4Addr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::future::join_all;
use netlantern_platform::{Interface, InterfaceKind, NetIo, Platform, PlatformError, TcpProbe};

use super::judge::classify_portal;
use super::model::*;
use crate::wifi::WifiService;

/// What an unfinished request reports.
const TIMED_OUT: &str = "timed out";

/// A layer's probe as it fills in, and the checks not finished yet (as
/// evidence labels).
pub(crate) struct Progress<P> {
    state: Mutex<(P, Vec<String>)>,
}

impl<P> Progress<P> {
    pub(crate) fn new(initial: P, unfinished: Vec<String>) -> Self {
        Self {
            state: Mutex::new((initial, unfinished)),
        }
    }

    fn lock(&self) -> MutexGuard<'_, (P, Vec<String>)> {
        // Every update is a single assignment, so a poisoned lock holds no
        // half-written state.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, change: impl FnOnce(&mut P)) {
        change(&mut self.lock().0);
    }

    fn begin(&self, check: String) {
        self.lock().1.push(check);
    }

    fn finish(&self, check: &str) {
        self.lock().1.retain(|c| c != check);
    }

    /// The probe, and one "timed out" evidence line per unfinished check.
    pub(crate) fn into_parts(self) -> (P, Vec<String>) {
        let (probe, unfinished) = self
            .state
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        let lines = unfinished
            .into_iter()
            .map(|c| format!("{c}: {TIMED_OUT}"))
            .collect();
        (probe, lines)
    }
}

fn rtt_ms(rtt: Duration) -> u32 {
    u32::try_from(rtt.as_millis()).unwrap_or(u32::MAX)
}

// ---- Link ----------------------------------------------------------------

const READ_INTERFACE: &str = "Reading the network interface";
const READ_WIFI: &str = "Reading the Wi-Fi connection";
const SCAN_NEARBY: &str = "Scanning nearby networks";

/// The link probe, the interface when connected, and whether the link layer
/// ran a fresh radio scan (or was cut off during one).
pub(crate) type LinkFill = (LinkProbe, Option<Interface>, bool);

pub(crate) fn link_progress() -> Progress<LinkFill> {
    let probe = LinkProbe {
        interface: None,
        wifi: WifiReading::Disconnected,
        congestion: None,
        congestion_advice: None,
    };
    Progress::new((probe, None, false), vec![READ_INTERFACE.into()])
}

pub(crate) async fn fill_link(
    progress: &Progress<LinkFill>,
    platform: &dyn Platform,
    wifi: &WifiService,
    cfg: &DiagnoseConfig,
) {
    let iface = match platform.active_interface().await {
        Ok(iface) => Some(iface),
        Err(PlatformError::NotConnected) => None,
        Err(e) => {
            tracing::warn!(error = %e, "couldn't read the network interface; treated as not connected");
            None
        }
    };
    let kind = iface.as_ref().map(|i| i.kind);
    progress.update(|(probe, slot, _)| {
        probe.interface = kind;
        probe.wifi = WifiReading::NotWifi;
        *slot = iface;
    });
    progress.finish(READ_INTERFACE);
    match kind {
        None => {
            progress.update(|(probe, _, _)| probe.wifi = WifiReading::Disconnected);
            return;
        }
        Some(InterfaceKind::Wifi) => {}
        Some(_) => return,
    }

    progress.update(|(probe, _, _)| probe.wifi = WifiReading::Unavailable(TIMED_OUT.into()));
    progress.begin(READ_WIFI.into());
    let reading = match platform.wifi_current().await {
        Ok(Some(conn)) => WifiReading::Connected(conn),
        Ok(None) => WifiReading::Disconnected,
        Err(PlatformError::PermissionNeeded { .. }) => WifiReading::PermissionNeeded,
        Err(e) => WifiReading::Unavailable(e.to_string()),
    };
    let connected = matches!(reading, WifiReading::Connected(_));
    progress.update(|(probe, _, _)| probe.wifi = reading);
    progress.finish(READ_WIFI);
    if !connected {
        return;
    }

    progress.begin(SCAN_NEARBY.into());
    // Assume a radio scan until the cache says otherwise, so a layer cut off
    // mid-scan still lets the radio settle.
    progress.update(|(_, _, scanned)| *scanned = true);
    match wifi.nearby_with_freshness(cfg.nearby_max_age).await {
        Ok((scan, fresh)) => progress.update(|(probe, _, scanned)| {
            probe.congestion = scan.congestion.current;
            probe.congestion_advice = Some(scan.congestion.advice);
            *scanned = fresh;
        }),
        Err(e) => {
            progress.update(|(_, _, scanned)| *scanned = false);
            tracing::warn!(error = %e, "nearby-network scan failed during a diagnosis");
        }
    }
    progress.finish(SCAN_NEARBY);
}

/// What the laptop's link looks like, and the interface when it is connected.
pub async fn gather_link(
    platform: &dyn Platform,
    wifi: &WifiService,
    cfg: &DiagnoseConfig,
) -> (LinkProbe, Option<Interface>) {
    let progress = link_progress();
    fill_link(&progress, platform, wifi, cfg).await;
    let (probe, iface, _) = progress.into_parts().0;
    (probe, iface)
}

// ---- Router --------------------------------------------------------------

pub(crate) fn router_progress(gateway: Ipv4Addr, cfg: &DiagnoseConfig) -> Progress<RouterProbe> {
    let probe = RouterProbe {
        pings: PingStats {
            target: gateway,
            sent: cfg.gateway_pings,
            rtts_ms: Vec::new(),
        },
        answers_arp: Some(false),
    };
    Progress::new(
        probe,
        vec![pings_check(gateway), format!("ARP request to {gateway}")],
    )
}

fn pings_check(target: Ipv4Addr) -> String {
    format!("Pings to {target}")
}

/// Pings `target` `count` times, one after another, adding each round trip
/// to the stats `record` picks out of the progress.
async fn ping_series<P>(
    progress: &Progress<P>,
    record: impl Fn(&mut P) -> &mut PingStats,
    platform: &dyn Platform,
    target: Ipv4Addr,
    count: u32,
    timeout: Duration,
) {
    for _ in 0..count {
        match platform.ping(target, timeout).await {
            Ok(Some(reply)) => progress.update(|p| record(p).rtts_ms.push(rtt_ms(reply.rtt))),
            Ok(None) => {}
            Err(e) => tracing::warn!(%target, error = %e, "ping failed during a diagnosis"),
        }
    }
    progress.finish(&pings_check(target));
}

pub(crate) async fn fill_router(
    progress: &Progress<RouterProbe>,
    platform: &dyn Platform,
    gateway: Ipv4Addr,
    cfg: &DiagnoseConfig,
) {
    ping_series(
        progress,
        |p| &mut p.pings,
        platform,
        gateway,
        cfg.gateway_pings,
        cfg.gateway_ping_timeout,
    )
    .await;
    let arp_check = format!("ARP request to {gateway}");
    let replied = {
        let state = progress.lock();
        state.0.pings.replies() > 0
    };
    if replied {
        progress.update(|p| p.answers_arp = None);
        progress.finish(&arp_check);
        return;
    }
    let answers = match platform.arp_resolve(gateway, cfg.arp_timeout).await {
        Ok(mac) => mac.is_some(),
        Err(e) => {
            tracing::warn!(%gateway, error = %e, "ARP request failed during a diagnosis");
            false
        }
    };
    progress.update(|p| p.answers_arp = Some(answers));
    progress.finish(&arp_check);
}

/// Whether the router answers: pings first, ARP only when no ping replied.
pub async fn gather_router(
    platform: &dyn Platform,
    gateway: Ipv4Addr,
    cfg: &DiagnoseConfig,
) -> RouterProbe {
    let progress = router_progress(gateway, cfg);
    fill_router(&progress, platform, gateway, cfg).await;
    progress.into_parts().0
}

// ---- Internet ------------------------------------------------------------

fn tcp_check(cfg: &DiagnoseConfig) -> String {
    let (target, port) = cfg.tcp_target;
    format!("TCP {port} to {target}")
}

pub(crate) fn internet_progress(cfg: &DiagnoseConfig) -> Progress<InternetProbe> {
    let (target, port) = cfg.tcp_target;
    let probe = InternetProbe {
        pings: cfg
            .internet_targets
            .iter()
            .map(|&target| PingStats {
                target,
                sent: cfg.internet_pings,
                rtts_ms: Vec::new(),
            })
            .collect(),
        tcp: TcpCheck {
            target,
            port,
            open: false,
        },
        // Already reads "timed out" in the evidence, so it needs no label.
        portal: PortalCheck::Failed(TIMED_OUT.into()),
    };
    let mut unfinished: Vec<String> = cfg
        .internet_targets
        .iter()
        .map(|&t| pings_check(t))
        .collect();
    unfinished.push(tcp_check(cfg));
    Progress::new(probe, unfinished)
}

pub(crate) async fn fill_internet(
    progress: &Progress<InternetProbe>,
    platform: &dyn Platform,
    netio: &dyn NetIo,
    cfg: &DiagnoseConfig,
) {
    let pings = join_all(cfg.internet_targets.iter().enumerate().map(|(i, &target)| {
        ping_series(
            progress,
            move |p: &mut InternetProbe| &mut p.pings[i],
            platform,
            target,
            cfg.internet_pings,
            cfg.internet_ping_timeout,
        )
    }));
    let tcp = async {
        let (target, port) = cfg.tcp_target;
        let open = match platform.tcp_probe(target, port, cfg.tcp_timeout).await {
            Ok(probe) => probe == TcpProbe::Open,
            Err(e) => {
                tracing::warn!(%target, port, error = %e, "TCP probe failed during a diagnosis");
                false
            }
        };
        progress.update(|p| p.tcp.open = open);
        progress.finish(&tcp_check(cfg));
    };
    let portal = async {
        let res = netio.http_get(&cfg.portal_url, false).await;
        let check = classify_portal(&res, &cfg.portal_expected_body);
        progress.update(|p| p.portal = check);
    };
    tokio::join!(pings, tcp, portal);
}

/// Whether the internet answers: pings, a TCP connect and the connectivity
/// check, all at once.
pub async fn gather_internet(
    platform: &dyn Platform,
    netio: &dyn NetIo,
    cfg: &DiagnoseConfig,
) -> InternetProbe {
    let progress = internet_progress(cfg);
    fill_internet(&progress, platform, netio, cfg).await;
    progress.into_parts().0
}

// ---- DNS -----------------------------------------------------------------

fn timed_out_lookup(name: &str, server: Ipv4Addr) -> DnsLookup {
    DnsLookup {
        name: name.into(),
        server,
        result: Err(TIMED_OUT.into()),
    }
}

pub(crate) fn dns_progress(dns_servers: &[Ipv4Addr], cfg: &DiagnoseConfig) -> Progress<DnsProbe> {
    let system = match dns_servers.first() {
        Some(&first) => cfg
            .dns_names
            .iter()
            .map(|n| timed_out_lookup(n, first))
            .collect(),
        None => Vec::new(),
    };
    let probe = DnsProbe {
        system_servers: dns_servers.to_vec(),
        direct_server: cfg.direct_dns,
        system,
        direct: cfg
            .dns_names
            .iter()
            .map(|n| timed_out_lookup(n, cfg.direct_dns))
            .collect(),
    };
    // Unfinished lookups already read "timed out", so no labels are needed.
    Progress::new(probe, Vec::new())
}

async fn lookup(netio: &dyn NetIo, server: Ipv4Addr, name: &str) -> DnsLookup {
    let result = netio
        .dns_resolve(server, name)
        .await
        .map_err(|e| e.to_string());
    DnsLookup {
        name: name.into(),
        server,
        result,
    }
}

pub(crate) async fn fill_dns(
    progress: &Progress<DnsProbe>,
    netio: &dyn NetIo,
    dns_servers: &[Ipv4Addr],
    cfg: &DiagnoseConfig,
) {
    let names = cfg
        .dns_names
        .iter()
        .enumerate()
        .map(|(i, name)| async move {
            let system = async {
                for &server in dns_servers {
                    // Until it answers, the unfinished lookup names this server.
                    progress.update(|p| p.system[i] = timed_out_lookup(name, server));
                    let l = lookup(netio, server, name).await;
                    let answered = l.result.as_ref().is_ok_and(|a| !a.is_empty());
                    progress.update(|p| p.system[i] = l);
                    if answered {
                        break;
                    }
                }
            };
            let direct = async {
                let l = lookup(netio, cfg.direct_dns, name).await;
                progress.update(|p| p.direct[i] = l);
            };
            tokio::join!(system, direct);
        });
    join_all(names).await;
}

/// Each test name looked up through the system servers (in order, until one
/// answers) and directly through `cfg.direct_dns`.
pub async fn gather_dns(
    netio: &dyn NetIo,
    dns_servers: &[Ipv4Addr],
    cfg: &DiagnoseConfig,
) -> DnsProbe {
    let progress = dns_progress(dns_servers, cfg);
    fill_dns(&progress, netio, dns_servers, cfg).await;
    progress.into_parts().0
}

// ---- Website -------------------------------------------------------------

pub(crate) fn website_progress(cfg: &DiagnoseConfig) -> Progress<WebsiteProbe> {
    let probe = WebsiteProbe {
        url: cfg.website_url.clone(),
        outcome: WebsiteOutcome::Failed(TIMED_OUT.into()),
    };
    Progress::new(probe, Vec::new())
}

pub(crate) async fn fill_website(
    progress: &Progress<WebsiteProbe>,
    netio: &dyn NetIo,
    cfg: &DiagnoseConfig,
) {
    let outcome = match netio.http_get(&cfg.website_url, false).await {
        Ok(r) => WebsiteOutcome::Status(r.status),
        Err(PlatformError::TlsCertificate { message }) => WebsiteOutcome::Certificate(message),
        Err(e) => WebsiteOutcome::Failed(e.to_string()),
    };
    progress.update(|p| p.outcome = outcome);
}

/// Whether a secure website loads.
pub async fn gather_website(netio: &dyn NetIo, cfg: &DiagnoseConfig) -> WebsiteProbe {
    let progress = website_progress(cfg);
    fill_website(&progress, netio, cfg).await;
    progress.into_parts().0
}
