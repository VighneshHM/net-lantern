//! Everything the routes share: the engine, the store, the event bus, and the
//! security state. Cloning is cheap (every field is an `Arc` or a handle).

use std::sync::Arc;

use netlantern_core::{
    Clock, CongestionConfig, DiagnoseConfig, DiagnoseManager, EventBus, Monitor, MonitorConfig,
    NoopNotifier, Notifier, ScanConfig, ScanManager, Store, SystemClock, Updates, WifiService,
};
use netlantern_platform::{NetIo, Platform};
use serde::Serialize;

use crate::auth::Auth;
use crate::guard::AccessPolicy;
use crate::phone::PhoneAccess;
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

/// Which program hosts the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    /// `netlantern.exe`, with the dashboard in the browser.
    Exe,
    /// The desktop app, with the dashboard in its own window.
    Desktop,
}

/// Start at login, as the host provides it. Only the desktop app has one.
pub trait Autostart: Send + Sync {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
}

/// What the routes may tell about the host: its kind, the version, and its
/// start-at-login switch if it has one.
#[derive(Clone)]
pub struct HostInfo {
    pub kind: HostKind,
    pub version: &'static str,
    pub autostart: Option<Arc<dyn Autostart>>,
}

impl Default for HostInfo {
    /// The exe, this version, no start at login.
    fn default() -> Self {
        Self {
            kind: HostKind::Exe,
            version: env!("CARGO_PKG_VERSION"),
            autostart: None,
        }
    }
}

/// Where the debug info gets the host's recent log lines. The server does not
/// know about the logger: the host passes in closures over it.
#[derive(Clone)]
pub struct LogTaps {
    /// The latest log lines (INFO and above), newest last.
    pub lines: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    /// The latest problems (WARN and above), newest last.
    pub problems: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
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
    /// Phone access, once the host has built it from this state (see
    /// [`PhoneAccess::attach_router`]). `None`: the phone routes answer 503
    /// and the guard accepts only the loopback names.
    pub phone: Option<Arc<PhoneAccess>>,
    /// The program hosting the server; the exe unless the host says otherwise.
    pub host: HostInfo,
    /// The host's recent log lines for the debug info; `None` shows none.
    pub logs: Option<LogTaps>,
    /// The opt-in update check. Built but not started: the host calls
    /// `updates.start()`.
    pub updates: Arc<Updates>,
    /// Held for the whole of "Delete all data", so two at once run one
    /// after the other (the second finds an empty store).
    pub reset: Arc<tokio::sync::RwLock<()>>,
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
        let updates = Updates::new(
            store.clone(),
            Arc::clone(&netio),
            Arc::clone(&bus),
            Arc::clone(&monitor.clock),
            HostInfo::default().version,
        );
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
            phone: None,
            host: HostInfo::default(),
            logs: None,
            updates,
            reset: Arc::default(),
        }
    }
}
