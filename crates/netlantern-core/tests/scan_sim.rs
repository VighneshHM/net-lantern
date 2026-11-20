//! End-to-end quick-scan tests against the simulated networks in `scenarios/`,
//! persisting into an in-memory store.
//!
//! Every test but one runs with a paused clock, so the simulated ping latencies
//! and timeouts cost no real time. The exception races real threads against an
//! instant network.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use netlantern_core::{
    CoreError, Db, Device, DeviceId, DevicePatch, DeviceType, Event, EventBus, EventKind,
    NetworkId, ReplacedRouter, ScanConfig, ScanId, ScanManager, ScanMode, ScanStage, ScanState,
    StartOutcome, Store, NEW_ROUTER_TEXT,
};
use netlantern_platform::{
    ArpEntry, HttpResponse, Interface, MacAddr, MdnsHost, MdnsService, NearbyNetwork,
    NeighborState, NetIo, NetbiosInfo, PingReply, Platform, PlatformError, SimNetwork, SsdpReply,
    TcpProbe, WifiConnection,
};
use tokio::sync::broadcast;
use tokio::time::Instant;

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn scenario_path(name: &str) -> String {
    format!("{}/../../scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"))
}

fn scenario(name: &str) -> SimNetwork {
    SimNetwork::load(scenario_path(name)).unwrap()
}

/// The home scenario without the device at `gone`.
fn home_without(gone: &str) -> SimNetwork {
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["devices"]
        .as_array_mut()
        .unwrap()
        .retain(|d| d["ip"] != gone);
    SimNetwork::from_json(&json.to_string()).unwrap()
}

fn by_ip<'a>(list: &'a [Device], addr: &str) -> &'a Device {
    list.iter()
        .find(|d| d.ip == Some(ip(addr)))
        .unwrap_or_else(|| panic!("no device at {addr}"))
}

/// A manager wired to an event bus and an in-memory store, plus a
/// subscription that was taken out before any scan could publish.
struct Rig {
    mgr: ScanManager,
    bus: Arc<EventBus>,
    store: Store,
    rx: broadcast::Receiver<Event>,
}

fn store() -> Store {
    Store::new(Db::open_in_memory().unwrap())
}

/// A simulated network used as both the platform and the discovery I/O.
fn rig(sim: SimNetwork) -> Rig {
    let sim = Arc::new(sim);
    rig_on(sim.clone(), sim, store(), ScanConfig::default())
}

/// A wrapped platform; discovery runs against the plain home scenario.
fn rig_with(platform: impl Platform + 'static, config: ScanConfig) -> Rig {
    rig_on(
        Arc::new(platform),
        Arc::new(scenario("home")),
        store(),
        config,
    )
}

fn rig_on(
    platform: Arc<dyn Platform>,
    netio: Arc<dyn NetIo>,
    store: Store,
    config: ScanConfig,
) -> Rig {
    let bus = Arc::new(EventBus::new(4096));
    let rx = bus.subscribe();
    let mgr = ScanManager::new(platform, netio, Arc::clone(&bus), store.clone(), config);
    Rig {
        mgr,
        bus,
        store,
        rx,
    }
}

fn started(outcome: StartOutcome) -> ScanId {
    match outcome {
        StartOutcome::Started(id) => id,
        other => panic!("expected a new scan to start, got {other:?}"),
    }
}

fn scan_of(kind: &EventKind) -> ScanId {
    match kind {
        EventKind::ScanProgress { scan_id, .. }
        | EventKind::ScanDevice { scan_id, .. }
        | EventKind::ScanDone { scan_id, .. } => *scan_id,
        other => panic!("not a scan event: {other:?}"),
    }
}

/// Collects events up to and including the first `scan.done`.
async fn collect_until_done(rx: &mut broadcast::Receiver<Event>) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        let event = rx.recv().await.unwrap();
        let finished = matches!(event.kind, EventKind::ScanDone { .. });
        events.push(event);
        if finished {
            return events;
        }
    }
}

/// Like [`collect_until_done`], with each event's arrival time since `start`.
async fn collect_until_done_timed(
    rx: &mut broadcast::Receiver<Event>,
    start: Instant,
) -> Vec<(Event, Duration)> {
    let mut events = Vec::new();
    loop {
        let event = rx.recv().await.unwrap();
        let finished = matches!(event.kind, EventKind::ScanDone { .. });
        events.push((event, start.elapsed()));
        if finished {
            return events;
        }
    }
}

fn done_of(events: &[Event]) -> (ScanState, Option<String>, bool, u32) {
    match &events.last().unwrap().kind {
        EventKind::ScanDone {
            state,
            error,
            isolated,
            devices_found,
            ..
        } => (*state, error.clone(), *isolated, *devices_found),
        other => panic!("expected scan.done, got {other:?}"),
    }
}

fn streamed(events: &[Event]) -> Vec<Device> {
    events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ScanDevice { device, .. } => Some(device.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn home_scan_identifies_every_device_within_targets() {
    let mut rig = rig(scenario("home"));
    let start = Instant::now();
    let _id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let timed = collect_until_done_timed(&mut rig.rx, start).await;
    let events: Vec<Event> = timed.iter().map(|(e, _)| e.clone()).collect();
    let total = start.elapsed();
    assert!(total <= secs(20), "scan took {total:?}");
    let first_device_at = timed
        .iter()
        .find(|(e, _)| matches!(e.kind, EventKind::ScanDevice { .. }))
        .unwrap()
        .1;
    assert!(
        first_device_at <= secs(3),
        "first device at {first_device_at:?}"
    );
    assert!(matches!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            devices_found: 13,
            state: ScanState::Done,
            isolated: false,
            ..
        }
    ));
    let net = rig.mgr.current_network().await.unwrap().unwrap();
    let list = rig.store.list_devices(net).await.unwrap();
    assert_eq!(list.len(), 13);
    let typed = list
        .iter()
        .filter(|d| d.device_type != DeviceType::Unknown)
        .count();
    assert!(typed * 10 >= list.len() * 7, "only {typed} of 13 typed");
    assert!(list
        .iter()
        .filter(|d| !d.private_mac)
        .all(|d| d.vendor.is_some()));
    assert_eq!(
        by_ip(&list, "192.168.1.52").display_name,
        "[TV] Living Room"
    );
    assert!(!by_ip(&list, "192.168.1.88").answers_ping);
    assert!(by_ip(&list, "192.168.1.57").open_ports.contains(&62078));
}

#[tokio::test(start_paused = true)]
async fn devices_are_merged_from_ping_arp_and_discovery() {
    let mut rig = rig(scenario("home"));
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    let net = rig.mgr.current_network().await.unwrap().unwrap();
    let list = rig.store.list_devices(net).await.unwrap();

    // Answers ARP but ignores ping: present, with a MAC and no round-trip time.
    let quiet = by_ip(&list, "192.168.1.88");
    assert!(!quiet.answers_ping && quiet.mac.is_some());
    assert_eq!(quiet.rtt_ms, None);

    let this_laptop = by_ip(&list, "192.168.1.23");
    assert!(this_laptop.is_self);
    assert_eq!(
        this_laptop.mac,
        Some("F4:8E:38:9A:20:7C".parse::<MacAddr>().unwrap())
    );
    // This laptop is never port-probed.
    assert!(this_laptop.open_ports.is_empty());
    assert_eq!(list.iter().filter(|d| d.is_self).count(), 1);

    let gateway = by_ip(&list, "192.168.1.1");
    assert!(gateway.is_gateway && gateway.answers_ping);
    assert_eq!(gateway.rtt_ms, Some(2));
    // Only the configured ports are probed: 53 is open but never asked.
    assert_eq!(gateway.open_ports, vec![80, 443]);
    assert_eq!(list.iter().filter(|d| d.is_gateway).count(), 1);

    assert!(by_ip(&list, "192.168.1.44").private_mac);
    assert_eq!(list.iter().filter(|d| d.private_mac).count(), 1);
    assert!(list.iter().all(|d| d.online));

    // The last event for each device carries exactly what the store holds.
    let mut last: HashMap<DeviceId, Device> = HashMap::new();
    for device in streamed(&events) {
        last.insert(device.id, device);
    }
    assert_eq!(last.len(), 13);
    for device in &list {
        assert_eq!(&last[&device.id], device);
    }
}

#[tokio::test(start_paused = true)]
async fn events_move_through_every_stage_then_done() {
    let mut rig = rig(scenario("home"));
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;

    // The sweep announces itself at 0, then every 16 replies and at the end.
    let sweep: Vec<u32> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::ScanProgress {
                done,
                total,
                stage: ScanStage::Sweep,
                ..
            } => {
                assert_eq!(total, 254);
                Some(done)
            }
            _ => None,
        })
        .collect();
    let expected: Vec<u32> = (0..=15).map(|n| n * 16).chain([254]).collect();
    assert_eq!(sweep, expected);

    // Every later stage announces itself once, already at 100%.
    let later: Vec<(ScanStage, u32, u32)> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::ScanProgress {
                done, total, stage, ..
            } if stage != ScanStage::Sweep => Some((stage, done, total)),
            _ => None,
        })
        .collect();
    assert_eq!(
        later,
        vec![
            (ScanStage::Arp, 254, 254),
            (ScanStage::Ports, 254, 254),
            (ScanStage::Discover, 254, 254),
            (ScanStage::Merge, 254, 254)
        ]
    );

    // Devices stream during the sweep (those that replied), and again,
    // enriched, after the merge stage starts; nothing in between.
    let rank = |kind: &EventKind| match kind {
        EventKind::ScanProgress {
            stage: ScanStage::Sweep,
            ..
        }
        | EventKind::ScanDevice { .. } => 0,
        EventKind::ScanProgress {
            stage: ScanStage::Arp,
            ..
        } => 1,
        EventKind::ScanProgress {
            stage: ScanStage::Ports,
            ..
        } => 2,
        EventKind::ScanProgress {
            stage: ScanStage::Discover,
            ..
        } => 3,
        EventKind::ScanProgress {
            stage: ScanStage::Merge,
            ..
        } => 4,
        EventKind::ScanDone { .. } => 6,
        other => panic!("not a scan event: {other:?}"),
    };
    let merge_at = events
        .iter()
        .position(|e| {
            matches!(
                e.kind,
                EventKind::ScanProgress {
                    stage: ScanStage::Merge,
                    ..
                }
            )
        })
        .unwrap();
    let ranks: Vec<u8> = events
        .iter()
        .enumerate()
        .map(|(i, e)| match e.kind {
            EventKind::ScanDevice { .. } if i > merge_at => 5,
            _ => rank(&e.kind),
        })
        .collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "out of order: {ranks:?}"
    );
    // The 12 pingers stream during the sweep, except the phone at .44 whose
    // private MAC is new and waits for the merge; all 13 in the merge.
    assert_eq!(ranks.iter().filter(|&&r| r == 0).count() - sweep.len(), 11);
    assert_eq!(ranks.iter().filter(|&&r| r == 5).count(), 13);
}

#[tokio::test(start_paused = true)]
async fn quick_scan_snapshot_reports_the_finished_scan() {
    let Rig {
        mgr, mut rx, store, ..
    } = rig(scenario("home"));

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    assert_eq!(id, ScanId(1));
    let events = collect_until_done(&mut rx).await;
    assert_eq!(events[0].seq, 1);
    assert!(events.windows(2).all(|w| w[1].seq == w[0].seq + 1));
    assert_eq!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            scan_id: id,
            state: ScanState::Done,
            error: None,
            isolated: false,
            devices_found: 13
        }
    );

    let snapshot = mgr.snapshot(id).await.unwrap().unwrap();
    assert_eq!(snapshot.mode, ScanMode::Quick);
    assert_eq!(
        (
            snapshot.state,
            snapshot.stage,
            snapshot.done,
            snapshot.total,
            snapshot.devices_found,
            snapshot.error,
            snapshot.isolated
        ),
        (ScanState::Done, ScanStage::Merge, 254, 254, 13, None, false)
    );
    assert_eq!(mgr.snapshot(ScanId(2)).await.unwrap(), None);
    assert_eq!(mgr.latest().await.unwrap().unwrap().id, id);

    // The row is final in the store, so another process sees the same result.
    let stored = store.scan(id).await.unwrap().unwrap();
    assert_eq!(
        (
            stored.state,
            stored.devices_found,
            stored.error,
            stored.isolated
        ),
        (ScanState::Done, 13, None, false)
    );
}

#[tokio::test(start_paused = true)]
async fn a_new_scan_can_start_as_soon_as_scan_done_arrives() {
    let Rig {
        mgr, store, mut rx, ..
    } = rig(scenario("home"));
    let first = started(mgr.start(ScanMode::Quick).await.unwrap());
    let first_events = collect_until_done(&mut rx).await;

    // The running guard is released together with `scan.done`, so a client
    // reacting to it never gets "already running".
    let second = started(mgr.start(ScanMode::Quick).await.unwrap());
    assert_eq!((first, second), (ScanId(1), ScanId(2)));
    let events = collect_until_done(&mut rx).await;

    // Every event of the first scan precedes every event of the second.
    assert!(first_events.iter().all(|e| scan_of(&e.kind) == first));
    assert!(events.iter().all(|e| scan_of(&e.kind) == second));
    assert!(first_events.last().unwrap().seq < events[0].seq);
    assert!(matches!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            scan_id: ScanId(2),
            state: ScanState::Done,
            devices_found: 13,
            ..
        }
    ));
    assert_eq!(
        mgr.snapshot(first).await.unwrap().unwrap().state,
        ScanState::Done
    );
    assert_eq!(
        mgr.snapshot(second).await.unwrap().unwrap().state,
        ScanState::Done
    );
    // Re-scanning the same network updates devices rather than duplicating them.
    let net = mgr.current_network().await.unwrap().unwrap();
    assert_eq!(store.list_devices(net).await.unwrap().len(), 13);
}

#[tokio::test(start_paused = true)]
async fn rescan_keeps_names_and_marks_missing_devices_offline() {
    let Rig {
        mgr,
        bus,
        store,
        mut rx,
    } = rig(scenario("home"));
    started(mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rx).await;
    let net = mgr.current_network().await.unwrap().unwrap();
    let list = store.list_devices(net).await.unwrap();
    let nas = by_ip(&list, "192.168.1.70").id;
    store
        .update_device(
            net,
            nas,
            DevicePatch {
                name: Some(Some("Backup box".into())),
                ..DevicePatch::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    let printer = by_ip(&list, "192.168.1.95").id;

    // Same network, same store, a new manager (as after a restart): .95 is gone.
    let sim = Arc::new(home_without("192.168.1.95"));
    let mgr2 = ScanManager::new(
        sim.clone(),
        sim,
        Arc::clone(&bus),
        store.clone(),
        ScanConfig::default(),
    );
    // Before scanning, the network is identified passively from the ARP table.
    assert_eq!(mgr2.current_network().await.unwrap(), Some(net));
    // A scan this process did not run is read from the store.
    assert_eq!(
        mgr2.snapshot(ScanId(1)).await.unwrap().unwrap().state,
        ScanState::Done
    );
    assert_eq!(mgr2.latest().await.unwrap().unwrap().id, ScanId(1));

    let second = started(mgr2.start(ScanMode::Quick).await.unwrap());
    assert_eq!(second, ScanId(2));
    let events = collect_until_done(&mut rx).await;
    assert_eq!(done_of(&events), (ScanState::Done, None, false, 12));

    let list = store.list_devices(net).await.unwrap();
    assert_eq!(list.len(), 13);
    let nas = list.iter().find(|d| d.id == nas).unwrap();
    assert_eq!(nas.display_name, "Backup box");
    assert!(nas.online);
    let printer = list.iter().find(|d| d.id == printer).unwrap();
    assert!(!printer.online);
    assert_eq!(list.iter().filter(|d| d.online).count(), 12);
}

#[tokio::test(start_paused = true)]
async fn isolated_network_is_flagged() {
    let mut rig = rig(scenario("isolated"));
    let id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert!(matches!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            isolated: true,
            devices_found: 2,
            state: ScanState::Done,
            ..
        }
    ));
    assert!(rig.mgr.snapshot(id).await.unwrap().unwrap().isolated);
    assert!(rig.store.scan(id).await.unwrap().unwrap().isolated);
}

#[tokio::test(start_paused = true)]
async fn home_network_is_not_isolated_even_with_only_a_quiet_device() {
    // The gateway plus a ping-silent device is not isolation.
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["devices"].as_array_mut().unwrap().retain(|d| {
        ["192.168.1.1", "192.168.1.23", "192.168.1.88"].contains(&d["ip"].as_str().unwrap())
    });
    let mut rig = rig(SimNetwork::from_json(&json.to_string()).unwrap());
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events), (ScanState::Done, None, false, 3));
}

/// Discovery I/O where every method fails at once.
struct FailingNetIo;

fn boom() -> PlatformError {
    PlatformError::Io {
        message: "boom".into(),
    }
}

#[async_trait]
impl NetIo for FailingNetIo {
    async fn dns_resolve(&self, _: Ipv4Addr, _: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
        Err(boom())
    }
    async fn http_get(&self, _: &str, _: bool) -> Result<HttpResponse, PlatformError> {
        Err(boom())
    }
    async fn reverse_dns(&self, _: Ipv4Addr, _: Ipv4Addr) -> Result<Option<String>, PlatformError> {
        Err(boom())
    }
    async fn mdns_browse(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        Err(boom())
    }
    async fn ssdp_search(&self, _: Ipv4Addr, _: Duration) -> Result<Vec<SsdpReply>, PlatformError> {
        Err(boom())
    }
    async fn netbios_status(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        Err(boom())
    }
}

#[tokio::test(start_paused = true)]
async fn discovery_failure_does_not_fail_the_scan() {
    let Rig {
        mgr, store, mut rx, ..
    } = rig_on(
        Arc::new(scenario("home")),
        Arc::new(FailingNetIo),
        store(),
        ScanConfig::default(),
    );
    started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    assert_eq!(done_of(&events), (ScanState::Done, None, false, 13));
    let net = mgr.current_network().await.unwrap().unwrap();
    let list = store.list_devices(net).await.unwrap();
    assert_eq!(list.len(), 13);
    assert!(list
        .iter()
        .filter(|d| !d.private_mac)
        .all(|d| d.vendor.is_some()));
}

/// Ways a [`Faulty`] network misbehaves.
#[derive(Default)]
struct Faults {
    /// The OS refuses to hand over its ARP table.
    arp_unreadable: bool,
    /// The first ping to every address is lost; a second one gets through.
    first_ping_lost: bool,
    /// The platform code reading the ARP table panics (a bug in a driver shim).
    arp_panics: bool,
    /// The link drops after the first interface lookup.
    disconnects_after_first_lookup: bool,
    /// Pinging this address fails outright.
    ping_fails_at: Option<Ipv4Addr>,
    /// The gateway is in neither the ARP table nor answers ARP requests.
    silent_gateway: bool,
    /// The gateway is missing from the ARP table but answers ARP requests.
    gateway_unlisted: bool,
    /// Every TCP probe fails outright.
    probes_fail: bool,
    /// Every interface lookup after the first takes five seconds.
    slow_lookups_after_first: bool,
}

/// A simulated network with injected faults.
struct Faulty {
    sim: SimNetwork,
    faults: Faults,
    pinged: Mutex<HashSet<Ipv4Addr>>,
    lookups: AtomicUsize,
    resolved: Mutex<Vec<Ipv4Addr>>,
}

impl Faulty {
    fn new(sim: SimNetwork, faults: Faults) -> Self {
        Self {
            sim,
            faults,
            pinged: Mutex::default(),
            lookups: AtomicUsize::new(0),
            resolved: Mutex::default(),
        }
    }

    fn gateway(&self) -> Ipv4Addr {
        ip("192.168.1.1")
    }
}

#[async_trait]
impl Platform for Faulty {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        let earlier = self.lookups.fetch_add(1, Ordering::SeqCst);
        if self.faults.disconnects_after_first_lookup && earlier > 0 {
            return Err(PlatformError::NotConnected);
        }
        if self.faults.slow_lookups_after_first && earlier > 0 {
            tokio::time::sleep(secs(5)).await;
        }
        self.sim.active_interface().await
    }

    async fn ping(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<PingReply>, PlatformError> {
        if self.faults.ping_fails_at == Some(ip) {
            return Err(PlatformError::Io {
                message: "the ping helper crashed".into(),
            });
        }
        let first_try = self.pinged.lock().unwrap().insert(ip);
        if self.faults.first_ping_lost && first_try {
            return Ok(None);
        }
        self.sim.ping(ip, timeout).await
    }

    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.resolved.lock().unwrap().push(ip);
        if self.faults.silent_gateway && ip == self.gateway() {
            return Ok(None);
        }
        self.sim.arp_resolve(ip, timeout).await
    }

    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        if self.faults.arp_panics {
            panic!("injected fault: arp_table panicked");
        }
        if self.faults.arp_unreadable {
            return Err(PlatformError::Unavailable {
                reason: "neighbor table unreadable".into(),
            });
        }
        let mut table = self.sim.arp_table().await?;
        if self.faults.silent_gateway || self.faults.gateway_unlisted {
            table.retain(|e| e.ip != self.gateway());
        }
        Ok(table)
    }

    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        if self.faults.probes_fail {
            return Err(PlatformError::Io {
                message: "the probe helper crashed".into(),
            });
        }
        self.sim.tcp_probe(ip, port, timeout).await
    }
}

fn faulty(faults: Faults) -> Rig {
    rig_with(Faulty::new(scenario("home"), faults), ScanConfig::default())
}

#[tokio::test(start_paused = true)]
async fn a_scan_failing_because_the_link_dropped_is_cancelled_with_the_reason() {
    let Rig {
        mgr, store, mut rx, ..
    } = faulty(Faults {
        ping_fails_at: Some(ip("192.168.1.40")),
        disconnects_after_first_lookup: true,
        ..Faults::default()
    });
    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    let (state, error, isolated, _) = done_of(&events);
    assert_eq!(
        (state, error.as_deref(), isolated),
        (
            ScanState::Cancelled,
            Some("The connection dropped during the scan."),
            false
        )
    );
    let stored = store.scan(id).await.unwrap().unwrap();
    assert_eq!(stored.state, ScanState::Cancelled);
}

#[tokio::test(start_paused = true)]
async fn a_probe_failure_on_an_unchanged_network_still_fails() {
    let Rig { mgr, mut rx, .. } = faulty(Faults {
        probes_fail: true,
        ..Faults::default()
    });
    started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    let (state, error, isolated, _) = done_of(&events);
    assert_eq!((state, isolated), (ScanState::Failed, false));
    let msg = error.expect("a failed scan says why");
    assert!(msg.contains("the probe helper crashed"), "{msg}");
}

#[tokio::test(start_paused = true)]
async fn a_stop_while_a_failing_scan_checks_the_network_ends_cancelled() {
    let Rig { mgr, mut rx, .. } = faulty(Faults {
        ping_fails_at: Some(ip("192.168.1.40")),
        slow_lookups_after_first: true,
        ..Faults::default()
    });
    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    // The ping to .40 fails at the start of the sweep (virtual time zero);
    // the scan then spends seconds reading the interface before it reports.
    tokio::time::sleep(secs(1)).await;
    assert!(mgr.cancel(id));
    let events = collect_until_done(&mut rx).await;
    let (state, error, isolated, _) = done_of(&events);
    assert_eq!(
        (state, error, isolated),
        (ScanState::Cancelled, None, false)
    );
}

#[tokio::test(start_paused = true)]
async fn platform_error_mid_sweep_fails_with_a_reason() {
    let Rig {
        mgr, store, mut rx, ..
    } = faulty(Faults {
        ping_fails_at: Some(ip("192.168.1.40")),
        ..Faults::default()
    });

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    let (state, error, isolated, _) = done_of(&events);
    assert_eq!((state, isolated), (ScanState::Failed, false));
    let msg = error.expect("a failed scan says why");
    assert!(msg.contains("the ping helper crashed"), "{msg}");

    let snapshot = mgr.snapshot(id).await.unwrap().unwrap();
    assert_eq!(snapshot.state, ScanState::Failed);
    assert_eq!(snapshot.error.as_deref(), Some(msg.as_str()));
    assert_eq!(
        store.scan(id).await.unwrap().unwrap().error.as_deref(),
        Some(msg.as_str())
    );

    // A failed scan does not wedge the manager.
    assert_eq!(
        started(mgr.start(ScanMode::Quick).await.unwrap()),
        ScanId(2)
    );
    collect_until_done(&mut rx).await;
}

#[tokio::test(start_paused = true)]
async fn silent_gateway_fails_with_the_router_message() {
    let Rig {
        mgr, store, mut rx, ..
    } = faulty(Faults {
        silent_gateway: true,
        ..Faults::default()
    });
    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    assert_eq!(
        done_of(&events),
        (
            ScanState::Failed,
            Some(
                "Your router at 192.168.1.1 didn't answer, so this network can't be identified."
                    .into()
            ),
            false,
            0
        )
    );
    assert!(streamed(&events).is_empty());
    assert_eq!(
        store.scan(id).await.unwrap().unwrap().state,
        ScanState::Failed
    );
    // Nothing was identified, so there is no current network.
    assert_eq!(mgr.current_network().await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn a_gateway_missing_from_the_table_is_resolved_once() {
    let platform = Arc::new(Faulty::new(
        scenario("home"),
        Faults {
            gateway_unlisted: true,
            ..Faults::default()
        },
    ));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events), (ScanState::Done, None, false, 13));
    let gateway_lookups = platform
        .resolved
        .lock()
        .unwrap()
        .iter()
        .filter(|a| **a == ip("192.168.1.1"))
        .count();
    assert_eq!(gateway_lookups, 1);
    let net = rig.mgr.current_network().await.unwrap();
    // Not in the table, so only the last scan's memory identifies the network.
    assert!(net.is_some());
    let list = rig.store.list_devices(net.unwrap()).await.unwrap();
    assert!(by_ip(&list, "192.168.1.1").mac.is_some());
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_arp_table_fails_the_scan_and_frees_the_guard() {
    let Rig { mgr, mut rx, .. } = faulty(Faults {
        arp_unreadable: true,
        ..Faults::default()
    });

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    let (state, error, _, found) = done_of(&events);
    assert_eq!((state, found), (ScanState::Failed, 0));
    assert!(error.unwrap().contains("neighbor table unreadable"));
    assert!(streamed(&events).is_empty());
    assert_eq!(
        mgr.snapshot(id).await.unwrap().unwrap().state,
        ScanState::Failed
    );
    assert_eq!(
        started(mgr.start(ScanMode::Quick).await.unwrap()),
        ScanId(2)
    );
}

#[tokio::test(start_paused = true)]
async fn a_panicking_platform_fails_the_scan_and_frees_the_guard() {
    let Rig { mgr, mut rx, .. } = faulty(Faults {
        arp_panics: true,
        ..Faults::default()
    });

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    assert_eq!(
        done_of(&events),
        (
            ScanState::Failed,
            Some("Internal error during the scan.".into()),
            false,
            0
        )
    );
    assert_eq!(
        mgr.snapshot(id).await.unwrap().unwrap().state,
        ScanState::Failed
    );
    assert_eq!(
        started(mgr.start(ScanMode::Quick).await.unwrap()),
        ScanId(2)
    );
}

/// A simulated network, plus `Unreachable` ARP entries for every other swept
/// address except `without_entry`; counts `arp_resolve` calls per address,
/// and TCP connects in flight per address.
struct CountingPlatform {
    sim: SimNetwork,
    without_entry: Vec<Ipv4Addr>,
    resolved: Mutex<Vec<Ipv4Addr>>,
    /// How long each TCP probe takes.
    probe_delay: Duration,
    /// Per address: TCP connects in flight now, and the most ever in flight.
    connecting: Mutex<HashMap<Ipv4Addr, (usize, usize)>>,
}

impl CountingPlatform {
    fn new(sim: SimNetwork, without_entry: Vec<Ipv4Addr>) -> Self {
        Self {
            sim,
            without_entry,
            resolved: Mutex::default(),
            probe_delay: Duration::ZERO,
            connecting: Mutex::default(),
        }
    }

    /// The most TCP connects ever in flight at once to `addr`.
    fn peak_connects(&self, addr: Ipv4Addr) -> usize {
        self.connecting
            .lock()
            .unwrap()
            .get(&addr)
            .map_or(0, |(_, peak)| *peak)
    }
}

#[async_trait]
impl Platform for CountingPlatform {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.sim.active_interface().await
    }
    async fn ping(&self, ip: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.sim.ping(ip, t).await
    }
    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.resolved.lock().unwrap().push(ip);
        self.sim.arp_resolve(ip, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        let mut table = self.sim.arp_table().await?;
        let known: HashSet<Ipv4Addr> = table.iter().map(|e| e.ip).collect();
        for host in 1..=254u8 {
            let addr = Ipv4Addr::new(192, 168, 1, host);
            if !known.contains(&addr) && !self.without_entry.contains(&addr) {
                table.push(ArpEntry {
                    ip: addr,
                    mac: MacAddr([0; 6]),
                    state: NeighborState::Unreachable,
                });
            }
        }
        // Broadcast and multicast entries, as Windows lists them.
        for addr in ["192.168.1.255", "224.0.0.22"] {
            table.push(ArpEntry {
                ip: ip(addr),
                mac: MacAddr([0xFF; 6]),
                state: NeighborState::Permanent,
            });
        }
        Ok(table)
    }
    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        p: u16,
        t: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        if self.probe_delay.is_zero() {
            return self.sim.tcp_probe(ip, p, t).await;
        }
        {
            let mut connecting = self.connecting.lock().unwrap();
            let (now, peak) = connecting.entry(ip).or_default();
            *now += 1;
            *peak = (*peak).max(*now);
        }
        tokio::time::sleep(self.probe_delay).await;
        let result = self.sim.tcp_probe(ip, p, t).await;
        self.connecting.lock().unwrap().get_mut(&ip).unwrap().0 -= 1;
        result
    }
}

#[tokio::test(start_paused = true)]
async fn arp_resolve_only_for_addresses_without_entries() {
    let without_entry = vec![
        ip("192.168.1.200"),
        ip("192.168.1.201"),
        ip("192.168.1.202"),
    ];
    let platform = Arc::new(CountingPlatform::new(
        scenario("home"),
        without_entry.clone(),
    ));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    // Broadcast/multicast and unreachable entries add no device.
    assert_eq!(done_of(&events), (ScanState::Done, None, false, 13));
    let mut resolved = platform.resolved.lock().unwrap().clone();
    resolved.sort();
    // The gateway came from the table, so it was never resolved.
    assert_eq!(resolved, without_entry);
    // No device took the all-zero MAC of an Unreachable entry.
    let net = rig.mgr.current_network().await.unwrap().unwrap();
    let list = rig.store.list_devices(net).await.unwrap();
    assert!(list.iter().all(|d| d.mac != Some(MacAddr([0; 6]))));
}

#[tokio::test(start_paused = true)]
async fn a_device_found_only_by_arp_resolve_is_present() {
    // The neighbor table misses a device that answers an ARP request.
    struct Forgetful(SimNetwork);
    #[async_trait]
    impl Platform for Forgetful {
        async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
            Err(PlatformError::Unavailable {
                reason: "no Wi-Fi".into(),
            })
        }

        async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
            Err(PlatformError::Unavailable {
                reason: "no Wi-Fi".into(),
            })
        }

        async fn active_interface(&self) -> Result<Interface, PlatformError> {
            self.0.active_interface().await
        }
        async fn ping(&self, a: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
            self.0.ping(a, t).await
        }
        async fn arp_resolve(
            &self,
            a: Ipv4Addr,
            t: Duration,
        ) -> Result<Option<MacAddr>, PlatformError> {
            self.0.arp_resolve(a, t).await
        }
        async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
            let mut table = self.0.arp_table().await?;
            table.retain(|e| e.ip != ip("192.168.1.88"));
            Ok(table)
        }
        async fn tcp_probe(
            &self,
            a: Ipv4Addr,
            p: u16,
            t: Duration,
        ) -> Result<TcpProbe, PlatformError> {
            self.0.tcp_probe(a, p, t).await
        }
    }
    let Rig {
        mgr, store, mut rx, ..
    } = rig_with(Forgetful(scenario("home")), ScanConfig::default());
    started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    assert_eq!(done_of(&events).3, 13);
    let net = mgr.current_network().await.unwrap().unwrap();
    let list = store.list_devices(net).await.unwrap();
    let quiet = by_ip(&list, "192.168.1.88");
    assert_eq!(
        quiet.mac,
        Some("24:6F:28:3A:9F:21".parse::<MacAddr>().unwrap())
    );
}

#[tokio::test(start_paused = true)]
async fn already_running_and_offline_and_deep_behave_as_in_phase_0() {
    // Already running: a second start joins the first.
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));
    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    assert_eq!(
        mgr.start(ScanMode::Quick).await.unwrap(),
        StartOutcome::AlreadyRunning(id)
    );
    collect_until_done(&mut rx).await;

    // Offline: fails synchronously, claims nothing, publishes nothing.
    let Rig { mgr, bus, .. } = rig(scenario("offline"));
    assert_eq!(
        mgr.start(ScanMode::Quick).await,
        Err(CoreError::NotConnected)
    );
    assert_eq!(
        mgr.start(ScanMode::Quick).await,
        Err(CoreError::NotConnected)
    );
    assert_eq!(bus.last_seq(), 0);
    assert_eq!(mgr.snapshot(ScanId(1)).await.unwrap(), None);
    assert_eq!(mgr.latest().await.unwrap(), None);
    assert_eq!(mgr.current_network().await.unwrap(), None);

    // Deep without consent: refused, and does not use up scan id 1.
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));
    let err = mgr.start(ScanMode::Deep).await.unwrap_err();
    assert_eq!(err, CoreError::ConsentNeeded);
    assert_eq!(mgr.snapshot(ScanId(1)).await.unwrap(), None);
    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    assert_eq!(id, ScanId(1));
    collect_until_done(&mut rx).await;
}

#[tokio::test(start_paused = true)]
async fn manager_can_be_shared_with_spawned_tasks() {
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));
    let mgr = Arc::new(mgr);

    // The shape of an HTTP handler: a `Send` future holding an `Arc<ScanManager>`.
    let handler = Arc::clone(&mgr);
    let outcome = tokio::spawn(async move { handler.start(ScanMode::Quick).await })
        .await
        .unwrap()
        .unwrap();
    let id = started(outcome);

    collect_until_done(&mut rx).await;
    assert_eq!(
        mgr.snapshot(id).await.unwrap().unwrap().state,
        ScanState::Done
    );
}

/// How many of the home devices answered ping in a scan with the given faults and config.
async fn pingers_found(faults: Faults, config: ScanConfig) -> (usize, usize) {
    let Rig {
        mgr, store, mut rx, ..
    } = rig_with(Faulty::new(scenario("home"), faults), config);
    started(mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rx).await;
    let net = mgr.current_network().await.unwrap().unwrap();
    let list = store.list_devices(net).await.unwrap();
    (list.iter().filter(|d| d.answers_ping).count(), list.len())
}

#[tokio::test(start_paused = true)]
async fn an_unanswered_ping_is_retried_once_by_default() {
    let faults = Faults {
        first_ping_lost: true,
        ..Faults::default()
    };
    // 12 of the 13 devices answer ping; the 13th is ARP-only.
    assert_eq!(pingers_found(faults, ScanConfig::default()).await, (12, 13));
}

#[tokio::test(start_paused = true)]
async fn without_retries_a_lost_ping_leaves_the_device_ping_silent() {
    let faults = Faults {
        first_ping_lost: true,
        ..Faults::default()
    };
    let config = ScanConfig {
        icmp_retries: 0,
        ..ScanConfig::default()
    };
    // Still present, through ARP, but no longer counted as answering ping.
    assert_eq!(pingers_found(faults, config).await, (0, 13));
}

#[tokio::test(start_paused = true)]
async fn zero_concurrency_is_treated_as_one() {
    let config = ScanConfig {
        icmp_concurrency: 0,
        tcp_concurrency: 0,
        arp_resolve_concurrency: 0,
        ..ScanConfig::default()
    };
    assert_eq!(pingers_found(Faults::default(), config).await, (12, 13));
}

#[tokio::test(start_paused = true)]
async fn a_second_start_during_a_link_drop_joins_the_running_scan() {
    let Rig { mgr, mut rx, .. } = faulty(Faults {
        disconnects_after_first_lookup: true,
        ..Faults::default()
    });

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    // The link is gone now, but a scan is running: the caller is told to
    // follow it (409), not that the laptop is offline (503).
    assert_eq!(
        mgr.start(ScanMode::Quick).await,
        Ok(StartOutcome::AlreadyRunning(id))
    );
    collect_until_done(&mut rx).await;
    // With nothing running, the dropped link is reported as usual.
    assert_eq!(
        mgr.start(ScanMode::Quick).await,
        Err(CoreError::NotConnected)
    );
}

#[tokio::test(start_paused = true)]
async fn the_sweep_pings_concurrently() {
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));

    let begun = Instant::now();
    started(mgr.start(ScanMode::Quick).await.unwrap());
    // Wait for the end of the sweep.
    loop {
        let event = rx.recv().await.unwrap();
        if let EventKind::ScanProgress {
            stage: ScanStage::Arp,
            ..
        } = event.kind
        {
            break;
        }
    }
    let took = begun.elapsed();

    // ~240 silent addresses x 2 attempts x 800 ms is over 6 minutes one at a
    // time; 64 in flight brings it to a few seconds.
    assert!(took < secs(10), "the sweep took {took:?}");
}

/// A one-address network where only the gateway answers ARP and every call
/// returns at once, so a scan finishes in microseconds.
struct Idle;

#[async_trait]
impl Platform for Idle {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        Ok(Interface {
            ip: ip("10.0.0.5"),
            prefix: 32,
            gateway: ip("10.0.0.1"),
            dns_servers: vec![],
            kind: netlantern_platform::InterfaceKind::Wifi,
            mac: MacAddr([0xF4, 0x8E, 0x38, 0x9A, 0x20, 0x7C]),
        })
    }

    async fn ping(&self, _: Ipv4Addr, _: Duration) -> Result<Option<PingReply>, PlatformError> {
        Ok(None)
    }

    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        Ok((ip == Ipv4Addr::new(10, 0, 0, 1)).then_some(MacAddr([0x50, 0xC7, 0xBF, 0, 0, 1])))
    }

    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        Ok(vec![])
    }

    async fn tcp_probe(&self, _: Ipv4Addr, _: u16, _: Duration) -> Result<TcpProbe, PlatformError> {
        Ok(TcpProbe::Filtered)
    }
}

/// Real threads racing to start scans while scans finish: the stream must
/// never interleave two scans, and a scan must never start before the previous
/// one's `scan.done` is on the bus.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_starts_never_interleave_scans_on_the_bus() {
    let bus = Arc::new(EventBus::new(1 << 14));
    let mut rx = bus.subscribe();
    let mgr = Arc::new(ScanManager::new(
        Arc::new(Idle),
        Arc::new(FailingNetIo),
        bus,
        store(),
        ScanConfig::default(),
    ));
    let racing = Arc::new(std::sync::atomic::AtomicBool::new(true));

    // Checks each event against the one before it as they arrive, and stops
    // once the racers are finished and the bus has gone quiet.
    let checker = {
        let racing = Arc::clone(&racing);
        tokio::spawn(async move {
            let mut prev: Option<Event> = None;
            let mut scans = 0u64;
            loop {
                let event = match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await
                {
                    Ok(event) => event.expect("checker lagged behind the bus"),
                    Err(_) if !racing.load(Ordering::SeqCst) => break,
                    Err(_) => continue,
                };
                if let Some(prev) = &prev {
                    let (a, b) = (scan_of(&prev.kind), scan_of(&event.kind));
                    assert!(
                        a == b
                            || (b.0 == a.0 + 1 && matches!(prev.kind, EventKind::ScanDone { .. })),
                        "scan {b:?} published seq {} before scan {a:?} was done: {:?}",
                        event.seq,
                        prev.kind
                    );
                }
                if let EventKind::ScanDone { state, .. } = event.kind {
                    assert_eq!(state, ScanState::Done);
                    scans += 1;
                }
                prev = Some(event);
            }
            let last = prev.expect("no scan ran");
            assert!(matches!(last.kind, EventKind::ScanDone { .. }));
            scans
        })
    };

    // Race until enough scans have started, however fast this machine is: a
    // fixed number of attempts finishes too few scans on a slow CI runner.
    const SCANS: u64 = 150;
    let started = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let racers: Vec<_> = (0..4)
        .map(|_| {
            let mgr = Arc::clone(&mgr);
            let started = Arc::clone(&started);
            tokio::spawn(async move {
                while started.load(Ordering::SeqCst) < SCANS
                    && tokio::time::Instant::now() < deadline
                {
                    if let StartOutcome::Started(_) = mgr.start(ScanMode::Quick).await.unwrap() {
                        started.fetch_add(1, Ordering::SeqCst);
                    }
                    tokio::task::yield_now().await;
                }
            })
        })
        .collect();
    for racer in racers {
        racer.await.unwrap();
    }
    racing.store(false, Ordering::SeqCst);
    let scans = checker.await.unwrap();
    let started = started.load(Ordering::SeqCst);
    assert!(started >= SCANS, "only {started} scans started in 60 s");
    assert_eq!(scans, started, "every started scan must end with scan.done");
}

/// Real threads racing a cancel against the end of an instant scan: a cancel
/// that returned true always ends the scan cancelled, one that returned false
/// leaves it done, and every scan publishes exactly one `scan.done`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancel_racing_the_end_of_a_scan_agrees_with_its_outcome() {
    let bus = Arc::new(EventBus::new(1 << 14));
    let mut rx = bus.subscribe();
    let mgr = Arc::new(ScanManager::new(
        Arc::new(Idle),
        Arc::new(FailingNetIo),
        bus,
        store(),
        ScanConfig::default(),
    ));
    for round in 0..200u32 {
        let id = started(mgr.start(ScanMode::Quick).await.unwrap());
        let canceller = {
            let mgr = Arc::clone(&mgr);
            tokio::spawn(async move {
                // 0 to 2 ms, spun on this worker: the scan takes about that
                // long, so some cancels land before its end and some after.
                let delay = Duration::from_micros(u64::from(round % 50) * 40);
                let until = std::time::Instant::now() + delay;
                while std::time::Instant::now() < until {
                    std::hint::spin_loop();
                }
                mgr.cancel(id)
            })
        };
        let events = collect_until_done(&mut rx).await;
        let told = canceller.await.unwrap();
        assert!(
            events.iter().all(|e| scan_of(&e.kind) == id),
            "another scan's event arrived during scan {id:?}"
        );
        let (state, error, ..) = done_of(&events);
        if told {
            assert_eq!((state, error), (ScanState::Cancelled, None), "{id:?}");
        } else {
            assert_eq!(state, ScanState::Done, "{id:?}");
        }
        assert!(!mgr.cancel(id));
    }
    assert!(matches!(
        rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

/// A platform whose network can be swapped underneath it, as when the laptop
/// moves between networks, and whose interface can vanish (the link drops)
/// while probes already under way keep answering.
struct Switchable {
    sim: Mutex<Arc<SimNetwork>>,
    link_down: std::sync::atomic::AtomicBool,
}

impl Switchable {
    fn new(sim: SimNetwork) -> Self {
        Self {
            sim: Mutex::new(Arc::new(sim)),
            link_down: Default::default(),
        }
    }
    fn switch(&self, sim: SimNetwork) {
        *self.sim.lock().unwrap() = Arc::new(sim);
    }
    fn drop_link(&self) {
        self.link_down.store(true, Ordering::SeqCst);
    }
    fn sim(&self) -> Arc<SimNetwork> {
        self.sim.lock().unwrap().clone()
    }
}

#[async_trait]
impl Platform for Switchable {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }

    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        if self.link_down.load(Ordering::SeqCst) {
            return Err(PlatformError::NotConnected);
        }
        self.sim().active_interface().await
    }
    async fn ping(&self, a: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.sim().ping(a, t).await
    }
    async fn arp_resolve(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.sim().arp_resolve(a, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.sim().arp_table().await
    }
    async fn tcp_probe(&self, a: Ipv4Addr, p: u16, t: Duration) -> Result<TcpProbe, PlatformError> {
        self.sim().tcp_probe(a, p, t).await
    }
}

/// The home scenario with another router: same addressing, another MAC.
fn home_with_another_router() -> SimNetwork {
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    for d in json["devices"].as_array_mut().unwrap() {
        if d["ip"] == "192.168.1.1" {
            d["mac"] = "50:C7:BF:00:00:99".into();
        }
    }
    SimNetwork::from_json(&json.to_string()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn same_addressing_behind_another_router_is_not_the_last_network() {
    let platform = Arc::new(Switchable::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rig.rx).await;
    let a = rig.mgr.current_network().await.unwrap().unwrap();

    // Same ip, prefix and gateway, but the gateway has another MAC: an
    // unknown network, so none of A's devices are shown.
    platform.switch(home_with_another_router());
    assert_eq!(rig.mgr.current_network().await.unwrap(), None);

    // Once scanned, it is its own network.
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rig.rx).await;
    let b = rig.mgr.current_network().await.unwrap().unwrap();
    assert_ne!(a, b);

    // Back home with the same addressing: A again, found passively.
    platform.switch(scenario("home"));
    assert_eq!(rig.mgr.current_network().await.unwrap(), Some(a));
}

/// A manager on the home network with a database file that the test can
/// tamper with through its own connection.
struct FileRig {
    rig: Rig,
    path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn file_rig() -> FileRig {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("netlantern.db");
    let sim = Arc::new(scenario("home"));
    let rig = rig_on(
        sim.clone(),
        sim,
        Store::new(Db::open(&path).unwrap()),
        ScanConfig::default(),
    );
    FileRig {
        rig,
        path,
        _dir: dir,
    }
}

fn tamper(path: &std::path::Path, sql: &str) {
    rusqlite::Connection::open(path)
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}

#[tokio::test]
async fn store_errors_surface_instead_of_looking_empty() {
    let FileRig { rig, path, _dir } = file_rig();
    tamper(&path, "DROP TABLE scans; DROP TABLE networks;");
    let mgr = &rig.mgr;
    assert!(matches!(
        mgr.snapshot(ScanId(1)).await,
        Err(CoreError::Store(_))
    ));
    assert!(matches!(mgr.latest().await, Err(CoreError::Store(_))));
    assert!(matches!(
        mgr.current_network().await,
        Err(CoreError::Store(_))
    ));
    assert!(matches!(
        mgr.start(ScanMode::Quick).await,
        Err(CoreError::Store(_))
    ));
}

#[tokio::test(start_paused = true)]
async fn a_store_failure_during_a_scan_is_reported_in_plain_words() {
    let FileRig {
        mut rig,
        path,
        _dir,
    } = file_rig();
    tamper(
        &path,
        "CREATE TRIGGER no_devices BEFORE INSERT ON devices
         BEGIN SELECT RAISE(ABORT, 'disk I/O error 0x1234'); END;",
    );
    let id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    let message = Some("Couldn't save the scan results.".to_string());
    assert_eq!(
        done_of(&events),
        (ScanState::Failed, message.clone(), false, 0)
    );
    assert_eq!(rig.store.scan(id).await.unwrap().unwrap().error, message);
}

#[tokio::test(start_paused = true)]
async fn a_scan_whose_end_cannot_be_saved_is_not_reported_done() {
    let FileRig {
        mut rig,
        path,
        _dir,
    } = file_rig();
    tamper(
        &path,
        "CREATE TRIGGER no_finish BEFORE UPDATE OF state ON scans
         BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    );
    let id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    let message = Some("Couldn't save the scan results.".to_string());
    let (state, error, isolated, _) = done_of(&events);
    assert_eq!(
        (state, error, isolated),
        (ScanState::Failed, message.clone(), false)
    );
    let snapshot = rig.mgr.snapshot(id).await.unwrap().unwrap();
    assert_eq!(
        (snapshot.state, snapshot.error),
        (ScanState::Failed, message)
    );
}

/// The home scenario with its device list edited.
fn home_edited(edit: impl FnOnce(&mut Vec<serde_json::Value>)) -> SimNetwork {
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    edit(json["devices"].as_array_mut().unwrap());
    SimNetwork::from_json(&json.to_string()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_phone_with_a_new_private_mac_keeps_its_identity() {
    let Rig {
        mgr,
        bus,
        store,
        mut rx,
    } = rig(scenario("home"));
    started(mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rx).await;
    let net = mgr.current_network().await.unwrap().unwrap();
    let list = store.list_devices(net).await.unwrap();
    let phone = by_ip(&list, "192.168.1.44").clone();
    assert!(phone.private_mac);
    store
        .update_device(
            net,
            phone.id,
            DevicePatch {
                name: Some(Some("Asha's phone".into())),
                starred: Some(true),
                is_new: Some(false),
            },
        )
        .await
        .unwrap()
        .unwrap();

    // The phone rotated its private MAC; it still announces Pixel-7.lan.
    let sim = Arc::new(home_edited(|devices| {
        for d in devices {
            if d["ip"] == "192.168.1.44" {
                d["mac"] = "DE:5C:77:10:2A:44".into();
            }
        }
    }));
    let mgr2 = ScanManager::new(
        sim.clone(),
        sim,
        Arc::clone(&bus),
        store.clone(),
        ScanConfig::default(),
    );
    started(mgr2.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);
    // Nothing was streamed as a new device for the new MAC.
    assert!(streamed(&events)
        .iter()
        .filter(|d| d.ip == Some(ip("192.168.1.44")))
        .all(|d| d.id == phone.id));

    let list = store.list_devices(net).await.unwrap();
    assert_eq!(list.len(), 13);
    let now = by_ip(&list, "192.168.1.44");
    assert_eq!(now.id, phone.id);
    assert_eq!(now.display_name, "Asha's phone");
    assert!(now.starred);
    assert!(!now.is_new);
    assert!(now.online);
    assert_eq!(
        now.mac,
        Some("DE:5C:77:10:2A:44".parse::<MacAddr>().unwrap())
    );
}

#[tokio::test(start_paused = true)]
async fn two_phones_announcing_the_same_name_stay_two_devices() {
    let sim = home_edited(|devices| {
        devices.push(serde_json::json!({
            "ip": "192.168.1.140", "mac": "AA:10:20:30:40:01", "ping_ms": 14, "ttl": 64,
            "mdns": [{ "service": "_apple-mobdev2._tcp", "instance": "iPhone", "hostname": "iPhone.local" }]
        }));
        devices.push(serde_json::json!({
            "ip": "192.168.1.141", "mac": "AE:10:20:30:40:02", "ping_ms": 16, "ttl": 64,
            "mdns": [{ "service": "_apple-mobdev2._tcp", "instance": "iPhone", "hostname": "iPhone.local" }]
        }));
    });
    let mut rig = rig(sim);
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events), (ScanState::Done, None, false, 15));

    let net = rig.mgr.current_network().await.unwrap().unwrap();
    let list = rig.store.list_devices(net).await.unwrap();
    assert_eq!(list.len(), 15);
    let a = by_ip(&list, "192.168.1.140");
    let b = by_ip(&list, "192.168.1.141");
    assert_ne!(a.id, b.id);
    assert_ne!(a.id, by_ip(&list, "192.168.1.57").id);
    assert!(a.private_mac && b.private_mac);
}

#[tokio::test(start_paused = true)]
async fn latest_is_the_scan_of_the_current_network() {
    let platform = Arc::new(Switchable::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    let a = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    // While it runs, the scan is the latest even before its network is known.
    assert_eq!(rig.mgr.latest().await.unwrap().unwrap().id, a);
    collect_until_done(&mut rig.rx).await;
    assert_eq!(rig.mgr.latest().await.unwrap().unwrap().id, a);

    // Another network: A's scan is not its latest.
    platform.switch(home_with_another_router());
    assert_eq!(rig.mgr.latest().await.unwrap(), None);
    let b = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rig.rx).await;
    assert_eq!(rig.mgr.latest().await.unwrap().unwrap().id, b);

    // Back home: A's scan again, with its in-memory progress figures.
    platform.switch(scenario("home"));
    let latest = rig.mgr.latest().await.unwrap().unwrap();
    assert_eq!(latest.id, a);
    assert_eq!(latest.state, ScanState::Done);
    assert_eq!(latest.done, latest.total);
    assert!(latest.total > 0);

    // Offline: nothing.
    platform.switch(scenario("offline"));
    assert_eq!(rig.mgr.latest().await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn only_the_most_recent_snapshots_are_kept_in_memory() {
    let mut rig = rig(scenario("isolated"));
    let mut ids = Vec::new();
    for _ in 0..17 {
        ids.push(started(rig.mgr.start(ScanMode::Quick).await.unwrap()));
        collect_until_done(&mut rig.rx).await;
    }
    // The oldest scan now comes from the store, which keeps no progress
    // figures; the 16 most recent are still in memory.
    let oldest = rig.mgr.snapshot(ids[0]).await.unwrap().unwrap();
    assert_eq!(oldest.state, ScanState::Done);
    assert_eq!(oldest.total, 0);
    for id in &ids[1..] {
        let kept = rig.mgr.snapshot(*id).await.unwrap().unwrap();
        assert_eq!(kept.state, ScanState::Done);
        assert!(kept.total > 0, "scan {id:?} was evicted");
    }
}

#[tokio::test(start_paused = true)]
async fn private_mac_phones_get_named_and_typed_by_unicast_mdns() {
    let mut rig = rig(scenario("private-phones"));
    let _id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    let list = streamed(&events);
    let named = by_ip(&list, "192.168.1.23");
    assert_eq!(named.display_name, "Johns-iPhone");
    assert_eq!(named.device_type, DeviceType::Phone);
    let silent = by_ip(&list, "192.168.1.24");
    assert_eq!(silent.device_type, DeviceType::Unknown);
}

// ------------------------------------------------------------------ deep scan

/// Grants consent, runs a deep scan to the end, and returns its events and
/// the devices stored on its network.
async fn deep_scan_of(rig: &mut Rig) -> (Vec<Event>, Vec<Device>) {
    rig.mgr.set_scan_consent(true).await.unwrap();
    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    let net = rig.mgr.current_network().await.unwrap().unwrap();
    let list = rig.store.list_devices(net).await.unwrap();
    (events, list)
}

fn port_summary(d: &Device) -> Vec<(u16, Option<&str>, Option<&str>)> {
    d.ports
        .iter()
        .map(|p| (p.port, p.service.as_deref(), p.banner.as_deref()))
        .collect()
}

fn banner_of(d: &Device, port: u16) -> Option<&str> {
    d.ports
        .iter()
        .find(|p| p.port == port)
        .and_then(|p| p.banner.as_deref())
}

#[tokio::test(start_paused = true)]
async fn deep_scan_needs_consent_and_creates_no_scan() {
    let mut rig = rig(scenario("home"));
    let quick = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rig.rx).await;

    assert_eq!(
        rig.mgr.start(ScanMode::Deep).await,
        Err(CoreError::ConsentNeeded)
    );
    let latest = rig.mgr.latest().await.unwrap().unwrap();
    assert_eq!((latest.id, latest.mode), (quick, ScanMode::Quick));
    assert_eq!(rig.store.scan(ScanId(2)).await.unwrap(), None);

    // Consent that was taken back refuses too.
    rig.mgr.set_scan_consent(false).await.unwrap();
    assert_eq!(
        rig.mgr.start(ScanMode::Deep).await,
        Err(CoreError::ConsentNeeded)
    );
    assert_eq!(rig.store.scan(ScanId(2)).await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn consent_on_a_new_network_identifies_it() {
    let mut rig = rig(scenario("home"));
    assert_eq!(rig.mgr.scan_consent().await.unwrap(), None);

    let id = rig.mgr.set_scan_consent(true).await.unwrap();
    assert_eq!(rig.mgr.scan_consent().await.unwrap(), Some((id, true)));
    assert_eq!(rig.mgr.current_network().await.unwrap(), Some(id));

    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);
}

#[tokio::test(start_paused = true)]
async fn consent_needs_the_router() {
    let rig = rig(scenario("router-down"));
    match rig.mgr.set_scan_consent(true).await {
        Err(CoreError::Unidentified(msg)) => {
            assert!(msg.starts_with("Your router at 192.168.1.1"), "{msg}");
        }
        other => panic!("expected Unidentified, got {other:?}"),
    }
    assert_eq!(rig.mgr.scan_consent().await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn deep_scan_finds_ports_banners_and_titles() {
    let mut rig = rig(scenario("home"));
    let (events, list) = deep_scan_of(&mut rig).await;
    assert_eq!(done_of(&events).0, ScanState::Done);

    let router = by_ip(&list, "192.168.1.1");
    assert_eq!(
        port_summary(router),
        [
            (53, Some("dns"), None),
            (80, Some("http"), Some("Archer AX55")),
            (443, Some("https"), Some("Server: lighttpd")),
        ]
    );

    let nas = by_ip(&list, "192.168.1.70");
    assert_eq!(banner_of(nas, 22), Some("SSH-2.0-OpenSSH_9.6"));
    assert_eq!(banner_of(nas, 5000), Some("DiskStation"));
    // Port 5000 is outside the quick list, and this store saw no quick scan:
    // it is stored only if the sighting's open ports carry it, not just the
    // deep findings.
    assert_eq!(nas.open_ports, vec![22, 445, 5000]);
    assert_eq!(
        nas.ports.iter().map(|p| p.port).collect::<Vec<_>>(),
        vec![22, 445, 5000]
    );

    let hue = by_ip(&list, "192.168.1.130");
    assert_eq!(banner_of(hue, 80), Some("hue personal wireless lighting"));

    for d in &list {
        if d.is_self {
            assert_eq!(d.deep_scanned_at, None, "the laptop is never deep scanned");
        } else {
            assert!(
                d.deep_scanned_at.is_some(),
                "{:?} was not deep scanned",
                d.ip
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn deep_scan_progress_counts_probes() {
    let mut rig = rig(scenario("home"));
    let (events, _) = deep_scan_of(&mut rig).await;
    let (state, _, _, devices_found) = done_of(&events);
    assert_eq!(state, ScanState::Done);
    let others = devices_found - 1;

    let progress: Vec<(ScanStage, u32, u32)> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::ScanProgress {
                stage, done, total, ..
            } => Some((stage, done, total)),
            _ => None,
        })
        .collect();
    let of = |wanted: ScanStage| -> Vec<(u32, u32)> {
        progress
            .iter()
            .filter(|(stage, ..)| *stage == wanted)
            .map(|(_, done, total)| (*done, *total))
            .collect()
    };

    let port_scan = of(ScanStage::PortScan);
    assert!(port_scan.len() > 2, "{port_scan:?}");
    assert!(port_scan.iter().all(|(_, total)| *total == 100 * others));
    assert!(port_scan.windows(2).all(|w| w[0].0 < w[1].0));
    let (done, total) = *port_scan.last().unwrap();
    assert_eq!(done, total);
    assert!(of(ScanStage::Ports).is_empty());

    let banners = of(ScanStage::Banners);
    let (done, total) = *banners.last().expect("a banners stage");
    assert_eq!(done, total);
    assert!(total > 0);

    // Port scan, then banners, then the rest, then done.
    let stages: Vec<ScanStage> = progress.iter().map(|(s, ..)| *s).collect();
    let last_port_scan = stages
        .iter()
        .rposition(|s| *s == ScanStage::PortScan)
        .unwrap();
    let first_banner = stages
        .iter()
        .position(|s| *s == ScanStage::Banners)
        .unwrap();
    let first_discover = stages
        .iter()
        .position(|s| *s == ScanStage::Discover)
        .unwrap();
    assert!(last_port_scan < first_banner && first_banner < first_discover);
    assert!(matches!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            state: ScanState::Done,
            ..
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn per_device_connects_stay_at_eight() {
    let mut platform = CountingPlatform::new(scenario("isolated"), Vec::new());
    platform.probe_delay = Duration::from_millis(100);
    let platform = Arc::new(platform);
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("isolated")),
        store(),
        ScanConfig::default(),
    );
    rig.mgr.set_scan_consent(true).await.unwrap();
    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);
    assert_eq!(platform.peak_connects(ip("192.168.1.1")), 8);
    // The laptop is never probed.
    assert_eq!(platform.peak_connects(ip("192.168.1.23")), 0);
}

/// The home scenario with the router serving web pages on 12 ports.
fn home_with_a_busy_router() -> SimNetwork {
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    for d in json["devices"].as_array_mut().unwrap() {
        if d["ip"] == "192.168.1.1" {
            d["open_ports"] = serde_json::json!([
                80, 81, 1400, 2869, 3000, 3001, 5000, 5800, 7547, 8000, 8001, 8002
            ]);
        }
    }
    SimNetwork::from_json(&json.to_string()).unwrap()
}

/// The address in a `scheme://a.b.c.d[:port]/...` URL.
fn url_host(url: &str) -> Ipv4Addr {
    let rest = url.split("://").nth(1).unwrap();
    let host = rest.split(['/', ':']).next().unwrap();
    host.parse().unwrap()
}

/// A simulated network's I/O where each banner read (a greeting or a LAN
/// page) takes 100 ms, counting the reads in flight per address.
struct CountingBanners {
    sim: SimNetwork,
    /// Per address: reads in flight now, and the most ever in flight.
    reading: Mutex<HashMap<Ipv4Addr, (usize, usize)>>,
}

impl CountingBanners {
    async fn held<T>(&self, addr: Ipv4Addr, read: impl std::future::Future<Output = T>) -> T {
        {
            let mut reading = self.reading.lock().unwrap();
            let (now, peak) = reading.entry(addr).or_default();
            *now += 1;
            *peak = (*peak).max(*now);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = read.await;
        self.reading.lock().unwrap().get_mut(&addr).unwrap().0 -= 1;
        result
    }

    fn peak(&self, addr: Ipv4Addr) -> usize {
        self.reading
            .lock()
            .unwrap()
            .get(&addr)
            .map_or(0, |(_, peak)| *peak)
    }
}

#[async_trait]
impl NetIo for CountingBanners {
    async fn dns_resolve(&self, s: Ipv4Addr, n: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
        self.sim.dns_resolve(s, n).await
    }
    async fn http_get(&self, u: &str, f: bool) -> Result<HttpResponse, PlatformError> {
        self.sim.http_get(u, f).await
    }
    async fn reverse_dns(&self, s: Ipv4Addr, a: Ipv4Addr) -> Result<Option<String>, PlatformError> {
        self.sim.reverse_dns(s, a).await
    }
    async fn mdns_browse(
        &self,
        l: Ipv4Addr,
        w: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        self.sim.mdns_browse(l, w).await
    }
    async fn ssdp_search(&self, l: Ipv4Addr, w: Duration) -> Result<Vec<SsdpReply>, PlatformError> {
        self.sim.ssdp_search(l, w).await
    }
    async fn netbios_status(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        self.sim.netbios_status(a, t).await
    }
    async fn mdns_query(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MdnsHost>, PlatformError> {
        self.sim.mdns_query(a, t).await
    }
    async fn tcp_banner(
        &self,
        a: Ipv4Addr,
        p: u16,
        t: Duration,
    ) -> Result<Option<String>, PlatformError> {
        self.held(a, self.sim.tcp_banner(a, p, t)).await
    }
    async fn lan_http_get(&self, u: &str, f: bool) -> Result<HttpResponse, PlatformError> {
        self.held(url_host(u), self.sim.lan_http_get(u, f)).await
    }
}

#[tokio::test(start_paused = true)]
async fn banner_reads_stay_at_eight_per_device() {
    let netio = Arc::new(CountingBanners {
        sim: home_with_a_busy_router(),
        reading: Mutex::default(),
    });
    let mut rig = rig_on(
        Arc::new(home_with_a_busy_router()),
        netio.clone(),
        store(),
        ScanConfig::default(),
    );
    rig.mgr.set_scan_consent(true).await.unwrap();
    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);
    // 12 pages to read on the router, 16 reads allowed in all: the
    // per-device cap holds it at 8.
    assert_eq!(netio.peak(ip("192.168.1.1")), 8);
}

/// The home network's I/O, except that reading a service greeting never
/// returns.
struct HangingBanners(SimNetwork);

#[async_trait]
impl NetIo for HangingBanners {
    async fn dns_resolve(&self, s: Ipv4Addr, n: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
        self.0.dns_resolve(s, n).await
    }
    async fn http_get(&self, u: &str, f: bool) -> Result<HttpResponse, PlatformError> {
        self.0.http_get(u, f).await
    }
    async fn reverse_dns(&self, s: Ipv4Addr, a: Ipv4Addr) -> Result<Option<String>, PlatformError> {
        self.0.reverse_dns(s, a).await
    }
    async fn mdns_browse(
        &self,
        l: Ipv4Addr,
        w: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        self.0.mdns_browse(l, w).await
    }
    async fn ssdp_search(&self, l: Ipv4Addr, w: Duration) -> Result<Vec<SsdpReply>, PlatformError> {
        self.0.ssdp_search(l, w).await
    }
    async fn netbios_status(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        self.0.netbios_status(a, t).await
    }
    async fn mdns_query(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MdnsHost>, PlatformError> {
        self.0.mdns_query(a, t).await
    }
    async fn tcp_banner(
        &self,
        _: Ipv4Addr,
        _: u16,
        _: Duration,
    ) -> Result<Option<String>, PlatformError> {
        std::future::pending().await
    }
    async fn lan_http_get(&self, u: &str, f: bool) -> Result<HttpResponse, PlatformError> {
        self.0.lan_http_get(u, f).await
    }
}

#[tokio::test(start_paused = true)]
async fn a_hanging_banner_read_costs_its_timeout() {
    let mut rig = rig_on(
        Arc::new(scenario("home")),
        Arc::new(HangingBanners(scenario("home"))),
        store(),
        ScanConfig::default(),
    );
    rig.mgr.set_scan_consent(true).await.unwrap();
    let begun = Instant::now();
    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let timed = collect_until_done_timed(&mut rig.rx, begun).await;
    let events: Vec<Event> = timed.iter().map(|(e, _)| e.clone()).collect();
    assert_eq!(done_of(&events).0, ScanState::Done);
    let net = rig.mgr.current_network().await.unwrap().unwrap();
    let list = rig.store.list_devices(net).await.unwrap();
    let nas = by_ip(&list, "192.168.1.70");
    assert!(nas.ports.iter().any(|p| p.port == 22));
    assert_eq!(banner_of(nas, 22), None);
    // Page titles still arrive.
    assert_eq!(banner_of(nas, 5000), Some("DiskStation"));

    // The banners stage (virtual time, from its first event to the first
    // event of the next stage) lasts as long as the one hung greeting (ssh on
    // .70) is allowed: its timeout plus the grace, and no more. Every other
    // read answers at once.
    let first_at = |wanted: ScanStage| {
        timed
            .iter()
            .find_map(|(e, at)| match e.kind {
                EventKind::ScanProgress { stage, .. } if stage == wanted => Some(*at),
                _ => None,
            })
            .unwrap()
    };
    let spent = first_at(ScanStage::Discover) - first_at(ScanStage::Banners);
    let timeout = ScanConfig::default().banner_timeout;
    assert!(
        spent >= timeout && spent <= timeout + Duration::from_millis(500),
        "the banners stage took {spent:?}"
    );
}

/// A platform whose network can be swapped underneath it, which can hide the
/// gateway from the ARP table, and which counts TCP probes.
struct Moving {
    sim: Mutex<Arc<SimNetwork>>,
    hide_gateway: std::sync::atomic::AtomicBool,
    probes: AtomicUsize,
}

impl Moving {
    fn new(sim: SimNetwork) -> Self {
        Self {
            sim: Mutex::new(Arc::new(sim)),
            hide_gateway: Default::default(),
            probes: AtomicUsize::new(0),
        }
    }
    fn sim(&self) -> Arc<SimNetwork> {
        self.sim.lock().unwrap().clone()
    }
}

#[async_trait]
impl Platform for Moving {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }
    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        Err(PlatformError::Unavailable {
            reason: "no Wi-Fi".into(),
        })
    }
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.sim().active_interface().await
    }
    async fn ping(&self, a: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.sim().ping(a, t).await
    }
    async fn arp_resolve(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.sim().arp_resolve(a, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        let mut table = self.sim().arp_table().await?;
        if self.hide_gateway.load(Ordering::SeqCst) {
            table.retain(|e| e.ip != ip("192.168.1.1"));
        }
        Ok(table)
    }
    async fn tcp_probe(&self, a: Ipv4Addr, p: u16, t: Duration) -> Result<TcpProbe, PlatformError> {
        self.probes.fetch_add(1, Ordering::SeqCst);
        self.sim().tcp_probe(a, p, t).await
    }
}

const NO_CONSENT: &str = "Deep scan needs your permission for this network.";

#[tokio::test(start_paused = true)]
async fn a_deep_scan_behind_another_router_needs_its_own_consent() {
    let platform = Arc::new(Moving::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    let a = rig.mgr.set_scan_consent(true).await.unwrap();

    // Same addressing behind another router, whose entry is not in the ARP
    // table yet: the passive check still sees the consented network.
    *platform.sim.lock().unwrap() = Arc::new(home_with_another_router());
    platform.hide_gateway.store(true, Ordering::SeqCst);
    assert_eq!(rig.mgr.current_network().await.unwrap(), Some(a));

    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(
        done_of(&events),
        (ScanState::Failed, Some(NO_CONSENT.into()), false, 0)
    );
    assert!(streamed(&events).is_empty());
    assert_eq!(platform.probes.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn consent_revoked_before_the_scan_identifies_its_network_stops_it() {
    let platform = Arc::new(Moving::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    let net = rig.mgr.set_scan_consent(true).await.unwrap();
    started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    // The scan task has not run yet (one thread), and the store serves calls
    // in order, so this lands before the scan reads the consent.
    rig.store.set_scan_consent(net, false).await.unwrap();

    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(
        done_of(&events),
        (ScanState::Failed, Some(NO_CONSENT.into()), false, 0)
    );
    assert_eq!(platform.probes.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn an_offline_deep_scan_is_not_connected() {
    let Rig { mgr, bus, .. } = rig(scenario("offline"));
    assert_eq!(
        mgr.start(ScanMode::Deep).await,
        Err(CoreError::NotConnected)
    );
    assert_eq!(bus.last_seq(), 0);
}

// ------------------------------------------------------------- stopping scans

const NETWORK_CHANGED: &str = "The network changed during the scan.";
const CONNECTION_DROPPED: &str = "The connection dropped during the scan.";

/// The home network, except that every TCP probe takes a second.
struct SlowProbes(SimNetwork);

#[async_trait]
impl Platform for SlowProbes {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        self.0.wifi_current().await
    }
    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        self.0.wifi_scan().await
    }
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.0.active_interface().await
    }
    async fn ping(&self, a: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.0.ping(a, t).await
    }
    async fn arp_resolve(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.0.arp_resolve(a, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.0.arp_table().await
    }
    async fn tcp_probe(&self, a: Ipv4Addr, p: u16, t: Duration) -> Result<TcpProbe, PlatformError> {
        tokio::time::sleep(secs(1)).await;
        self.0.tcp_probe(a, p, t).await
    }
}

/// Collects events until the first progress event of `stage`, inclusive.
async fn collect_until_stage(rx: &mut broadcast::Receiver<Event>, stage: ScanStage) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        let event = rx.recv().await.unwrap();
        let reached = matches!(event.kind, EventKind::ScanProgress { stage: s, .. } if s == stage);
        assert!(
            !matches!(event.kind, EventKind::ScanDone { .. }),
            "the scan ended before {stage:?}: {:?}",
            event.kind
        );
        events.push(event);
        if reached {
            return events;
        }
    }
}

/// Collects events until a sweep progress event past zero, inclusive.
async fn collect_into_the_sweep(rx: &mut broadcast::Receiver<Event>) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        let event = rx.recv().await.unwrap();
        let reached = matches!(
            event.kind,
            EventKind::ScanProgress { stage: ScanStage::Sweep, done, .. } if done > 0
        );
        assert!(!matches!(event.kind, EventKind::ScanDone { .. }));
        events.push(event);
        if reached {
            return events;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_stopped_deep_scan_ends_cancelled_and_keeps_its_devices() {
    let mut rig = rig_on(
        Arc::new(SlowProbes(scenario("home"))),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    let net = rig.mgr.set_scan_consent(true).await.unwrap();
    let id = started(rig.mgr.start(ScanMode::Deep).await.unwrap());
    let mut events = collect_until_stage(&mut rig.rx, ScanStage::PortScan).await;
    assert!(rig.mgr.cancel(id));
    events.extend(collect_until_done(&mut rig.rx).await);

    let (state, error, isolated, devices_found) = done_of(&events);
    assert_eq!(
        (state, error, isolated),
        (ScanState::Cancelled, None, false)
    );
    let list = rig.store.list_devices(net).await.unwrap();
    assert!(!list.is_empty());
    assert_eq!(devices_found as usize, streamed(&events).len());
    assert!(devices_found > 0);
    let stored = rig.store.scan(id).await.unwrap().unwrap();
    assert_eq!(stored.state, ScanState::Cancelled);
    assert_eq!(stored.error, None);
    assert_eq!(stored.devices_found, devices_found);
    let snapshot = rig.mgr.snapshot(id).await.unwrap().unwrap();
    assert_eq!(snapshot.state, ScanState::Cancelled);
    assert!(!rig.mgr.cancel(id));

    // Exactly one scan.done, and the guard is free straight away.
    assert!(matches!(
        rig.rx.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
}

/// The home scenario behind another gateway address.
fn home_with_another_gateway() -> SimNetwork {
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["interface"]["gateway"] = "192.168.1.254".into();
    SimNetwork::from_json(&json.to_string()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_network_change_cancels_the_scan_with_a_reason() {
    let platform = Arc::new(Switchable::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    let id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let mut events = collect_into_the_sweep(&mut rig.rx).await;
    platform.switch(home_with_another_gateway());
    events.extend(collect_until_done(&mut rig.rx).await);

    let (state, error, isolated, _) = done_of(&events);
    assert_eq!(
        (state, error.as_deref(), isolated),
        (ScanState::Cancelled, Some(NETWORK_CHANGED), false)
    );
    let stored = rig.store.scan(id).await.unwrap().unwrap();
    assert_eq!(stored.state, ScanState::Cancelled);
    assert_eq!(stored.error.as_deref(), Some(NETWORK_CHANGED));
    assert!(!rig.mgr.cancel(id));
}

#[tokio::test(start_paused = true)]
async fn losing_the_connection_cancels_the_scan_with_a_reason() {
    let platform = Arc::new(Switchable::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let mut events = collect_into_the_sweep(&mut rig.rx).await;
    platform.drop_link();
    events.extend(collect_until_done(&mut rig.rx).await);

    let (state, error, isolated, _) = done_of(&events);
    assert_eq!(
        (state, error.as_deref(), isolated),
        (ScanState::Cancelled, Some(CONNECTION_DROPPED), false)
    );
}

#[tokio::test(start_paused = true)]
async fn cancel_of_an_unknown_scan_is_false() {
    let mut rig = rig(scenario("home"));
    assert!(!rig.mgr.cancel(ScanId(42)));

    // Nor is a running scan stopped by a cancel of another id.
    let id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    assert!(!rig.mgr.cancel(ScanId(id.0 + 1)));
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);
    assert!(!rig.mgr.cancel(id));
}

// ------------------------------------------------------------ replaced router

/// The home router's MAC, and the Wi-Fi name `home.json` reports.
const HOME_ROUTER: &str = "50:C7:BF:3A:11:02";
const HOME_SSID: &str = "Lantern Home";
const HOME_SUBNET: &str = "192.168.1.0/24";
/// The router the home network had before.
const OLD_ROUTER: &str = "50:C7:BF:00:00:01";

fn mac(s: &str) -> MacAddr {
    s.parse().unwrap()
}

/// The home scenario with its whole JSON edited.
fn home_json(edit: impl FnOnce(&mut serde_json::Value)) -> SimNetwork {
    let text = std::fs::read_to_string(scenario_path("home")).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    edit(&mut json);
    SimNetwork::from_json(&json.to_string()).unwrap()
}

/// A network known under the home SSID and subnet, behind `router`.
async fn known_network(store: &Store, router: &str, seen: i64) -> NetworkId {
    let id = store
        .network_for(mac(router), HOME_SUBNET, seen)
        .await
        .unwrap();
    store.set_ssid_label(id, HOME_SSID).await.unwrap();
    id
}

/// A rig on the home scenario over `store`.
fn home_rig_on(store: Store) -> Rig {
    let sim = Arc::new(scenario("home"));
    rig_on(sim.clone(), sim, store, ScanConfig::default())
}

#[tokio::test(start_paused = true)]
async fn replaced_router_is_offered_and_blocks_scans() {
    let store = store();
    let old = known_network(&store, OLD_ROUTER, 10).await;
    let rig = home_rig_on(store);
    assert_eq!(
        rig.mgr.replaced_router().await.unwrap(),
        Some(ReplacedRouter {
            network_id: old,
            label: HOME_SSID.to_string(),
        })
    );
    assert_eq!(
        rig.mgr.start(ScanMode::Quick).await,
        Err(CoreError::NetworkDecisionNeeded {
            candidate: old,
            label: HOME_SSID.to_string(),
        })
    );
    // No scan was created.
    assert_eq!(rig.store.scan(ScanId(1)).await.unwrap(), None);
    // Deep-scan consent is refused the same way.
    assert_eq!(
        rig.mgr.set_scan_consent(true).await,
        Err(CoreError::NetworkDecisionNeeded {
            candidate: old,
            label: HOME_SSID.to_string(),
        })
    );
    assert_eq!(
        rig.store
            .find_network(mac(HOME_ROUTER), HOME_SUBNET)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test(start_paused = true)]
async fn keeping_moves_the_old_network_and_its_devices_to_the_new_router() {
    // The home network scanned behind its old router, with a device named.
    let store = store();
    let before = Arc::new(home_edited(|devices| {
        for d in devices {
            if d["ip"] == "192.168.1.1" {
                d["mac"] = OLD_ROUTER.into();
            }
        }
    }));
    let mut seeding = rig_on(before.clone(), before, store.clone(), ScanConfig::default());
    started(seeding.mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut seeding.rx).await;
    let old = store
        .find_network(mac(OLD_ROUTER), HOME_SUBNET)
        .await
        .unwrap()
        .unwrap();
    store.set_ssid_label(old, HOME_SSID).await.unwrap();
    let tv = by_ip(&store.list_devices(old).await.unwrap(), "192.168.1.52").clone();
    store
        .update_device(
            old,
            tv.id,
            DevicePatch {
                name: Some(Some("Lounge TV".into())),
                starred: None,
                is_new: None,
            },
        )
        .await
        .unwrap()
        .unwrap();

    // The router is replaced: keep the network.
    let mut rig = home_rig_on(store);
    assert_eq!(rig.mgr.decide_replaced_router(true).await, Ok(old));
    assert_eq!(rig.mgr.replaced_router().await.unwrap(), None);
    assert_eq!(rig.mgr.current_network().await.unwrap(), Some(old));
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);

    assert_eq!(rig.mgr.current_network().await.unwrap(), Some(old));
    assert_eq!(
        rig.store
            .find_network(mac(HOME_ROUTER), HOME_SUBNET)
            .await
            .unwrap(),
        Some(old)
    );
    let list = rig.store.list_devices(old).await.unwrap();
    let now = by_ip(&list, "192.168.1.52");
    assert_eq!(now.id, tv.id);
    assert_eq!(now.display_name, "Lounge TV");
    assert!(now.online);
}

#[tokio::test(start_paused = true)]
async fn declining_creates_a_new_network() {
    let store = store();
    let old = known_network(&store, OLD_ROUTER, 10).await;
    let mut rig = home_rig_on(store);
    let new = rig.mgr.decide_replaced_router(false).await.unwrap();
    assert_ne!(new, old);
    assert_eq!(rig.mgr.replaced_router().await.unwrap(), None);

    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);

    // The old network keeps its router; the scan's devices are on the new one.
    assert_eq!(
        rig.store
            .find_network(mac(OLD_ROUTER), HOME_SUBNET)
            .await
            .unwrap(),
        Some(old)
    );
    assert_eq!(rig.mgr.current_network().await.unwrap(), Some(new));
    assert_eq!(rig.store.list_devices(new).await.unwrap().len(), 13);
    assert!(rig.store.list_devices(old).await.unwrap().is_empty());
    assert_eq!(rig.mgr.replaced_router().await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn deciding_a_replaced_router_with_nothing_pending_is_unidentified() {
    let rig = home_rig_on(store());
    assert!(matches!(
        rig.mgr.decide_replaced_router(true).await,
        Err(CoreError::Unidentified(_))
    ));
    assert_eq!(
        rig.store
            .find_network(mac(HOME_ROUTER), HOME_SUBNET)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test(start_paused = true)]
async fn replaced_router_without_an_ssid_is_not_offered() {
    // Location permission off: Windows hides the SSID, so no prompt is
    // possible and the network is new, as before.
    let store = store();
    let old = known_network(&store, OLD_ROUTER, 10).await;
    let sim = Arc::new(home_json(|json| {
        json["wifi"]["permission"] = "denied".into();
    }));
    let mut rig = rig_on(sim.clone(), sim, store, ScanConfig::default());
    assert_eq!(rig.mgr.replaced_router().await.unwrap(), None);
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(done_of(&events).0, ScanState::Done);
    let new = rig.mgr.current_network().await.unwrap().unwrap();
    assert_ne!(new, old);
    assert_eq!(
        rig.store
            .find_network(mac(HOME_ROUTER), HOME_SUBNET)
            .await
            .unwrap(),
        Some(new)
    );
}

#[tokio::test(start_paused = true)]
async fn replaced_router_offers_the_newest_candidate() {
    let store = store();
    known_network(&store, OLD_ROUTER, 10).await;
    let newer = known_network(&store, "50:C7:BF:00:00:02", 20).await;
    let rig = home_rig_on(store);
    assert_eq!(
        rig.mgr
            .replaced_router()
            .await
            .unwrap()
            .map(|r| r.network_id),
        Some(newer)
    );
}

/// The home scenario, whose Wi-Fi reads can be held until the test lets
/// them through.
struct HeldWifi {
    sim: SimNetwork,
    hold: std::sync::atomic::AtomicBool,
    gate: tokio::sync::Semaphore,
}

impl HeldWifi {
    fn new(sim: SimNetwork) -> Self {
        Self {
            sim,
            hold: Default::default(),
            gate: tokio::sync::Semaphore::new(0),
        }
    }
    fn release(&self) {
        self.gate.add_permits(1024);
    }
}

#[async_trait]
impl Platform for HeldWifi {
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        if self.hold.load(Ordering::SeqCst) {
            let _permit = self.gate.acquire().await;
        }
        self.sim.wifi_current().await
    }
    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        self.sim.wifi_scan().await
    }
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.sim.active_interface().await
    }
    async fn ping(&self, a: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.sim.ping(a, t).await
    }
    async fn arp_resolve(
        &self,
        a: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.sim.arp_resolve(a, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.sim.arp_table().await
    }
    async fn tcp_probe(&self, a: Ipv4Addr, p: u16, t: Duration) -> Result<TcpProbe, PlatformError> {
        self.sim.tcp_probe(a, p, t).await
    }
}

#[tokio::test(start_paused = true)]
async fn replaced_router_found_by_a_running_scan_fails_it_without_writing() {
    let platform = Arc::new(HeldWifi::new(scenario("home")));
    let mut rig = rig_on(
        platform.clone(),
        Arc::new(scenario("home")),
        store(),
        ScanConfig::default(),
    );
    let id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    // The scan task has not run yet (one thread): hold its Wi-Fi read, so it
    // identifies the network only once the old one is known.
    platform.hold.store(true, Ordering::SeqCst);
    let old = known_network(&rig.store, OLD_ROUTER, 10).await;
    platform.release();

    let events = collect_until_done(&mut rig.rx).await;
    assert_eq!(
        done_of(&events),
        (
            ScanState::Failed,
            Some(NEW_ROUTER_TEXT.to_string()),
            false,
            0
        )
    );
    assert!(streamed(&events).is_empty());
    assert_eq!(
        rig.store
            .find_network(mac(HOME_ROUTER), HOME_SUBNET)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        rig.store.scan(id).await.unwrap().unwrap().state,
        ScanState::Failed
    );
    assert_eq!(
        rig.mgr
            .replaced_router()
            .await
            .unwrap()
            .map(|r| r.network_id),
        Some(old)
    );
}

#[tokio::test(start_paused = true)]
async fn identification_records_the_ssid_label() {
    let mut rig = rig(scenario("home"));
    started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rig.rx).await;
    let net = rig.mgr.current_network().await.unwrap().unwrap();
    // Seen behind another router on the same addressing, it is the
    // candidate, under the SSID it was identified with.
    assert_eq!(
        rig.store
            .replaced_router_candidate(mac(OLD_ROUTER), HOME_SUBNET, HOME_SSID)
            .await
            .unwrap(),
        Some((net, HOME_SSID.to_string()))
    );
}
