//! End-to-end quick-scan tests against the simulated networks in `scenarios/`.
//!
//! Every test but one runs with a paused clock, so the simulated ping latencies
//! and timeouts cost no real time. The exception races real threads against an
//! instant network.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use netbot_core::{
    CoreError, DeviceStore, DeviceSummary, Event, EventBus, EventKind, ScanConfig, ScanId,
    ScanManager, ScanMode, ScanStage, ScanState, StartOutcome,
};
use netbot_platform::{
    ArpEntry, Interface, MacAddr, PingReply, Platform, PlatformError, SimNetwork, TcpProbe,
};
use tokio::sync::broadcast;

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn scenario(name: &str) -> SimNetwork {
    SimNetwork::load(format!(
        "{}/../../scenarios/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A manager wired to an event bus and device store, plus a subscription that
/// was taken out before any scan could publish.
struct Rig {
    mgr: ScanManager,
    bus: Arc<EventBus>,
    store: Arc<DeviceStore>,
    rx: broadcast::Receiver<Event>,
}

fn rig(platform: impl Platform + 'static) -> Rig {
    rig_with(platform, ScanConfig::default())
}

fn rig_with(platform: impl Platform + 'static, config: ScanConfig) -> Rig {
    let bus = Arc::new(EventBus::new(1024));
    let rx = bus.subscribe();
    let store = Arc::new(DeviceStore::default());
    let mgr = ScanManager::new(
        Arc::new(platform),
        Arc::clone(&bus),
        Arc::clone(&store),
        config,
    );
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

#[tokio::test(start_paused = true)]
async fn quick_scan_completes_with_every_present_device() {
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    assert_eq!(id, ScanId(1));
    // A second start while the first is still running joins it.
    assert_eq!(
        mgr.start(ScanMode::Quick).await.unwrap(),
        StartOutcome::AlreadyRunning(id)
    );

    let events = collect_until_done(&mut rx).await;
    assert_eq!(events[0].seq, 1);
    assert!(events.windows(2).all(|w| w[1].seq == w[0].seq + 1));
    assert_eq!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            scan_id: id,
            state: ScanState::Done,
            devices_found: 13
        }
    );

    let snapshot = mgr.snapshot(id).unwrap();
    assert_eq!(snapshot.mode, ScanMode::Quick);
    assert_eq!(
        (
            snapshot.state,
            snapshot.stage,
            snapshot.done,
            snapshot.total,
            snapshot.devices_found
        ),
        (ScanState::Done, ScanStage::Merge, 254, 254, 13)
    );
    assert_eq!(mgr.snapshot(ScanId(2)), None);
}

#[tokio::test(start_paused = true)]
async fn events_move_through_sweep_arp_merge_then_done() {
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));
    started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;

    // Sweep progress arrives every 16 replies and once more at the very end.
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
    let expected: Vec<u32> = (1..=15).map(|n| n * 16).chain([254]).collect();
    assert_eq!(sweep, expected);

    // The Arp and Merge stages each announce themselves once, already at 100%.
    let later_stages: Vec<(ScanStage, u32, u32)> = events
        .iter()
        .filter_map(|e| match e.kind {
            EventKind::ScanProgress {
                done, total, stage, ..
            } if stage != ScanStage::Sweep => Some((stage, done, total)),
            _ => None,
        })
        .collect();
    assert_eq!(
        later_stages,
        vec![(ScanStage::Arp, 254, 254), (ScanStage::Merge, 254, 254)]
    );

    // Events appear in pipeline order: sweep, arp, merge, devices, done.
    let rank = |kind: &EventKind| match kind {
        EventKind::ScanProgress {
            stage: ScanStage::Sweep,
            ..
        } => 0,
        EventKind::ScanProgress {
            stage: ScanStage::Arp,
            ..
        } => 1,
        EventKind::ScanProgress {
            stage: ScanStage::Merge,
            ..
        } => 2,
        EventKind::ScanDevice { .. } => 3,
        EventKind::ScanDone { .. } => 4,
    };
    let ranks: Vec<u8> = events.iter().map(|e| rank(&e.kind)).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "out of order: {ranks:?}"
    );
    assert_eq!(ranks.iter().filter(|&&r| r == 3).count(), 13);
}

#[tokio::test(start_paused = true)]
async fn devices_are_merged_from_ping_and_arp() {
    let Rig {
        mgr, store, mut rx, ..
    } = rig(scenario("home"));
    started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;

    let list = store.list();
    assert_eq!(list.len(), 13);
    assert!(list.windows(2).all(|w| w[0].ip < w[1].ip));
    let device = |addr: &str| -> &DeviceSummary { list.iter().find(|d| d.ip == ip(addr)).unwrap() };

    // Answers ARP but ignores ping: present, with a MAC and no round-trip time.
    let quiet = device("192.168.1.88");
    assert!(!quiet.answers_ping && quiet.mac.is_some());
    assert_eq!(quiet.rtt_ms, None);

    let this_laptop = device("192.168.1.23");
    assert!(this_laptop.is_self);
    assert_eq!(
        this_laptop.mac,
        Some("F4:8E:38:9A:20:7C".parse::<MacAddr>().unwrap())
    );
    assert_eq!(list.iter().filter(|d| d.is_self).count(), 1);

    let gateway = device("192.168.1.1");
    assert!(gateway.is_gateway && gateway.answers_ping);
    assert_eq!(gateway.rtt_ms, Some(2));
    assert_eq!(list.iter().filter(|d| d.is_gateway).count(), 1);

    assert!(device("192.168.1.44").private_mac);
    assert_eq!(list.iter().filter(|d| d.private_mac).count(), 1);

    // The stream carries exactly what the store holds.
    let streamed: Vec<DeviceSummary> = events
        .into_iter()
        .filter_map(|e| match e.kind {
            EventKind::ScanDevice { device, .. } => Some(device),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, list);
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
            devices_found: 13
        }
    ));
    assert_eq!(mgr.snapshot(first).unwrap().state, ScanState::Done);
    assert_eq!(mgr.snapshot(second).unwrap().state, ScanState::Done);
    // Re-scanning the same network updates devices rather than duplicating them.
    assert_eq!(store.list().len(), 13);
}

#[tokio::test(start_paused = true)]
async fn offline_start_fails_synchronously_without_side_effects() {
    let Rig { mgr, bus, .. } = rig(scenario("offline"));

    assert_eq!(
        mgr.start(ScanMode::Quick).await,
        Err(CoreError::NotConnected)
    );
    // Nothing was claimed, so asking again reports the same error rather than
    // "already running".
    assert_eq!(
        mgr.start(ScanMode::Quick).await,
        Err(CoreError::NotConnected)
    );
    assert_eq!(bus.last_seq(), 0);
    assert_eq!(mgr.snapshot(ScanId(1)), None);
}

#[tokio::test(start_paused = true)]
async fn deep_scan_is_unsupported_and_leaves_no_trace() {
    let Rig { mgr, mut rx, .. } = rig(scenario("home"));

    let err = mgr.start(ScanMode::Deep).await.unwrap_err();
    assert_eq!(err, CoreError::Unsupported("deep scan arrives in phase 4"));
    assert_eq!(mgr.snapshot(ScanId(1)), None);

    // The refused request did not use up scan id 1.
    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    assert_eq!(id, ScanId(1));
    collect_until_done(&mut rx).await;
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
}

/// A simulated network with injected faults.
struct Faulty {
    sim: SimNetwork,
    faults: Faults,
    pinged: Mutex<HashSet<Ipv4Addr>>,
    lookups: AtomicUsize,
}

impl Faulty {
    fn new(sim: SimNetwork, faults: Faults) -> Self {
        Self {
            sim,
            faults,
            pinged: Mutex::default(),
            lookups: AtomicUsize::new(0),
        }
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
        self.sim.arp_table().await
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

#[tokio::test(start_paused = true)]
async fn a_platform_error_mid_scan_fails_the_scan_and_frees_the_guard() {
    let Rig {
        mgr, store, mut rx, ..
    } = rig(Faulty::new(
        scenario("home"),
        Faults {
            arp_unreadable: true,
            ..Faults::default()
        },
    ));

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;

    assert_eq!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            scan_id: id,
            state: ScanState::Failed,
            devices_found: 0
        }
    );
    assert!(!events
        .iter()
        .any(|e| matches!(e.kind, EventKind::ScanDevice { .. })));
    assert_eq!(mgr.snapshot(id).unwrap().state, ScanState::Failed);
    assert!(store.list().is_empty());

    // A failed scan does not wedge the manager.
    assert_eq!(
        started(mgr.start(ScanMode::Quick).await.unwrap()),
        ScanId(2)
    );
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
    assert_eq!(mgr.snapshot(id).unwrap().state, ScanState::Done);
}

/// How many of the home devices answered ping in a scan with the given faults and config.
async fn pingers_found(faults: Faults, config: ScanConfig) -> (usize, usize) {
    let Rig {
        mgr, store, mut rx, ..
    } = rig_with(Faulty::new(scenario("home"), faults), config);
    started(mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rx).await;
    let list = store.list();
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
        ..ScanConfig::default()
    };
    assert_eq!(pingers_found(Faults::default(), config).await, (12, 13));
}

#[tokio::test(start_paused = true)]
async fn a_panicking_platform_fails_the_scan_and_frees_the_guard() {
    let Rig {
        mgr, store, mut rx, ..
    } = rig(Faulty::new(
        scenario("home"),
        Faults {
            arp_panics: true,
            ..Faults::default()
        },
    ));

    let id = started(mgr.start(ScanMode::Quick).await.unwrap());
    let events = collect_until_done(&mut rx).await;

    assert_eq!(
        events.last().unwrap().kind,
        EventKind::ScanDone {
            scan_id: id,
            state: ScanState::Failed,
            devices_found: 0
        }
    );
    assert_eq!(mgr.snapshot(id).unwrap().state, ScanState::Failed);
    assert!(store.list().is_empty());
    assert_eq!(
        started(mgr.start(ScanMode::Quick).await.unwrap()),
        ScanId(2)
    );
}

#[tokio::test(start_paused = true)]
async fn a_second_start_during_a_link_drop_joins_the_running_scan() {
    let Rig { mgr, mut rx, .. } = rig(Faulty::new(
        scenario("home"),
        Faults {
            disconnects_after_first_lookup: true,
            ..Faults::default()
        },
    ));

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

    let begun = tokio::time::Instant::now();
    started(mgr.start(ScanMode::Quick).await.unwrap());
    collect_until_done(&mut rx).await;
    let took = begun.elapsed();

    // ~240 silent addresses x 2 attempts x 800 ms is over 6 minutes one at a
    // time; 64 in flight brings it to a few seconds.
    assert!(took < Duration::from_secs(10), "quick scan took {took:?}");
}

/// A one-address network where nothing answers and every call returns at once,
/// so a scan finishes in microseconds.
struct Idle;

#[async_trait]
impl Platform for Idle {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        Ok(Interface {
            ip: ip("10.0.0.5"),
            prefix: 32,
            gateway: ip("10.0.0.1"),
            dns_servers: vec![],
            kind: netbot_platform::InterfaceKind::Wifi,
            mac: MacAddr([0xF4, 0x8E, 0x38, 0x9A, 0x20, 0x7C]),
        })
    }

    async fn ping(&self, _: Ipv4Addr, _: Duration) -> Result<Option<PingReply>, PlatformError> {
        Ok(None)
    }

    async fn arp_resolve(
        &self,
        _: Ipv4Addr,
        _: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        Ok(None)
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
        bus,
        Arc::new(DeviceStore::default()),
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
                if matches!(event.kind, EventKind::ScanDone { .. }) {
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
                for _ in 0..50_000 {
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
