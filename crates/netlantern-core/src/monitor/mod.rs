//! Monitoring: shared types for samples, sessions, checks, configuration and the clock.

mod clock;
mod config;
mod model;

pub use clock::{Clock, SystemClock, TokioClock};
pub use config::{MonitorConfig, Retention};
pub use model::{
    Check, CheckOutcome, EndReason, Metric, MetricPoint, MetricSeries, Resolution, Sample,
    SessionId, SessionSpan, Target,
};
