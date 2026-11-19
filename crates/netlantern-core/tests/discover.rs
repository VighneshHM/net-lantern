//! Discovery orchestration against the simulated home network and test doubles.

use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use netlantern_core::discover::{discover, DiscoverConfig};
use netlantern_platform::{
    HttpResponse, Interface, MdnsHost, MdnsService, NetIo, NetbiosInfo, Platform, PlatformError,
    SimNetwork, SsdpReply,
};

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn home() -> SimNetwork {
    SimNetwork::load(format!(
        "{}/../../scenarios/home.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn present_home() -> Vec<Ipv4Addr> {
    [1, 23, 31, 44, 52, 57, 61, 70, 88, 95, 102, 120, 130]
        .iter()
        .map(|n| Ipv4Addr::new(192, 168, 1, *n))
        .collect()
}

struct FailingNetIo;

fn fail() -> PlatformError {
    PlatformError::Io {
        message: "boom".into(),
    }
}

#[async_trait]
impl NetIo for FailingNetIo {
    async fn dns_resolve(&self, _: Ipv4Addr, _: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
        Err(fail())
    }
    async fn http_get(&self, _: &str, _: bool) -> Result<HttpResponse, PlatformError> {
        Err(fail())
    }
    async fn reverse_dns(&self, _: Ipv4Addr, _: Ipv4Addr) -> Result<Option<String>, PlatformError> {
        Err(fail())
    }
    async fn mdns_browse(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        Err(fail())
    }
    async fn ssdp_search(&self, _: Ipv4Addr, _: Duration) -> Result<Vec<SsdpReply>, PlatformError> {
        Err(fail())
    }
    async fn netbios_status(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        Err(fail())
    }
}

/// Replies to SSDP with canned replies and records every HTTP GET.
struct RecordingNetIo {
    replies: Vec<SsdpReply>,
    gets: Mutex<Vec<(String, bool)>>,
    mdns_hosts: Vec<MdnsHost>,
    mdns_browse: Vec<MdnsService>,
    mdns_asked: Mutex<Vec<Ipv4Addr>>,
}

impl RecordingNetIo {
    fn new(replies: Vec<SsdpReply>) -> Self {
        Self {
            replies,
            gets: Mutex::new(Vec::new()),
            mdns_hosts: Vec::new(),
            mdns_browse: Vec::new(),
            mdns_asked: Mutex::new(Vec::new()),
        }
    }
    fn gets(&self) -> Vec<(String, bool)> {
        self.gets.lock().unwrap().clone()
    }
}

#[async_trait]
impl NetIo for RecordingNetIo {
    async fn dns_resolve(&self, _: Ipv4Addr, _: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
        Err(fail())
    }
    async fn http_get(&self, url: &str, follow: bool) -> Result<HttpResponse, PlatformError> {
        self.gets.lock().unwrap().push((url.to_string(), follow));
        Ok(HttpResponse {
            status: 200,
            location: None,
            server: None,
            body_prefix: "<root><device><friendlyName>X</friendlyName></device></root>".into(),
        })
    }
    async fn reverse_dns(&self, _: Ipv4Addr, _: Ipv4Addr) -> Result<Option<String>, PlatformError> {
        Ok(None)
    }
    async fn mdns_browse(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        Ok(self.mdns_browse.clone())
    }
    async fn mdns_query(
        &self,
        ip: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<MdnsHost>, PlatformError> {
        self.mdns_asked.lock().unwrap().push(ip);
        Ok(self.mdns_hosts.iter().find(|h| h.ip == ip).cloned())
    }
    async fn ssdp_search(&self, _: Ipv4Addr, _: Duration) -> Result<Vec<SsdpReply>, PlatformError> {
        Ok(self.replies.clone())
    }
    async fn netbios_status(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        Ok(None)
    }
}

fn reply(from: &str, location: &str) -> SsdpReply {
    SsdpReply {
        ip: ip(from),
        st: "upnp:rootdevice".into(),
        server: None,
        location: Some(location.into()),
    }
}

async fn run_recording(replies: Vec<SsdpReply>) -> (RecordingNetIo, Interface) {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let rec = RecordingNetIo::new(replies);
    let _ = discover(&rec, &iface, &present_home(), &DiscoverConfig::default()).await;
    (rec, iface)
}

#[tokio::test(start_paused = true)]
async fn gathers_home_identities_within_the_window() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let start = tokio::time::Instant::now();
    let found = discover(&sim, &iface, &present_home(), &DiscoverConfig::default()).await;
    assert!(start.elapsed() <= secs(3) + secs(2));
    assert_eq!(found.len(), 13);
    assert_eq!(
        found[&ip("192.168.1.70")].netbios.as_ref().unwrap().name,
        "DISKSTATION"
    );
    assert_eq!(
        found[&ip("192.168.1.52")].ssdp[0]
            .1
            .as_ref()
            .unwrap()
            .friendly_name
            .as_deref(),
        Some("[TV] Living Room")
    );
    assert_eq!(
        found[&ip("192.168.1.61")].rdns.as_deref(),
        Some("echo-dot.lan")
    );
}

#[tokio::test(start_paused = true)]
async fn failing_netio_yields_empty_signals_not_an_error() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let found = discover(
        &FailingNetIo,
        &iface,
        &present_home(),
        &DiscoverConfig::default(),
    )
    .await;
    assert_eq!(found.len(), 13);
    assert!(found.values().all(|s| s.mdns.is_empty()
        && s.netbios.is_none()
        && s.ssdp.is_empty()
        && s.rdns.is_none()));
}

#[tokio::test(start_paused = true)]
async fn never_fetches_a_location_on_another_host() {
    let (rec, _) = run_recording(vec![
        reply("192.168.1.52", "http://203.0.113.9/x.xml"),
        reply("192.168.1.52", "https://192.168.1.52/x.xml"),
        reply("192.168.1.52", "http://192.168.1.53/x.xml"),
        reply("192.168.1.52", "http://192.168.1.52@203.0.113.9/x.xml"),
    ])
    .await;
    assert!(rec.gets().is_empty(), "{:?}", rec.gets());
}

#[tokio::test(start_paused = true)]
async fn fetches_own_private_location_once_without_redirects() {
    let loc = "http://192.168.1.52:8080/desc.xml";
    let (rec, _) =
        run_recording(vec![reply("192.168.1.52", loc), reply("192.168.1.52", loc)]).await;
    assert_eq!(rec.gets(), vec![(loc.to_string(), false)]);
}

#[tokio::test(start_paused = true)]
async fn never_fetches_a_public_ip_even_when_it_matches_the_reply() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let rec = RecordingNetIo::new(vec![reply("203.0.113.9", "http://203.0.113.9/x.xml")]);
    let present = vec![ip("203.0.113.9")];
    let _ = discover(&rec, &iface, &present, &DiscoverConfig::default()).await;
    assert!(rec.gets().is_empty());
}

/// Every discovery call hangs forever.
struct HangingNetIo;

#[async_trait]
impl NetIo for HangingNetIo {
    async fn dns_resolve(&self, _: Ipv4Addr, _: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
        std::future::pending().await
    }
    async fn http_get(&self, _: &str, _: bool) -> Result<HttpResponse, PlatformError> {
        std::future::pending().await
    }
    async fn reverse_dns(&self, _: Ipv4Addr, _: Ipv4Addr) -> Result<Option<String>, PlatformError> {
        std::future::pending().await
    }
    async fn mdns_browse(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        std::future::pending().await
    }
    async fn mdns_query(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<MdnsHost>, PlatformError> {
        std::future::pending().await
    }
    async fn ssdp_search(&self, _: Ipv4Addr, _: Duration) -> Result<Vec<SsdpReply>, PlatformError> {
        std::future::pending().await
    }
    async fn netbios_status(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn hung_methods_are_abandoned_within_their_bounds() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let cfg = DiscoverConfig::default();
    let start = tokio::time::Instant::now();
    let found = discover(&HangingNetIo, &iface, &present_home(), &cfg).await;
    // mDNS and SSDP are abandoned a second after their window; NetBIOS and
    // reverse DNS (all 13 addresses fit in one batch) well before that.
    assert_eq!(start.elapsed(), cfg.window + secs(1));
    assert_eq!(found.len(), 13);
    assert!(found.values().all(|s| s.mdns.is_empty()
        && s.netbios.is_none()
        && s.ssdp.is_empty()
        && s.rdns.is_none()));
}

#[tokio::test(start_paused = true)]
async fn hung_netbios_and_reverse_dns_are_abandoned_per_request() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    // One request at a time: 12 NetBIOS requests (not this laptop) at 1.5 s each,
    // 13 reverse DNS lookups at 1.5 s each, run side by side.
    let cfg = DiscoverConfig {
        window: Duration::ZERO,
        mdns_query_timeout: Duration::ZERO,
        concurrency: 1,
        ..DiscoverConfig::default()
    };
    let start = tokio::time::Instant::now();
    let found = discover(&HangingNetIo, &iface, &present_home(), &cfg).await;
    assert_eq!(start.elapsed(), Duration::from_millis(13 * 1500));
    assert_eq!(found.len(), 13);
}

fn companion(instance: &str) -> MdnsService {
    MdnsService {
        ip: ip("192.168.1.23"),
        service_type: "_companion-link._tcp".into(),
        instance: instance.into(),
        hostname: None,
        friendly_name: None,
        model: None,
    }
}

#[tokio::test(start_paused = true)]
async fn unicast_mdns_fills_host_and_services() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let mut rec = RecordingNetIo::new(vec![]);
    rec.mdns_hosts = [ip("192.168.1.23"), iface.ip]
        .into_iter()
        .map(|a| MdnsHost {
            ip: a,
            hostname: Some("Johns-iPhone.local".into()),
            services: vec![companion("Johns-iPhone")],
        })
        .collect();
    rec.mdns_browse = vec![companion("Johns-iPhone")];
    let present = vec![ip("192.168.1.23"), ip("192.168.1.31")];
    let found = discover(&rec, &iface, &present, &DiscoverConfig::default()).await;
    let phone = &found[&ip("192.168.1.31")];
    assert!(phone.mdns_host.is_none() && phone.mdns.is_empty());
    let own = &found[&ip("192.168.1.23")];
    assert_eq!(own.mdns_host, None, "own address is never queried");

    // Same again with the phone at a non-local address.
    let mut rec = RecordingNetIo::new(vec![]);
    let phone_ip = ip("192.168.1.31");
    let mut svc = companion("Johns-iPhone");
    svc.ip = phone_ip;
    rec.mdns_hosts = vec![MdnsHost {
        ip: phone_ip,
        hostname: Some("Johns-iPhone.local".into()),
        services: vec![svc.clone()],
    }];
    rec.mdns_browse = vec![svc];
    let found = discover(&rec, &iface, &present, &DiscoverConfig::default()).await;
    let phone = &found[&phone_ip];
    assert_eq!(phone.mdns_host.as_deref(), Some("Johns-iPhone.local"));
    assert_eq!(phone.mdns.len(), 1, "no duplicate of the browsed service");
    let asked = rec.mdns_asked.lock().unwrap().clone();
    assert!(!asked.contains(&iface.ip));
    assert!(asked.contains(&phone_ip));
}

#[tokio::test(start_paused = true)]
async fn a_hanging_mdns_query_costs_at_most_its_timeout() {
    let sim = home();
    let iface = sim.active_interface().await.unwrap();
    let cfg = DiscoverConfig::default();
    let start = tokio::time::Instant::now();
    let found = discover(&HangingNetIo, &iface, &present_home(), &cfg).await;
    assert!(start.elapsed() <= cfg.window + secs(1));
    assert!(found.values().all(|s| s.mdns_host.is_none()));
}
