//! Discovery orchestration: mDNS, SSDP, NetBIOS and reverse DNS run concurrently
//! and are folded into per-address [`Signals`].

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::time::Duration;

use futures::stream::{self, StreamExt};
use netlantern_platform::{Interface, NetIo, PlatformError};

use crate::identify::classify::Signals;
use crate::identify::upnp::{parse_upnp_description, UpnpDescription};

/// Timing and concurrency for one discovery pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoverConfig {
    /// How long mDNS and SSDP listen.
    pub window: Duration,
    /// Per-address NetBIOS node-status timeout.
    pub netbios_timeout: Duration,
    /// Per-fetch timeout for UPnP device descriptions.
    pub description_timeout: Duration,
    /// Maximum in-flight NetBIOS, reverse DNS and description requests.
    pub concurrency: usize,
}

impl Default for DiscoverConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(3),
            netbios_timeout: Duration::from_secs(1),
            description_timeout: Duration::from_secs(2),
            concurrency: 32,
        }
    }
}

/// Slack on top of a listen window or per-request timeout before a discovery
/// call that has not returned is abandoned.
const LISTEN_GRACE: Duration = Duration::from_secs(1);
const REQUEST_GRACE: Duration = Duration::from_millis(500);
/// How long one reverse DNS lookup may take before it is abandoned.
const REVERSE_DNS_TIMEOUT: Duration = Duration::from_millis(1500);

/// Runs `call`, turning a call that outlives `limit` into a timeout error so a
/// hung method cannot hold up the scan.
async fn bounded<T>(
    limit: Duration,
    call: impl std::future::Future<Output = Result<T, PlatformError>>,
) -> Result<T, PlatformError> {
    tokio::time::timeout(limit, call).await.unwrap_or_else(|_| {
        Err(PlatformError::Io {
            message: "timed out".into(),
        })
    })
}

/// Gather identity signals for the `present` addresses. Never fails: a method
/// that errors or does not return in time is logged once at debug level and
/// contributes nothing. Every
/// address in `present` gets an entry, empty when nothing was learned.
pub async fn discover(
    netio: &dyn NetIo,
    iface: &Interface,
    present: &[Ipv4Addr],
    cfg: &DiscoverConfig,
) -> HashMap<Ipv4Addr, Signals> {
    let present_set: HashSet<Ipv4Addr> = present.iter().copied().collect();
    let concurrency = cfg.concurrency.max(1);

    let mdns = async {
        match bounded(
            cfg.window + LISTEN_GRACE,
            netio.mdns_browse(iface.ip, cfg.window),
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                log_failure("mdns_browse", &e);
                Vec::new()
            }
        }
    };

    let ssdp = async {
        let replies = match bounded(
            cfg.window + LISTEN_GRACE,
            netio.ssdp_search(iface.ip, cfg.window),
        )
        .await
        {
            Ok(v) => v,
            Err(e) => {
                log_failure("ssdp_search", &e);
                return (Vec::new(), HashMap::new());
            }
        };
        let replies: Vec<_> = replies
            .into_iter()
            .filter(|r| present_set.contains(&r.ip))
            .collect();
        let mut seen = HashSet::new();
        let locations: Vec<String> = replies
            .iter()
            .filter_map(|r| r.location.as_deref().map(|l| (r.ip, l)))
            .filter(|(ip, l)| is_own_private_http_location(l, *ip))
            .filter(|(_, l)| seen.insert(l.to_string()))
            .map(|(_, l)| l.to_string())
            .collect();
        let mut failed = None;
        let fetched: Vec<(String, Option<UpnpDescription>)> = stream::iter(locations)
            .map(|loc| async move {
                let res =
                    tokio::time::timeout(cfg.description_timeout, netio.http_get(&loc, false))
                        .await;
                (loc, res)
            })
            .buffer_unordered(concurrency)
            .map(|(loc, res)| match res {
                Ok(Ok(r)) if r.status == 200 => (loc, Some(parse_upnp_description(&r.body_prefix))),
                Ok(Ok(_)) => (loc, None),
                Ok(Err(e)) => {
                    failed.get_or_insert(e.to_string());
                    (loc, None)
                }
                Err(_) => {
                    failed.get_or_insert("timed out".to_string());
                    (loc, None)
                }
            })
            .collect()
            .await;
        if let Some(e) = failed {
            tracing::debug!(method = "http_get", error = %e, "discovery method failed");
        }
        (replies, fetched.into_iter().collect::<HashMap<_, _>>())
    };

    let netbios = async {
        let mut failed = None;
        let out: Vec<_> = stream::iter(present.iter().copied().filter(|ip| *ip != iface.ip))
            .map(|ip| async move {
                let limit = cfg.netbios_timeout + REQUEST_GRACE;
                (
                    ip,
                    bounded(limit, netio.netbios_status(ip, cfg.netbios_timeout)).await,
                )
            })
            .buffer_unordered(concurrency)
            .filter_map(|(ip, res)| {
                let item = match res {
                    Ok(Some(info)) => Some((ip, info)),
                    Ok(None) => None,
                    Err(e) => {
                        failed.get_or_insert(e.to_string());
                        None
                    }
                };
                std::future::ready(item)
            })
            .collect()
            .await;
        if let Some(e) = failed {
            tracing::debug!(method = "netbios_status", error = %e, "discovery method failed");
        }
        out
    };

    let rdns = async {
        let mut failed = None;
        let out: Vec<_> = stream::iter(present.iter().copied())
            .map(|ip| async move {
                let res = bounded(REVERSE_DNS_TIMEOUT, netio.reverse_dns(iface.gateway, ip)).await;
                (ip, res)
            })
            .buffer_unordered(concurrency)
            .filter_map(|(ip, res)| {
                let item = match res {
                    Ok(Some(name)) => Some((ip, name)),
                    Ok(None) => None,
                    Err(e) => {
                        failed.get_or_insert(e.to_string());
                        None
                    }
                };
                std::future::ready(item)
            })
            .collect()
            .await;
        if let Some(e) = failed {
            tracing::debug!(method = "reverse_dns", error = %e, "discovery method failed");
        }
        out
    };

    let (mdns, (ssdp_replies, descriptions), netbios, rdns) =
        tokio::join!(mdns, ssdp, netbios, rdns);

    let mut found: HashMap<Ipv4Addr, Signals> =
        present.iter().map(|ip| (*ip, Signals::default())).collect();
    for service in mdns {
        if let Some(s) = found.get_mut(&service.ip) {
            s.mdns.push(service);
        }
    }
    for reply in ssdp_replies {
        let description = reply
            .location
            .as_ref()
            .and_then(|l| descriptions.get(l))
            .cloned()
            .flatten();
        if let Some(s) = found.get_mut(&reply.ip) {
            s.ssdp.push((reply, description));
        }
    }
    for (ip, info) in netbios {
        if let Some(s) = found.get_mut(&ip) {
            s.netbios = Some(info);
        }
    }
    for (ip, name) in rdns {
        if let Some(s) = found.get_mut(&ip) {
            s.rdns = Some(name);
        }
    }
    found
}

fn log_failure(method: &str, error: &PlatformError) {
    tracing::debug!(method, %error, "discovery method failed");
}

/// True for `http://<ip>[:port][/...]` where the host is exactly `ip` and `ip`
/// is an RFC 1918 address. Anything else (other schemes, names, userinfo,
/// other hosts, public addresses) is refused so a device cannot steer fetches
/// off the LAN.
fn is_own_private_http_location(location: &str, ip: Ipv4Addr) -> bool {
    let Some(rest) = location.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (authority, None),
    };
    if let Some(p) = port {
        if p.parse::<u16>().is_err() {
            return false;
        }
    }
    matches!(host.parse::<Ipv4Addr>(), Ok(h) if h == ip && h.is_private())
}
