//! Remembers which refused Host and Origin values were already logged, so a
//! page hammering us with one foreign value writes one log line, not thousands.

use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};

/// Distinct `(kind, value)` pairs remembered before logging stops.
const MAX_REMEMBERED: usize = 256;

#[derive(Default)]
pub struct RefusalLog {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    seen: HashSet<(&'static str, String)>,
    full: bool,
}

impl RefusalLog {
    /// True the first time this pair is seen and while there is room to
    /// remember it. Once full, logs one notice, then answers false forever.
    pub fn first_time(&self, kind: &'static str, value: &str) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (kind, truncate(value, 100));
        if state.seen.contains(&key) {
            return false;
        }
        if state.seen.len() >= MAX_REMEMBERED {
            if !state.full {
                state.full = true;
                tracing::warn!("further refusals are not logged");
            }
            return false;
        }
        state.seen.insert(key);
        true
    }
}

/// At most `max` characters, never splitting one.
pub fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_each_pair_once_and_stops_remembering_at_the_cap() {
        let log = RefusalLog::default();
        assert!(log.first_time("host", "evil:1"));
        assert!(!log.first_time("host", "evil:1"));
        assert!(log.first_time("origin", "evil:1"));
        for i in 0..MAX_REMEMBERED - 2 {
            assert!(log.first_time("host", &format!("h{i}")));
        }
        assert!(!log.first_time("host", "one-too-many"));
        assert!(!log.first_time("host", "one-too-many"));
        assert!(!log.first_time("host", "evil:1"));
    }

    #[test]
    fn long_values_truncate_on_character_boundaries() {
        let long = "é".repeat(300);
        assert_eq!(truncate(&long, 100).chars().count(), 100);
        let log = RefusalLog::default();
        assert!(log.first_time("host", &long));
        assert!(!log.first_time("host", &long));
    }
}
