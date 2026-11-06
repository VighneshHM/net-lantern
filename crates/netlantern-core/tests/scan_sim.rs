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
    ScanConfig, ScanId, ScanManager, ScanMode, ScanStage, ScanState, StartOutcome, Store,
};
use netlantern_platform::{
    ArpEntry, HttpResponse, Interface, MacAddr, MdnsService, NeighborState, NetIo, NetbiosInfo,
    PingReply, Platform, PlatformError, SimNetwork, SsdpReply, TcpProbe,
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
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        let earlier = self.lookups.fetch_add(1, Ordering::SeqCst);
        if self.faults.disconnects_after_first_lookup && earlier > 0 {
            return Err(PlatformError::NotConnected);
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
        self.sim.tcp_probe(ip, port, timeout).await
    }
}

fn faulty(faults: Faults) -> Rig {
    rig_with(Faulty::new(scenario("home"), faults), ScanConfig::default())
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

/// The home network, plus `Unreachable` ARP entries for every other swept
/// address except three; counts `arp_resolve` calls per address.
struct CountingPlatform {
    sim: SimNetwork,
    without_entry: Vec<Ipv4Addr>,
    resolved: Mutex<Vec<Ipv4Addr>>,
}

#[async_trait]
impl Platform for CountingPlatform {
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
        self.sim.tcp_probe(ip, p, t).await
    }
}

#[tokio::test(start_paused = true)]
async fn arp_resolve_only_for_addresses_without_entries() {
    let without_entry = vec![
        ip("192.168.1.200"),
        ip("192.168.1.201"),
        ip("192.168.1.202"),
    ];
    let platform = Arc::new(CountingPlatform {
        sim: scenario("home"),
        without_entry: without_entry.clone(),
        resolved: Mutex::default(),
    });
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

    // Deep: unsupported, and does not use up scan id 1.
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));
    let err = mgr.start(ScanMode::Deep).await.unwrap_err();
    assert_eq!(err, CoreError::Unsupported("deep scan arrives in phase 4"));
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

    let racers: Vec<_> = (0..4)
        .map(|_| {
            let mgr = Arc::clone(&mgr);
            tokio::spawn(async move {
                for _ in 0..20_000 {
                    mgr.start(ScanMode::Quick).await.unwrap();
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
    assert!(scans > 100, "only {scans} scans raced");
}

/// A platform whose network can be swapped underneath it, as when the laptop
/// moves between networks.
struct Switchable(Mutex<Arc<SimNetwork>>);

impl Switchable {
    fn new(sim: SimNetwork) -> Self {
        Self(Mutex::new(Arc::new(sim)))
    }
    fn switch(&self, sim: SimNetwork) {
        *self.0.lock().unwrap() = Arc::new(sim);
    }
    fn sim(&self) -> Arc<SimNetwork> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait]
impl Platform for Switchable {
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
