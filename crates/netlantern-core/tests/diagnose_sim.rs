//! End-to-end diagnosis runs against the simulated networks in `scenarios/`.
//!
//! Every test runs with a paused clock, so simulated pings, DNS waits, radio
//! scans and the per-layer time limit cost no real time.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use netlantern_core::{
    gather_dns, CongestionConfig, DiagnoseConfig, DiagnoseManager, DiagnoseStart, DiagnosisId,
    DiagnosisRun, Event, EventBus, EventKind, Finding, Layer, LayerStatus, RunState, WifiService,
};
use netlantern_platform::{
    ArpEntry, HttpResponse, Interface, MacAddr, MdnsService, NearbyNetwork, NetIo, NetbiosInfo,
    PingReply, Platform, PlatformError, SimNetwork, SsdpReply, TcpProbe, WifiConnection,
};
use tokio::sync::broadcast;

fn scenario_path(name: &str) -> String {
    format!("{}/../../scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"))
}

fn scenario(name: &str) -> SimNetwork {
    SimNetwork::load(scenario_path(name)).unwrap()
}

fn manager(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>) -> (DiagnoseManager, Arc<EventBus>) {
    let bus = Arc::new(EventBus::new(256));
    let wifi = Arc::new(WifiService::new(
        Arc::clone(&platform),
        CongestionConfig::default(),
    ));
    let mgr = DiagnoseManager::new(
        platform,
        netio,
        wifi,
        Arc::clone(&bus),
        DiagnoseConfig::default(),
    );
    (mgr, bus)
}

fn sim_manager(name: &str) -> (DiagnoseManager, Arc<EventBus>) {
    let sim = Arc::new(scenario(name));
    manager(sim.clone(), sim)
}

fn started(outcome: DiagnoseStart) -> DiagnosisId {
    match outcome {
        DiagnoseStart::Started(id) => id,
        other => panic!("expected a new run to start, got {other:?}"),
    }
}

async fn run_with(mgr: &DiagnoseManager) -> DiagnosisRun {
    let id = started(mgr.start().await);
    mgr.wait(id).await.unwrap()
}

async fn run(name: &str) -> DiagnosisRun {
    let (mgr, _bus) = sim_manager(name);
    run_with(&mgr).await
}

async fn next(rx: &mut broadcast::Receiver<Event>) -> Event {
    rx.recv().await.unwrap()
}

#[tokio::test(start_paused = true)]
async fn every_scenario_gets_its_verdict() {
    for (name, finding) in [
        ("home", Finding::Healthy),
        ("weak-wifi", Finding::WeakAndCrowded),
        ("dns-failure", Finding::DnsServerProblem),
        ("provider-down", Finding::InternetDown),
        ("cafe", Finding::CaptivePortal),
        ("tls-intercept", Finding::TlsIntercepted),
        ("router-down", Finding::RouterUnreachable),
        ("router-silent", Finding::RouterIgnoresPing),
        ("location-denied", Finding::Healthy),
        ("offline", Finding::NotConnected),
    ] {
        let r = run(name).await;
        assert_eq!(r.state, RunState::Done, "{name}");
        assert_eq!(r.verdict.unwrap().finding, finding, "{name}");
        assert_eq!(r.steps.len(), 5, "{name}: always five steps");
        assert!(r.finished_at.unwrap() >= r.started_at, "{name}");
        assert_eq!(r.error, None, "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn location_denied_passes_the_link_with_the_signal_hidden() {
    let r = run("location-denied").await;
    assert_eq!(r.steps[0].finding, Finding::SignalHidden);
    assert_eq!(r.steps[0].status, LayerStatus::Pass);
}

#[tokio::test(start_paused = true)]
async fn offline_skips_the_four_layers_after_the_link() {
    let r = run("offline").await;
    assert_eq!(r.steps[0].status, LayerStatus::Fail);
    assert!(r.steps[1..]
        .iter()
        .all(|s| s.status == LayerStatus::Skipped && s.finding == Finding::Skipped));
}

#[tokio::test(start_paused = true)]
async fn cafe_stops_at_internet_and_skips_dns_and_website() {
    let r = run("cafe").await;
    assert_eq!(r.steps[0].status, LayerStatus::Pass);
    assert_eq!(r.steps[1].status, LayerStatus::Pass);
    assert_eq!(r.steps[2].layer, Layer::Internet);
    assert_eq!(r.steps[2].status, LayerStatus::Fail);
    assert_eq!(r.steps[2].finding, Finding::CaptivePortal);
    assert!(r.steps[2]
        .evidence
        .iter()
        .any(|e| e.contains("http://login.cafe-wifi.example/")));
    assert_eq!(
        r.steps[3..].iter().map(|s| s.layer).collect::<Vec<_>>(),
        vec![Layer::Dns, Layer::Website]
    );
    assert!(r.steps[3..]
        .iter()
        .all(|s| s.status == LayerStatus::Skipped));
}

#[tokio::test(start_paused = true)]
async fn events_stream_five_steps_then_done_in_order() {
    let (mgr, bus) = sim_manager("home");
    let mut rx = bus.subscribe();
    let id = started(mgr.start().await);

    let mut layers = Vec::new();
    let mut last_seq = 0;
    for _ in 0..5 {
        let event = next(&mut rx).await;
        assert!(event.seq > last_seq);
        last_seq = event.seq;
        match event.kind {
            EventKind::DiagnoseStep { run_id, step } => {
                assert_eq!(run_id, id);
                layers.push(step.layer);
            }
            other => panic!("expected diagnose.step, got {other:?}"),
        }
    }
    assert_eq!(
        layers,
        vec![
            Layer::Link,
            Layer::Router,
            Layer::Internet,
            Layer::Dns,
            Layer::Website
        ]
    );
    match next(&mut rx).await.kind {
        EventKind::DiagnoseDone {
            run_id,
            state,
            verdict,
            error,
        } => {
            assert_eq!(run_id, id);
            assert_eq!(state, RunState::Done);
            assert_eq!(verdict.unwrap().finding, Finding::Healthy);
            assert_eq!(error, None);
        }
        other => panic!("expected diagnose.done, got {other:?}"),
    }
    // The run is final by the time anyone hears of it.
    assert_eq!(mgr.get(id).unwrap().state, RunState::Done);
}

#[tokio::test(start_paused = true)]
async fn second_start_while_running_returns_the_same_id() {
    let (mgr, _bus) = sim_manager("home");
    let id = started(mgr.start().await);
    assert_eq!(mgr.start().await, DiagnoseStart::AlreadyRunning(id));
    mgr.wait(id).await.unwrap();
    // Once it has finished, a new run starts with the next id.
    assert_eq!(
        mgr.start().await,
        DiagnoseStart::Started(DiagnosisId(id.0 + 1))
    );
}

#[tokio::test(start_paused = true)]
async fn ids_start_at_one_and_unknown_ids_are_none() {
    let (mgr, _bus) = sim_manager("home");
    assert_eq!(mgr.latest(), None);
    let id = started(mgr.start().await);
    assert_eq!(id, DiagnosisId(1));
    assert_eq!(mgr.get(DiagnosisId(99)), None);
    assert_eq!(mgr.wait(DiagnosisId(99)).await, None);
    mgr.wait(id).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn get_returns_steps_so_far_mid_run() {
    let (mgr, bus) = sim_manager("home");
    let mut rx = bus.subscribe();
    let id = started(mgr.start().await);
    assert!(matches!(
        next(&mut rx).await.kind,
        EventKind::DiagnoseStep { .. }
    ));
    let run = mgr.get(id).unwrap();
    assert_eq!(run.steps.len(), 1);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.verdict, None);
    assert_eq!(run.finished_at, None);
    assert_eq!(mgr.latest().unwrap().id, id);
    mgr.wait(id).await.unwrap();
}

/// A `SimNetwork` whose plain HTTP GET never returns.
struct HungHttp(SimNetwork);

#[async_trait]
impl NetIo for HungHttp {
    async fn dns_resolve(
        &self,
        server: Ipv4Addr,
        name: &str,
    ) -> Result<Vec<Ipv4Addr>, PlatformError> {
        self.0.dns_resolve(server, name).await
    }
    async fn http_get(&self, _url: &str, _follow: bool) -> Result<HttpResponse, PlatformError> {
        std::future::pending().await
    }
    async fn reverse_dns(
        &self,
        server: Ipv4Addr,
        ip: Ipv4Addr,
    ) -> Result<Option<String>, PlatformError> {
        self.0.reverse_dns(server, ip).await
    }
    async fn mdns_browse(
        &self,
        local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        self.0.mdns_browse(local, window).await
    }
    async fn ssdp_search(
        &self,
        local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<SsdpReply>, PlatformError> {
        self.0.ssdp_search(local, window).await
    }
    async fn netbios_status(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        self.0.netbios_status(ip, timeout).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_hung_netio_is_cut_off_by_the_layer_limit() {
    let (mgr, _bus) = manager(
        Arc::new(scenario("home")),
        Arc::new(HungHttp(scenario("home"))),
    );
    let r = run_with(&mgr).await;
    assert_eq!(r.state, RunState::Done);
    assert_eq!(r.steps.len(), 5);
    let internet = &r.steps[2];
    assert!(
        internet.evidence.iter().any(|e| e.contains("timed out")),
        "{:?}",
        internet.evidence
    );
    // The pings and TCP still finished, so the internet layer passes.
    assert_eq!(internet.status, LayerStatus::Pass);
    let website = &r.steps[4];
    assert_eq!(website.finding, Finding::WebsiteUnreachable);
    assert!(website.evidence.iter().any(|e| e.contains("timed out")));
}

/// A `SimNetwork` whose gateway ping hangs: the router layer times out with
/// every ping unfinished.
struct Stuck {
    inner: SimNetwork,
    ping_panics: bool,
}

#[async_trait]
impl Platform for Stuck {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.inner.active_interface().await
    }
    async fn ping(
        &self,
        _ip: Ipv4Addr,
        _timeout: Duration,
    ) -> Result<Option<PingReply>, PlatformError> {
        if self.ping_panics {
            panic!("injected fault: ping panicked");
        }
        std::future::pending().await
    }
    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.inner.arp_resolve(ip, timeout).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.inner.arp_table().await
    }
    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        self.inner.tcp_probe(ip, port, timeout).await
    }
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        self.inner.wifi_current().await
    }
    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        self.inner.wifi_scan().await
    }
}

#[tokio::test(start_paused = true)]
async fn a_hung_gateway_ping_fails_the_router_as_timed_out() {
    let platform = Arc::new(Stuck {
        inner: scenario("home"),
        ping_panics: false,
    });
    let (mgr, _bus) = manager(platform, Arc::new(scenario("home")));
    let r = run_with(&mgr).await;
    assert_eq!(r.state, RunState::Done);
    let router = &r.steps[1];
    assert_eq!(router.finding, Finding::RouterUnreachable);
    assert!(
        router.evidence.iter().any(|e| e.contains("timed out")),
        "{:?}",
        router.evidence
    );
    assert_eq!(r.verdict.unwrap().finding, Finding::RouterUnreachable);
}

#[tokio::test(start_paused = true)]
async fn a_panicking_platform_fails_the_run_cleanly() {
    let platform = Arc::new(Stuck {
        inner: scenario("home"),
        ping_panics: true,
    });
    let (mgr, bus) = manager(platform, Arc::new(scenario("home")));
    let mut rx = bus.subscribe();
    let id = started(mgr.start().await);

    let done = loop {
        match next(&mut rx).await.kind {
            EventKind::DiagnoseDone {
                run_id,
                state,
                verdict,
                error,
            } => break (run_id, state, verdict, error),
            EventKind::DiagnoseStep { .. } => {}
            other => panic!("unexpected event {other:?}"),
        }
    };
    let error = Some("Internal error during the diagnosis.".to_string());
    assert_eq!(done, (id, RunState::Failed, None, error.clone()));
    let r = mgr.wait(id).await.unwrap();
    assert_eq!(r.state, RunState::Failed);
    assert_eq!(r.error, error);
    assert!(r.finished_at.is_some());
    // The guard is free again.
    assert!(matches!(mgr.start().await, DiagnoseStart::Started(_)));
}

#[tokio::test(start_paused = true)]
async fn only_eight_runs_are_kept() {
    let (mgr, _bus) = sim_manager("home");
    let mut ids = Vec::new();
    for _ in 0..9 {
        ids.push(run_with(&mgr).await.id);
    }
    assert_eq!(mgr.get(ids[0]), None);
    assert!(ids[1..].iter().all(|&id| mgr.get(id).is_some()));
    assert_eq!(mgr.latest().unwrap().id, ids[8]);
    assert_eq!(ids[8], DiagnosisId(9));
}

#[tokio::test(start_paused = true)]
async fn weak_wifi_mentions_the_crowded_channel_in_evidence() {
    let r = run("weak-wifi").await;
    assert!(
        r.steps[0]
            .evidence
            .iter()
            .any(|e| e.contains("Channel 6 is shared with 9 other networks")),
        "{:?}",
        r.steps[0].evidence
    );
    assert_eq!(r.steps[0].status, LayerStatus::Warn);
    assert!(r.steps[1..].iter().all(|s| s.status == LayerStatus::Pass));
}

#[tokio::test(start_paused = true)]
async fn dns_failure_records_the_last_system_server_tried() {
    let r = run("dns-failure").await;
    let dns = &r.steps[3];
    assert!(
        dns.evidence
            .iter()
            .any(|e| e.starts_with("www.microsoft.com via 192.168.1.1: failed")),
        "{:?}",
        dns.evidence
    );
    assert!(dns
        .evidence
        .iter()
        .any(|e| e.starts_with("www.microsoft.com via 1.1.1.1: ")));
}

#[tokio::test(start_paused = true)]
async fn dns_asks_system_servers_in_order_until_one_answers() {
    let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["dns_down"] = serde_json::json!(["192.168.1.1", "192.168.1.2"]);
    let sim = SimNetwork::from_json(&json.to_string()).unwrap();
    let cfg = DiagnoseConfig::default();

    // The second server answers, so its lookup is the one recorded.
    let probe = gather_dns(&sim, &[ip("192.168.1.1"), ip("9.9.9.9")], &cfg).await;
    assert!(probe
        .system
        .iter()
        .all(|l| l.server == ip("9.9.9.9") && l.result.is_ok()));

    // Every server fails: the last one tried is recorded, with its error.
    let probe = gather_dns(&sim, &[ip("192.168.1.1"), ip("192.168.1.2")], &cfg).await;
    assert!(probe.system.iter().all(|l| l.server == ip("192.168.1.2")
        && l.result == Err("I/O error: DNS query to 192.168.1.2 timed out".into())));
    assert_eq!(probe.direct.len(), 2);
    assert!(probe.direct.iter().all(|l| l.result.is_ok()));
}
