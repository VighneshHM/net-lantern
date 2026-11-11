//! End-to-end monitoring tests: a quick scan identifies the simulated
//! network, then the monitor runs on tokio's paused clock against the
//! scenario's timeline, persisting into a store.
//!
//! The store runs on its own thread, so while the monitor waits for it the
//! paused runtime may jump to the next timer. Time is therefore moved in
//! small steps ([`STEP`]), which bounds each such jump.

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use netlantern_core::{
    Alert, AlertFilter, AlertState, Clock, CongestionConfig, Db, DeviceId, DevicePatch,
    DiagnoseConfig, DiagnoseManager, DiagnoseStart, EndReason, Event, EventBus, EventKind, Metric,
    Monitor, MonitorConfig, MonitorState, NetworkId, Notifier, Resolution, ScanConfig, ScanManager,
    ScanMode, SessionSpan, Store, SystemClock, Target, TimelineKind, TokioClock, WifiService,
};
use netlantern_platform::{
    ArpEntry, Interface, MacAddr, NearbyNetwork, PingReply, Platform, PlatformError, SimNetwork,
    TcpProbe, WifiConnection,
};
use tokio::sync::broadcast;
use tokio::time::Instant;

const STEP: Duration = Duration::from_millis(100);
const MIN: u64 = 60;
const HOUR: u64 = 3600;
const HOUR_MS: i64 = 3_600_000;

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn scenario(name: &str) -> SimNetwork {
    let path = format!("{}/../../scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    SimNetwork::load(path).unwrap()
}

/// The simulator as the platform, with a switch to drop the connection, a
/// one-shot panic in `wifi_current`, and a count of radio scans.
struct Wrapped {
    sim: Arc<SimNetwork>,
    connected: AtomicBool,
    panic_once: AtomicBool,
    wifi_scans: AtomicUsize,
}

impl Wrapped {
    fn check(&self) -> Result<(), PlatformError> {
        if self.connected.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(PlatformError::NotConnected)
        }
    }
}

#[async_trait]
impl Platform for Wrapped {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.check()?;
        self.sim.active_interface().await
    }
    async fn ping(&self, ip: Ipv4Addr, t: Duration) -> Result<Option<PingReply>, PlatformError> {
        self.check()?;
        self.sim.ping(ip, t).await
    }
    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        t: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.check()?;
        self.sim.arp_resolve(ip, t).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.check()?;
        self.sim.arp_table().await
    }
    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        t: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        self.check()?;
        self.sim.tcp_probe(ip, port, t).await
    }
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        self.check()?;
        if self.panic_once.swap(false, Ordering::SeqCst) {
            panic!("simulated platform panic");
        }
        self.sim.wifi_current().await
    }
    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        self.wifi_scans.fetch_add(1, Ordering::SeqCst);
        self.sim.wifi_scan().await
    }
}

/// Records every notification.
#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String)>>);

impl Notifier for Recorder {
    fn notify(&self, title: &str, body: &str) {
        self.0.lock().unwrap().push((title.into(), body.into()));
    }
}

impl Recorder {
    fn titles(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }
}

struct Rig {
    sim: Arc<SimNetwork>,
    platform: Arc<Wrapped>,
    clock: Arc<TokioClock>,
    /// The clock's reading when the scenario started.
    epoch: i64,
    /// Tokio time when the scenario started.
    t0: Instant,
    store: Store,
    bus: Arc<EventBus>,
    scans: Arc<ScanManager>,
    network: NetworkId,
    notes: Arc<Recorder>,
    rx: broadcast::Receiver<Event>,
}

async fn rig(name: &str) -> Rig {
    rig_on(name, Store::new(Db::open_in_memory().unwrap())).await
}

/// Builds the rig and runs a quick scan so the network is known.
async fn rig_on(name: &str, store: Store) -> Rig {
    let sim = Arc::new(scenario(name));
    let t0 = Instant::now();
    let epoch = SystemClock.now_ms();
    let clock = Arc::new(TokioClock::new(epoch));
    let platform = Arc::new(Wrapped {
        sim: Arc::clone(&sim),
        connected: AtomicBool::new(true),
        panic_once: AtomicBool::new(false),
        wifi_scans: AtomicUsize::new(0),
    });
    let bus = Arc::new(EventBus::new(16_384));
    let mut rx = bus.subscribe();
    let scans = Arc::new(ScanManager::new(
        platform.clone(),
        sim.clone(),
        Arc::clone(&bus),
        store.clone(),
        ScanConfig::default(),
    ));
    scans.start(ScanMode::Quick).await.unwrap();
    loop {
        match rx.recv().await {
            Ok(e) if matches!(e.kind, EventKind::ScanDone { .. }) => break,
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(e) => panic!("bus closed: {e}"),
        }
    }
    let network = scans.current_network().await.unwrap().unwrap();
    Rig {
        sim,
        platform,
        clock,
        epoch,
        t0,
        store,
        bus,
        scans,
        network,
        notes: Arc::new(Recorder::default()),
        rx,
    }
}

impl Rig {
    fn monitor(&self) -> Monitor {
        Monitor::new(
            self.platform.clone(),
            self.sim.clone(),
            self.store.clone(),
            Arc::clone(&self.bus),
            Arc::clone(&self.scans),
            self.notes.clone(),
            self.clock.clone(),
            MonitorConfig::default(),
        )
    }

    async fn star(&self, addr: &str) -> DeviceId {
        self.star_named(addr, None).await
    }

    /// Stars the device at `addr`, naming it when `name` is given.
    async fn star_named(&self, addr: &str, name: Option<&str>) -> DeviceId {
        let devices = self.store.list_devices(self.network).await.unwrap();
        let d = devices
            .iter()
            .find(|d| d.ip == Some(ip(addr)))
            .unwrap_or_else(|| panic!("no device at {addr}"));
        let patch = DevicePatch {
            starred: Some(true),
            name: name.map(|n| Some(n.to_string())),
            ..DevicePatch::default()
        };
        self.store
            .update_device(self.network, d.id, patch)
            .await
            .unwrap()
            .unwrap();
        d.id
    }

    /// Lets tokio time run until `s` scenario seconds.
    async fn run_until(&self, s: u64) {
        let target = self.t0 + Duration::from_secs(s);
        while Instant::now() < target {
            tokio::time::sleep(STEP.min(target - Instant::now())).await;
        }
    }

    async fn alerts(&self) -> Vec<Alert> {
        self.store
            .list_alerts(self.network, AlertFilter::All, 500)
            .await
            .unwrap()
    }

    async fn alert(&self, key: &str) -> Option<Alert> {
        self.alerts().await.into_iter().find(|a| a.key == key)
    }

    async fn sessions(&self) -> Vec<SessionSpan> {
        self.store
            .metrics(
                self.network,
                Target::Internet,
                Metric::RttMs,
                self.epoch - HOUR_MS,
                self.clock.now_ms() + HOUR_MS,
            )
            .await
            .unwrap()
            .sessions
    }

    /// Every event published since the last drain.
    fn drain(&mut self) -> Vec<EventKind> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(e) => out.push(e.kind),
                Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => return out,
            }
        }
    }
}

fn ticks(events: &[EventKind]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, EventKind::MonitorTick { .. }))
        .count()
}

#[tokio::test(start_paused = true)]
async fn printer_offline_alerts_within_two_and_a_half_minutes_and_resolves() {
    let r = rig("printer-offline").await;
    let printer = r.star_named("192.168.1.40", Some("Office printer")).await;
    let m = r.monitor();
    m.start();
    r.run_until(900).await;

    let key = format!("starred_offline:{}", printer.0);
    let alert = r.alert(&key).await.expect("an offline alert");
    let late = alert.opened_at - r.epoch - 600_000;
    assert!(
        (0..=150_000).contains(&late),
        "opened {late} ms after 600 s"
    );
    assert_eq!(alert.title, "Office printer is offline");
    assert!(!alert.suppressed);
    let incident = r
        .store
        .incident(r.network, alert.incident_id)
        .await
        .unwrap()
        .unwrap();
    let kinds: Vec<_> = incident.timeline.iter().map(|e| e.kind).collect();
    let kinds: Vec<_> = kinds
        .into_iter()
        .filter(|k| *k != TimelineKind::LastGood)
        .collect();
    assert_eq!(kinds, [TimelineKind::Detected, TimelineKind::Alerted]);
    assert_eq!(r.notes.titles(), ["Office printer is offline"]);

    r.run_until(1300).await;
    let alert = r.alert(&key).await.unwrap();
    assert_eq!(alert.state, AlertState::Resolved);
    let incident = r
        .store
        .incident(r.network, alert.incident_id)
        .await
        .unwrap()
        .unwrap();
    assert!(incident.resolved_at.is_some());
    assert_eq!(
        incident.timeline.last().map(|e| e.kind),
        Some(TimelineKind::Recovered)
    );
    assert_eq!(r.notes.titles().len(), 1);
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn a_sleeping_phone_never_alerts_in_24_hours() {
    let r = rig("sleeping-phone").await;
    let phone = r.star("192.168.1.57").await;
    let m = r.monitor();
    m.start();
    r.run_until(24 * HOUR + MIN).await;
    assert_eq!(r.alerts().await, vec![]);
    let d = r.store.get_device(r.network, phone).await.unwrap().unwrap();
    assert!(d.online);
    assert!(r.notes.titles().is_empty());
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn router_outage_suppresses_device_alerts_then_unsuppresses_the_one_still_down() {
    let r = rig("router-outage").await;
    let printer = r.star_named("192.168.1.40", Some("Office printer")).await;
    let m = r.monitor();
    m.start();
    let printer_key = format!("starred_offline:{}", printer.0);

    r.run_until(700).await;
    let router = r.alert("router_down").await.expect("router down alert");
    assert_eq!(router.state, AlertState::Open);

    r.run_until(870).await;
    let p = r.alert(&printer_key).await.expect("printer alert");
    assert!(p.suppressed);
    assert_eq!(p.incident_id, router.incident_id);
    assert!(!r
        .notes
        .titles()
        .contains(&"Office printer is offline".to_string()));

    r.run_until(1000).await;
    let router = r.alert("router_down").await.unwrap();
    assert_eq!(router.state, AlertState::Resolved);
    let p = r.alert(&printer_key).await.unwrap();
    assert_eq!(p.state, AlertState::Open);
    assert!(!p.suppressed);
    assert_ne!(p.incident_id, router.incident_id);
    let printer_notes = r
        .notes
        .titles()
        .into_iter()
        .filter(|t| t == "Office printer is offline")
        .count();
    assert_eq!(printer_notes, 1);

    r.run_until(1600).await;
    let p = r.alert(&printer_key).await.unwrap();
    assert_eq!(p.state, AlertState::Resolved);
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn a_silent_router_raises_nothing() {
    let r = rig("router-silent").await;
    let m = r.monitor();
    m.start();
    r.run_until(HOUR).await;
    assert_eq!(r.alerts().await, vec![]);
    let gw = r
        .store
        .metrics(
            r.network,
            Target::Gateway,
            Metric::RttMs,
            r.epoch,
            r.clock.now_ms(),
        )
        .await
        .unwrap();
    assert!(gw.points.is_empty(), "{:?}", gw.points.len());
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn blocked_icmp_is_not_internet_down() {
    let r = rig("icmp-blocked").await;
    let m = r.monitor();
    m.start();
    r.run_until(HOUR).await;
    assert_eq!(r.alerts().await, vec![]);
    let net = r
        .store
        .metrics(
            r.network,
            Target::Internet,
            Metric::RttMs,
            r.epoch,
            r.clock.now_ms(),
        )
        .await
        .unwrap();
    assert!(net.points.len() > 100);
    assert!(net.points.iter().all(|p| p.avg.is_some()));
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn a_device_joining_later_raises_one_new_device_alert() {
    let r = rig("new-device").await;
    let m = r.monitor();
    m.start();
    r.run_until(590).await;
    assert_eq!(r.alerts().await, vec![]);

    r.run_until(900).await;
    let alerts: Vec<_> = r
        .alerts()
        .await
        .into_iter()
        .filter(|a| a.key.starts_with("new_device:"))
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert!(
        alerts[0].title.starts_with("New device: 192.168.1.77 — "),
        "{}",
        alerts[0].title
    );
    assert!(alerts[0].title.contains(", "));
    assert_eq!(r.notes.titles(), [alerts[0].title.clone()]);
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn the_first_scan_raises_no_new_device_alerts() {
    let r = rig("home").await;
    let m = r.monitor();
    m.start();
    r.run_until(10 * MIN).await;
    assert!(!r
        .alerts()
        .await
        .iter()
        .any(|a| a.key.starts_with("new_device:")));
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn a_wall_clock_jump_ends_the_session_as_sleep_without_offline_alerts() {
    let r = rig("printer-offline").await;
    r.star("192.168.1.40").await;
    let m = r.monitor();
    m.start();
    r.run_until(300).await;
    r.clock.jump(8 * HOUR_MS);
    r.run_until(420).await;

    let sessions = r.sessions().await;
    assert_eq!(sessions.len(), 2, "{sessions:?}");
    assert_eq!(sessions[0].end_reason, Some(EndReason::Sleep));
    assert!(sessions[0].ended_at.unwrap() < r.epoch + 300_000);
    assert!(sessions[1].started_at > r.epoch + 8 * HOUR_MS);
    assert_eq!(sessions[1].ended_at, None);
    assert_eq!(r.alerts().await, vec![]);
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn leaving_the_network_ends_the_session() {
    let mut r = rig("home").await;
    let m = r.monitor();
    m.start();
    r.run_until(2 * MIN).await;
    r.drain();
    r.platform.connected.store(false, Ordering::SeqCst);
    r.run_until(3 * MIN).await;

    let sessions = r.sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].end_reason, Some(EndReason::NetworkLeft));
    let events = r.drain();
    assert!(events.iter().any(|e| matches!(
        e,
        EventKind::MonitorStateChanged {
            state: MonitorState::NotConnected,
            ..
        }
    )));
    assert!(ticks(&events) <= 1);
    assert_eq!(m.status().state, MonitorState::NotConnected);
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn stop_ends_the_session_as_quit() {
    let mut r = rig("home").await;
    let m = r.monitor();
    m.start();
    m.start();
    r.run_until(2 * MIN).await;
    let status = m.status();
    assert_eq!(status.state, MonitorState::Monitoring);
    assert_eq!(status.network_id, Some(r.network));
    assert!(status.last.is_some());

    m.stop(EndReason::Quit).await;
    let sessions = r.sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].end_reason, Some(EndReason::Quit));
    assert!(sessions[0].ended_at.is_some());

    r.drain();
    r.run_until(5 * MIN).await;
    assert_eq!(ticks(&r.drain()), 0);
}

#[tokio::test(start_paused = true)]
async fn open_alerts_resume_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(Db::open(&dir.path().join("nl.db")).unwrap());
    let r = rig_on("printer-offline", store).await;
    let printer = r.star_named("192.168.1.40", Some("Office printer")).await;
    let key = format!("starred_offline:{}", printer.0);

    let m1 = r.monitor();
    m1.start();
    r.run_until(800).await;
    assert_eq!(r.alert(&key).await.unwrap().state, AlertState::Open);
    drop(m1);

    let m2 = r.monitor();
    m2.start();
    r.run_until(1100).await;
    let printer_alerts: Vec<_> = r
        .alerts()
        .await
        .into_iter()
        .filter(|a| a.key == key)
        .collect();
    assert_eq!(printer_alerts.len(), 1, "{printer_alerts:?}");
    assert_eq!(printer_alerts[0].state, AlertState::Open);
    assert_eq!(r.notes.titles().len(), 1);

    r.run_until(1300).await;
    let printer_alerts: Vec<_> = r
        .alerts()
        .await
        .into_iter()
        .filter(|a| a.key == key)
        .collect();
    assert_eq!(printer_alerts.len(), 1);
    assert_eq!(printer_alerts[0].state, AlertState::Resolved);
    m2.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn the_scheduler_never_scans_the_radio() {
    let r = rig("home").await;
    let before = r.platform.wifi_scans.load(Ordering::SeqCst);
    let m = r.monitor();
    m.start();
    r.run_until(HOUR).await;
    assert_eq!(r.platform.wifi_scans.load(Ordering::SeqCst) - before, 0);
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn a_diagnosis_lands_on_the_open_incident() {
    let r = rig("router-outage").await;
    let m = r.monitor();
    m.start();
    r.run_until(700).await;
    let router = r.alert("router_down").await.expect("router down alert");

    let wifi = Arc::new(WifiService::new(
        r.platform.clone(),
        CongestionConfig::default(),
    ));
    let diag = DiagnoseManager::new(
        r.platform.clone(),
        r.sim.clone(),
        wifi,
        Arc::clone(&r.bus),
        DiagnoseConfig::default(),
    );
    let DiagnoseStart::Started(id) = diag.start().await else {
        panic!("diagnosis did not start");
    };
    let run = diag.wait(id).await.unwrap();
    assert!(run.verdict.is_some());
    r.run_until(760).await;

    let incident = r
        .store
        .incident(r.network, router.incident_id)
        .await
        .unwrap()
        .unwrap();
    assert!(incident
        .timeline
        .iter()
        .any(|e| e.kind == TimelineKind::Diagnosed));
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn ticks_publish_samples_and_rollups_happen() {
    let mut r = rig("home").await;
    r.drain();
    let m = r.monitor();
    m.start();
    let start = r.t0.elapsed().as_secs();
    r.run_until(start + 30 * MIN).await;
    let n = ticks(&r.drain());
    assert!((58..=62).contains(&n), "{n} ticks");

    let now = r.clock.now_ms();
    let series = r
        .store
        .metrics(
            r.network,
            Target::Gateway,
            Metric::RttMs,
            now - 7 * 24 * HOUR_MS,
            now,
        )
        .await
        .unwrap();
    assert_eq!(series.resolution, Resolution::FiveMin);
    assert!(!series.points.is_empty());
    m.stop(EndReason::Quit).await;
}

#[tokio::test(start_paused = true)]
async fn a_panic_inside_a_tick_does_not_stop_monitoring() {
    let mut r = rig("home").await;
    let m = r.monitor();
    m.start();
    r.run_until(2 * MIN).await;
    r.drain();
    r.platform.panic_once.store(true, Ordering::SeqCst);
    r.run_until(5 * MIN).await;
    assert!(!r.platform.panic_once.load(Ordering::SeqCst));
    assert!(ticks(&r.drain()) >= 4);
    m.stop(EndReason::Quit).await;
}
