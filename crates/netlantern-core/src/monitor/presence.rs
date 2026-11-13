//! Presence: whether a device counts as online, with a longer grace for
//! devices that sleep.

use std::time::Duration;

use crate::model::DeviceType;
use crate::monitor::MonitorConfig;

/// How long a device may go unseen before it counts as offline.
pub fn grace(device_type: DeviceType, cfg: &MonitorConfig) -> Duration {
    if sleepy(device_type) {
        cfg.offline_grace_sleepy
    } else {
        cfg.offline_grace
    }
}

/// Online when seen within `grace`, counting from the later of `last_seen`
/// and the session start, so time asleep never counts against a device.
pub fn is_online(last_seen: i64, session_started_at: i64, now: i64, grace: Duration) -> bool {
    let grace_ms = i64::try_from(grace.as_millis()).unwrap_or(i64::MAX);
    now.saturating_sub(last_seen.max(session_started_at)) < grace_ms
}

/// Whether the packet-loss rule applies: not to phones and tablets, which
/// drop pings while asleep.
pub fn loss_applies(device_type: DeviceType) -> bool {
    !sleepy(device_type)
}

/// Phones and tablets sleep and drop off the network for minutes at a time.
fn sleepy(device_type: DeviceType) -> bool {
    matches!(device_type, DeviceType::Phone | DeviceType::Tablet)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    #[test]
    fn phones_get_ten_minutes() {
        let cfg = MonitorConfig::default();
        assert_eq!(grace(DeviceType::Phone, &cfg), Duration::from_secs(600));
        assert_eq!(grace(DeviceType::Tablet, &cfg), Duration::from_secs(600));
        assert_eq!(grace(DeviceType::Printer, &cfg), Duration::from_secs(120));
        assert!(!loss_applies(DeviceType::Phone));
        assert!(loss_applies(DeviceType::Printer));
    }

    #[test]
    fn grace_counts_from_the_session_start_after_a_sleep() {
        let now = 1_000 * MIN;
        let last_seen = now - 8 * 60 * MIN;
        let started = now - MIN;
        assert!(is_online(last_seen, started, now, Duration::from_secs(120)));
    }

    #[test]
    fn offline_after_grace() {
        let now = 1_000 * MIN;
        assert!(!is_online(
            now - 121_000,
            now - 500 * MIN,
            now,
            Duration::from_secs(120)
        ));
        assert!(is_online(
            now - 119_000,
            now - 500 * MIN,
            now,
            Duration::from_secs(120)
        ));
    }
}
