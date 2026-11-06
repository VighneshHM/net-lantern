//! Everything the routes share: the engine, the store, the event bus, and the
//! security state. Cloning is cheap (every field is an `Arc` or a handle).

use std::sync::Arc;

use netlantern_core::{EventBus, ScanConfig, ScanManager, Store};
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
    pub auth: Arc<Auth>,
    pub policy: Arc<AccessPolicy>,
    pub refusals: Arc<RefusalLog>,
}

impl AppState {
    /// Wires a scan manager to the store and a fresh event bus.
    pub fn new(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        scan: ScanConfig,
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
        Self {
            platform,
            netio,
            store,
            bus,
            scans,
            auth: Arc::new(Auth::new()),
            policy: Arc::new(policy),
            refusals: Arc::default(),
        }
    }
}
