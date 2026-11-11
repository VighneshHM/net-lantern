//! Net Lantern core: scan data model, sweep targets, event bus, and the quick scan.

mod diagnose;
pub mod discover;
mod error;
mod events;
pub mod identify;
mod model;
pub mod monitor;
mod scan;
mod store;
mod targets;
mod wifi;

pub use diagnose::{
    classify_portal, explanation, fix, gather_dns, gather_internet, gather_link, gather_router,
    gather_website, headline, judge_dns, judge_internet, judge_link, judge_router, judge_website,
    skipped, verdict, DiagnoseConfig, DiagnoseManager, DiagnoseStart, DiagnosisId, DiagnosisRun,
    DnsLookup, DnsProbe, Finding, InternetProbe, Layer, LayerResult, LayerStatus, LinkProbe,
    PingStats, PortalCheck, RouterProbe, RunState, TcpCheck, Verdict, WebsiteOutcome, WebsiteProbe,
    WifiReading,
};
pub use error::CoreError;
pub use events::{Event, EventBus, EventKind};
pub use identify::classify::{classify, Classification, Signals};
pub use identify::names::{announced_name, clean_name, looks_generated};
pub use identify::upnp::{parse_upnp_description, UpnpDescription};
pub use model::{
    Device, DeviceId, DeviceType, NetworkId, ScanId, ScanMode, ScanSnapshot, ScanStage, ScanState,
    Service, Sighting, API_VERSION,
};
pub use monitor::{
    should_notify, AlertEngine, AlertKey, Check, CheckOutcome, Clock, Decision, DeviceTick,
    EndReason, LossWindow, Metric, MetricPoint, MetricSeries, MonitorConfig, Reading, Resolution,
    Retention, Rule, RuleSettings, Sample, SessionId, SessionSpan, Severity, SystemClock, Target,
    TickResult, TokioClock,
};
pub use scan::{ScanConfig, ScanManager, StartOutcome};
pub use store::{Db, DevicePatch, PresenceSighting, Store, StoreError};
pub use targets::{scan_targets, MAX_SWEEP_PREFIX};
pub use wifi::{
    analyze_congestion, ChannelLoad, Congestion, CongestionConfig, CurrentLoad, NearbyScan,
    WifiService, WifiStatus,
};
