//! Net Lantern core: scan data model, sweep targets, event bus, and the quick scan.

pub mod discover;
mod error;
mod events;
pub mod identify;
mod model;
mod scan;
mod store;
mod targets;
mod wifi;

pub use error::CoreError;
pub use events::{Event, EventBus, EventKind};
pub use identify::classify::{classify, Classification, Signals};
pub use identify::names::{announced_name, clean_name, looks_generated};
pub use identify::upnp::{parse_upnp_description, UpnpDescription};
pub use model::{
    Device, DeviceId, DeviceType, NetworkId, ScanId, ScanMode, ScanSnapshot, ScanStage, ScanState,
    Service, Sighting, API_VERSION,
};
pub use scan::{ScanConfig, ScanManager, StartOutcome};
pub use store::{Db, DevicePatch, Store, StoreError};
pub use targets::{scan_targets, MAX_SWEEP_PREFIX};
pub use wifi::{
    analyze_congestion, ChannelLoad, Congestion, CongestionConfig, CurrentLoad, NearbyScan,
    WifiService, WifiStatus,
};
