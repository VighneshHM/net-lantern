//! NetBot core: scan data model, sweep targets and the event bus.

mod events;
mod model;
mod targets;

pub use events::{Event, EventBus, EventKind};
pub use model::{DeviceSummary, ScanId, ScanMode, ScanStage, ScanState, API_VERSION};
pub use targets::{scan_targets, MAX_SWEEP_PREFIX};
