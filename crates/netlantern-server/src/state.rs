//! Everything the routes share: the engine, the store, the event bus, and the
//! security state. Cloning is cheap (every field is an `Arc` or a handle).

use std::sync::Arc;

use netlantern_core::{
    Clock, CongestionConfig, DiagnoseConfig, DiagnoseManager, EventBus, Monitor, MonitorConfig,
    NoopNotifier, Notifier, ScanConfig, ScanManager, Store, SystemClock, WifiService,
};
use netlantern_platform::{NetIo, Platform};

use crate::auth::Auth;
use crate::guard::AccessPolicy;
use crate::refusals::RefusalLog;

/// How far a WebSocket client may fall behind before it misses events (and
/// resyncs over REST).
const EVENT_BUFFER: usize = 1024;

/// What the monitor is built from. The default is the real thing: default
/// intervals, no desktop notifications, the system clock.
#[derive(Clone)]
pub struct MonitorParts {
    pub config: MonitorConfig,
    pub notifier: Arc<dyn Notifier>,
    pub clock: Arc<dyn Clock>,
}

impl Default for MonitorParts {
    fn default() -> Self {
        Self {
            config: MonitorConfig::default(),
            notifier: Arc::new(NoopNotifier),
            clock: Arc::new(SystemClock),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub platform: Arc<dyn Platform>,
    pub netio: Arc<dyn NetIo>,
    pub store: Store,
    pub bus: Arc<EventBus>,
    pub scans: Arc<ScanManager>,
    pub wifi: Arc<WifiService>,
    pub diagnose: Arc<DiagnoseManager>,
    /// Built but not started: the host calls `monitor.start()`.
    pub monitor: Arc<Monitor>,
    /// The monitor's clock, for "now" in range queries and acknowledgements.
    pub clock: Arc<dyn Clock>,
    /// The most checks a starred device may have.
    pub max_checks: usize,
    pub auth: Arc<Auth>,
    pub policy: Arc<AccessPolicy>,
    pub refusals: Arc<RefusalLog>,
}

impl AppState {
    /// Wires the scan and diagnosis managers to the store and a fresh event
    /// bus. Diagnosis and the Wi-Fi routes share one radio-scan cache.
    pub fn new(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        scan: ScanConfig,
        diagnose: DiagnoseConfig,
        policy: AccessPolicy,
    ) -> Self {
        Self::with_monitor(
            platform,
            netio,
            store,
            scan,
            diagnose,
            MonitorParts::default(),
            policy,
        )
    }

    /// Like [`AppState::new`], with the monitor's config, notifier and clock
    /// chosen by the caller. The monitor is not started.
    pub fn with_monitor(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        scan: ScanConfig,
        diagnose: DiagnoseConfig,
        monitor: MonitorParts,
        policy: AccessPolicy,
    ) -> Self {
        let bus = Arc::new(EventBus::new(EVENT_BUFFER));
        let scans = Arc::new(ScanManager::new(
            Arc::clone(&platform),
            Arc::clone(&netio),
            Arc::clone(&bus),
            store.clone(),
            scan,
        ));
        let wifi = Arc::new(WifiService::new(
            Arc::clone(&platform),
            CongestionConfig::default(),
        ));
        let diagnose = Arc::new(DiagnoseManager::new(
            Arc::clone(&platform),
            Arc::clone(&netio),
            Arc::clone(&wifi),
            Arc::clone(&bus),
            diagnose,
        ));
        let max_checks = monitor.config.max_checks;
        let clock = Arc::clone(&monitor.clock);
        let monitor = Arc::new(Monitor::new(
            Arc::clone(&platform),
            Arc::clone(&netio),
            store.clone(),
            Arc::clone(&bus),
            Arc::clone(&scans),
            monitor.notifier,
            Arc::clone(&clock),
            monitor.config,
        ));
        Self {
            platform,
            netio,
            store,
            bus,
            scans,
            wifi,
            diagnose,
            monitor,
            clock,
            max_checks,
            auth: Arc::new(Auth::new()),
            policy: Arc::new(policy),
            refusals: Arc::default(),
        }
    }
}
