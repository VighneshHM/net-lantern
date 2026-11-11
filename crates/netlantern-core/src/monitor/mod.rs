//! Monitoring: shared types for samples, sessions, checks, configuration and the
//! clock, plus the pure decision logic (conditions, presence, the alert engine).

mod clock;
pub mod conditions;
mod config;
pub mod copy;
mod model;
pub mod presence;
pub mod rules;

pub use clock::{Clock, SystemClock, TokioClock};
pub use conditions::{DeviceTick, LossWindow, TickResult};
pub use config::{MonitorConfig, Retention};
pub use model::{
    AlertKey, Check, CheckOutcome, EndReason, Metric, MetricPoint, MetricSeries, Reading,
    Resolution, Rule, RuleSettings, Sample, SessionId, SessionSpan, Severity, Target,
};
pub use rules::{should_notify, AlertEngine, Decision};
