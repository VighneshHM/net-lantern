//! Everything the routes share: the engine, the event bus, and the security
//! state. Cloning is cheap (every field is an `Arc`).

use std::sync::Arc;

use netbot_core::{DeviceStore, EventBus, ScanConfig, ScanManager};
use netbot_platform::Platform;

use crate::auth::Auth;
use crate::guard::AccessPolicy;

/// How far a WebSocket client may fall behind before it misses events (and
/// resyncs over REST).
const EVENT_BUFFER: usize = 1024;

#[derive(Clone)]
pub struct AppState {
    pub platform: Arc<dyn Platform>,
    pub bus: Arc<EventBus>,
    pub scans: Arc<ScanManager>,
    pub devices: Arc<DeviceStore>,
    pub auth: Arc<Auth>,
    pub policy: Arc<AccessPolicy>,
}

impl AppState {
    /// Wires a scan manager to a fresh event bus and device store.
    pub fn new(platform: Arc<dyn Platform>, scan: ScanConfig, policy: AccessPolicy) -> Self {
        let bus = Arc::new(EventBus::new(EVENT_BUFFER));
        let devices = Arc::new(DeviceStore::default());
        let scans = Arc::new(ScanManager::new(
            Arc::clone(&platform),
            Arc::clone(&bus),
            Arc::clone(&devices),
            scan,
        ));
        Self {
            platform,
            bus,
            scans,
            devices,
            auth: Arc::new(Auth::new()),
            policy: Arc::new(policy),
        }
    }
}
