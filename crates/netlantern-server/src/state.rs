//! Everything the routes share: the engine, the store, the event bus, and the
//! security state. Cloning is cheap (every field is an `Arc` or a handle).

use std::sync::Arc;

use netlantern_core::{
    CongestionConfig, DiagnoseConfig, DiagnoseManager, EventBus, ScanConfig, ScanManager, Store,
    WifiService,
};
use netlantern_platform::{NetIo, Platform};

use crate::auth::Auth;
use crate::guard::AccessPolicy;
use crate::refusals::RefusalLog;

/// How far a WebSocket client may fall behind before it misses events (and
/// resyncs over REST).
const EVENT_BUFFER: usize = 1024;

#[derive(Clone)]
pub struct AppState {
    pub platform: Arc<dyn Platform>,
    pub netio: Arc<dyn NetIo>,
    pub store: Store,
    pub bus: Arc<EventBus>,
    pub scans: Arc<ScanManager>,
    pub wifi: Arc<WifiService>,
    pub diagnose: Arc<DiagnoseManager>,
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
        Self {
            platform,
            netio,
            store,
            bus,
            scans,
            wifi,
            diagnose,
            auth: Arc::new(Auth::new()),
            policy: Arc::new(policy),
            refusals: Arc::default(),
        }
    }
}
