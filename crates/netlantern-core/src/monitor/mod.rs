//! Monitoring: shared types for samples, sessions, checks, configuration and the
//! clock, the pure decision logic (conditions, presence, the alert engine),
//! and the [`Monitor`] task that runs it all on a schedule.

mod clock;
pub mod conditions;
mod config;
pub mod copy;
mod listener;
mod model;
mod notify;
pub mod presence;
mod readings;
mod refresh;
pub mod rules;
mod runner;
mod sampler;

pub use clock::{Clock, SystemClock, TokioClock};
pub use conditions::{DeviceTick, LossWindow, TickResult};
pub use config::{MonitorConfig, Retention};
pub use model::{
    AckOutcome, Alert, AlertFilter, AlertId, AlertKey, AlertState, Check, CheckOutcome, EndReason,
    Incident, IncidentId, Metric, MetricPoint, MetricSeries, Reading, Resolution, Rule,
    RuleSettings, Sample, SessionId, SessionSpan, Severity, Target, TimelineEvent, TimelineKind,
};
pub use model::{DeviceChecks, MonitorState, MonitorStatus, MonitorTick};
pub use notify::{NoopNotifier, Notifier};
pub use rules::{should_notify, AlertEngine, Decision};
pub use runner::Monitor;
