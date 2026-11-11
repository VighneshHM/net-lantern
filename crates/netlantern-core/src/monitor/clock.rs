//! Wall-clock time as a trait, so monitor tests can run on tokio's paused clock.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// The real clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// A clock that starts at `epoch_ms` and follows tokio's (possibly paused) time.
#[derive(Clone, Copy, Debug)]
pub struct TokioClock {
    pub epoch_ms: i64,
    pub start: tokio::time::Instant,
}

impl TokioClock {
    pub fn new(epoch_ms: i64) -> Self {
        Self {
            epoch_ms,
            start: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now_ms(&self) -> i64 {
        self.epoch_ms + self.start.elapsed().as_millis() as i64
    }
}
