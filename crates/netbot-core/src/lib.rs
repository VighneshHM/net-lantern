//! NetBot core: scan data model, sweep targets, event bus, and the quick scan.

mod devices;
mod error;
mod events;
mod model;
mod scan;
mod targets;

pub use devices::DeviceStore;
pub use error::CoreError;
pub use events::{Event, EventBus, EventKind};
pub use model::{DeviceSummary, ScanId, ScanMode, ScanStage, ScanState, API_VERSION};
pub use scan::{ScanConfig, ScanManager, ScanSnapshot, StartOutcome};
pub use targets::{scan_targets, MAX_SWEEP_PREFIX};
