//! Wall-clock time as a trait, so monitor tests can run on tokio's paused clock.

use std::sync::atomic::{AtomicI64, Ordering};
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
#[derive(Debug)]
pub struct TokioClock {
    pub epoch_ms: i64,
    pub start: tokio::time::Instant,
    /// Added by [`TokioClock::jump`].
    offset: AtomicI64,
}

impl TokioClock {
    pub fn new(epoch_ms: i64) -> Self {
        Self {
            epoch_ms,
            start: tokio::time::Instant::now(),
            offset: AtomicI64::new(0),
        }
    }

    /// Move the wall clock forward by `ms` without moving tokio time, which is
    /// what a laptop sleep looks like. For tests.
    pub fn jump(&self, ms: i64) {
        self.offset.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for TokioClock {
    fn now_ms(&self) -> i64 {
        self.epoch_ms + self.offset.load(Ordering::SeqCst) + self.start.elapsed().as_millis() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn jump_moves_only_the_wall_clock() {
        let c = TokioClock::new(1_000);
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        assert_eq!(c.now_ms(), 2_000);
        c.jump(5_000);
        assert_eq!(c.now_ms(), 7_000);
    }
}
