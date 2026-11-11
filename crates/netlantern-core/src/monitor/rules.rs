//! The alert engine: debounce, open, suppress behind the router, un-suppress
//! and resolve (spec §6.9). Pure state: no I/O and no clock reads.

use std::collections::HashMap;

use crate::monitor::{AlertKey, Reading, RuleSettings, Severity};

/// What the engine decided; the store applies it.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Open {
        key: AlertKey,
        title: String,
        detail: String,
        severity: Severity,
        suppressed: bool,
        detected_at: i64,
        last_good_at: Option<i64>,
        at: i64,
    },
    Unsuppress {
        key: AlertKey,
        at: i64,
    },
    Resolve {
        key: AlertKey,
        at: i64,
    },
}

/// The alert engine's state: pending counts (memory only), the last ok time
/// per key, and the open keys with their `suppressed` flag.
///
/// - `RouterDown`, `InternetDown` and `LatencyHigh` open after `consecutive`
///   failing readings in a row; every other key opens on its first failing
///   reading, because its loss window or offline grace is already built into
///   the reading.
/// - Open keys are kept in the order they opened (or were resumed), which is
///   the order `Unsuppress` and disabled-rule `Resolve` decisions come out in.
#[derive(Clone, Debug, Default)]
pub struct AlertEngine {
    consecutive: u32,
    pending: HashMap<AlertKey, Pending>,
    last_good: HashMap<AlertKey, i64>,
    open: Vec<(AlertKey, bool)>,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    count: u32,
    detected_at: i64,
}

impl AlertEngine {
    pub fn new(consecutive: u32) -> Self {
        Self {
            consecutive: consecutive.max(1),
            ..Self::default()
        }
    }

    /// Take over alerts that were open in the store before a restart. They
    /// resolve on their next ok reading and are never opened a second time.
    pub fn resume(&mut self, open: Vec<(AlertKey, bool)>) {
        for (key, suppressed) in open {
            self.pending.remove(&key);
            if !self.is_open(&key) {
                self.open.push((key, suppressed));
            }
        }
    }

    /// Drop every pending count, e.g. after the laptop slept. Open keys stay.
    pub fn reset_pending(&mut self) {
        self.pending.clear();
    }

    pub fn is_open(&self, key: &AlertKey) -> bool {
        self.open.iter().any(|(k, _)| k == key)
    }

    /// The open keys, in the order they opened.
    pub fn open_keys(&self) -> impl Iterator<Item = &AlertKey> {
        self.open.iter().map(|(k, _)| k)
    }

    /// How many failing readings in a row `key` needs before it opens.
    fn required(&self, key: &AlertKey) -> u32 {
        match key {
            AlertKey::RouterDown | AlertKey::InternetDown | AlertKey::LatencyHigh => {
                self.consecutive
            }
            _ => 1,
        }
    }

    /// `RouterDown` is open and not suppressed.
    fn router_down(&self) -> bool {
        self.open
            .iter()
            .any(|(k, suppressed)| *k == AlertKey::RouterDown && !*suppressed)
    }

    fn take_open(&mut self, key: &AlertKey) -> bool {
        match self.open.iter().position(|(k, _)| k == key) {
            Some(i) => {
                self.open.remove(i);
                true
            }
            None => false,
        }
    }

    /// Apply one round of readings. Decisions come out in this order:
    /// 1. `Resolve` for each open key with an ok reading, in reading order,
    ///    then for each open key whose rule is off, in the order they opened;
    /// 2. `Unsuppress` for each suppressed key still open, when the router is
    ///    no longer down (it resolved in this call, or a resumed suppressed
    ///    alert outlived it). A suppressed key that recovered was resolved in
    ///    step 1, so what is left was failing, or failing when last seen;
    /// 3. `Open` for each key whose pending count reached its requirement, in
    ///    reading order. A key that depends on the router opens suppressed
    ///    while `RouterDown` is open and unsuppressed, including when it
    ///    opened earlier in this same call.
    ///
    /// Readings for a disabled rule are ignored and clear its pending counts.
    pub fn evaluate(
        &mut self,
        now: i64,
        readings: Vec<Reading>,
        rules: &RuleSettings,
    ) -> Vec<Decision> {
        let readings: Vec<Reading> = readings
            .into_iter()
            .filter(|r| rules.enabled(r.key.rule()))
            .collect();
        self.pending.retain(|k, _| rules.enabled(k.rule()));

        let mut out = Vec::new();

        // 1. Resolves.
        for r in readings.iter().filter(|r| !r.failing) {
            self.last_good.insert(r.key.clone(), r.ts);
            self.pending.remove(&r.key);
            if self.take_open(&r.key) {
                out.push(Decision::Resolve {
                    key: r.key.clone(),
                    at: now,
                });
            }
        }
        let (kept, disabled): (Vec<_>, Vec<_>) = std::mem::take(&mut self.open)
            .into_iter()
            .partition(|(k, _)| rules.enabled(k.rule()));
        self.open = kept;
        for (key, _) in disabled {
            out.push(Decision::Resolve { key, at: now });
        }

        // 2. Un-suppress once the router is back.
        if !self.router_down() {
            for (key, suppressed) in self.open.iter_mut().filter(|(_, s)| *s) {
                *suppressed = false;
                out.push(Decision::Unsuppress {
                    key: key.clone(),
                    at: now,
                });
            }
        }

        // 3. Opens.
        for r in readings.into_iter().filter(|r| r.failing) {
            if self.is_open(&r.key) {
                continue;
            }
            let required = self.required(&r.key);
            let p = self.pending.entry(r.key.clone()).or_insert(Pending {
                count: 0,
                detected_at: r.ts,
            });
            p.count += 1;
            if p.count < required {
                continue;
            }
            let detected_at = p.detected_at;
            self.pending.remove(&r.key);
            let suppressed = r.key.depends_on_router() && self.router_down();
            self.open.push((r.key.clone(), suppressed));
            out.push(Decision::Open {
                severity: r.key.severity(),
                last_good_at: self.last_good.get(&r.key).copied(),
                key: r.key,
                title: r.title,
                detail: r.detail,
                suppressed,
                detected_at,
                at: now,
            });
        }
        out
    }

    /// Stop watching `key` (e.g. its device was unstarred): drop its state
    /// and resolve it if it was open.
    pub fn forget(&mut self, key: &AlertKey, now: i64) -> Option<Decision> {
        self.pending.remove(key);
        self.last_good.remove(key);
        self.take_open(key).then(|| Decision::Resolve {
            key: key.clone(),
            at: now,
        })
    }
}

/// Whether a decision should raise a desktop notification: an unsuppressed
/// `Open` or an `Unsuppress`, when notifications are on.
pub fn should_notify(d: &Decision, rules: &RuleSettings) -> bool {
    rules.notifications
        && match d {
            Decision::Open { suppressed, .. } => !suppressed,
            Decision::Unsuppress { .. } => true,
            Decision::Resolve { .. } => false,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceId;
    use crate::monitor::{Rule, Target};

    fn reading(key: AlertKey, failing: bool, ts: i64) -> Reading {
        Reading {
            key,
            failing,
            ts,
            title: "title".into(),
            detail: "detail".into(),
        }
    }

    fn on() -> RuleSettings {
        RuleSettings::default()
    }

    /// An engine with `RouterDown` open (opened at 60 s).
    fn router_down_engine() -> AlertEngine {
        let mut e = AlertEngine::new(2);
        e.evaluate(30, vec![reading(AlertKey::RouterDown, true, 30)], &on());
        let d = e.evaluate(60, vec![reading(AlertKey::RouterDown, true, 60)], &on());
        assert!(matches!(
            d.as_slice(),
            [Decision::Open {
                key: AlertKey::RouterDown,
                ..
            }]
        ));
        e
    }

    #[test]
    fn one_failure_is_pending_and_recovery_clears_it() {
        let mut e = AlertEngine::new(2);
        assert!(e
            .evaluate(30, vec![reading(AlertKey::RouterDown, true, 30)], &on())
            .is_empty());
        assert!(e
            .evaluate(60, vec![reading(AlertKey::RouterDown, false, 60)], &on())
            .is_empty());
        // The count started over: one more failure is pending again.
        assert!(e
            .evaluate(90, vec![reading(AlertKey::RouterDown, true, 90)], &on())
            .is_empty());
    }

    #[test]
    fn two_failures_open_with_detected_and_last_good() {
        let mut e = AlertEngine::new(2);
        assert!(e
            .evaluate(0, vec![reading(AlertKey::RouterDown, false, 0)], &on())
            .is_empty());
        assert!(e
            .evaluate(30, vec![reading(AlertKey::RouterDown, true, 30)], &on())
            .is_empty());
        let d = e.evaluate(60, vec![reading(AlertKey::RouterDown, true, 60)], &on());
        assert_eq!(
            d,
            vec![Decision::Open {
                key: AlertKey::RouterDown,
                title: "title".into(),
                detail: "detail".into(),
                severity: Severity::Critical,
                suppressed: false,
                detected_at: 30,
                last_good_at: Some(0),
                at: 60,
            }]
        );
        assert!(should_notify(&d[0], &on()));
        // Still failing: nothing new.
        assert!(e
            .evaluate(90, vec![reading(AlertKey::RouterDown, true, 90)], &on())
            .is_empty());
    }

    #[test]
    fn device_alerts_are_suppressed_while_the_router_is_down() {
        let mut e = router_down_engine();
        let key = AlertKey::StarredOffline(DeviceId(3));
        let d = e.evaluate(
            90,
            vec![
                reading(AlertKey::RouterDown, true, 90),
                reading(key.clone(), true, 90),
            ],
            &on(),
        );
        assert_eq!(d.len(), 1);
        match &d[0] {
            Decision::Open {
                key: k, suppressed, ..
            } => {
                assert_eq!(k, &key);
                assert!(*suppressed);
            }
            other => panic!("expected Open, got {other:?}"),
        }
        assert!(!should_notify(&d[0], &on()));

        // Gateway loss and new devices do not depend on the router.
        let d = e.evaluate(
            120,
            vec![
                reading(AlertKey::PacketLoss(Target::Gateway), true, 120),
                reading(AlertKey::NewDevice(DeviceId(9)), true, 120),
            ],
            &on(),
        );
        assert!(d.iter().all(|d| matches!(
            d,
            Decision::Open {
                suppressed: false,
                ..
            }
        )));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn router_recovery_unsuppresses_what_still_fails_and_resolves_what_recovered() {
        let mut e = router_down_engine();
        let a = AlertKey::StarredOffline(DeviceId(1));
        let b = AlertKey::PacketLoss(Target::Device(DeviceId(2)));
        let d = e.evaluate(
            90,
            vec![
                reading(AlertKey::RouterDown, true, 90),
                reading(a.clone(), true, 90),
                reading(b.clone(), true, 90),
            ],
            &on(),
        );
        assert_eq!(d.len(), 2);
        let d = e.evaluate(
            120,
            vec![
                reading(AlertKey::RouterDown, false, 120),
                reading(a.clone(), true, 120),
                reading(b.clone(), false, 120),
            ],
            &on(),
        );
        assert_eq!(
            d,
            vec![
                Decision::Resolve {
                    key: AlertKey::RouterDown,
                    at: 120
                },
                Decision::Resolve {
                    key: b.clone(),
                    at: 120
                },
                Decision::Unsuppress {
                    key: a.clone(),
                    at: 120
                },
            ]
        );
        assert!(should_notify(&d[2], &on()));
        assert!(!should_notify(&d[0], &on()));
        // Unsuppressed now: a later router outage doesn't re-suppress it, and
        // its recovery resolves it.
        let d = e.evaluate(150, vec![reading(a.clone(), false, 150)], &on());
        assert_eq!(d, vec![Decision::Resolve { key: a, at: 150 }]);
    }

    #[test]
    fn a_suppressed_key_with_no_reading_is_unsuppressed_with_the_router() {
        let mut e = router_down_engine();
        let a = AlertKey::StarredOffline(DeviceId(1));
        e.evaluate(90, vec![reading(a.clone(), true, 90)], &on());
        let d = e.evaluate(120, vec![reading(AlertKey::RouterDown, false, 120)], &on());
        assert_eq!(
            d,
            vec![
                Decision::Resolve {
                    key: AlertKey::RouterDown,
                    at: 120
                },
                Decision::Unsuppress { key: a, at: 120 },
            ]
        );
    }

    #[test]
    fn turning_a_rule_off_resolves_its_alerts() {
        let mut e = AlertEngine::new(2);
        e.evaluate(30, vec![reading(AlertKey::LatencyHigh, true, 30)], &on());
        let d = e.evaluate(60, vec![reading(AlertKey::LatencyHigh, true, 60)], &on());
        assert!(matches!(d.as_slice(), [Decision::Open { .. }]));
        let off = RuleSettings {
            latency_high: false,
            ..on()
        };
        let d = e.evaluate(90, vec![reading(AlertKey::LatencyHigh, true, 90)], &off);
        assert_eq!(
            d,
            vec![Decision::Resolve {
                key: AlertKey::LatencyHigh,
                at: 90
            }]
        );
        // Ignored while off.
        assert!(e
            .evaluate(120, vec![reading(AlertKey::LatencyHigh, true, 120)], &off)
            .is_empty());
        assert!(e
            .evaluate(150, vec![reading(AlertKey::LatencyHigh, true, 150)], &off)
            .is_empty());
    }

    #[test]
    fn resumed_alerts_resolve_on_recovery_and_do_not_reopen() {
        let mut e = AlertEngine::new(2);
        let key = AlertKey::StarredOffline(DeviceId(1));
        e.resume(vec![(key.clone(), false)]);
        assert!(e
            .evaluate(30, vec![reading(key.clone(), true, 30)], &on())
            .is_empty());
        assert!(e
            .evaluate(60, vec![reading(key.clone(), true, 60)], &on())
            .is_empty());
        let d = e.evaluate(90, vec![reading(key.clone(), false, 90)], &on());
        assert_eq!(d, vec![Decision::Resolve { key, at: 90 }]);
    }

    #[test]
    fn new_devices_open_once_and_resolve_when_the_flag_clears() {
        let mut e = AlertEngine::new(2);
        let key = AlertKey::NewDevice(DeviceId(5));
        let d = e.evaluate(30, vec![reading(key.clone(), true, 30)], &on());
        assert!(matches!(
            d.as_slice(),
            [Decision::Open {
                severity: Severity::Info,
                ..
            }]
        ));
        assert!(e
            .evaluate(60, vec![reading(key.clone(), true, 60)], &on())
            .is_empty());
        let d = e.evaluate(90, vec![reading(key.clone(), false, 90)], &on());
        assert_eq!(d, vec![Decision::Resolve { key, at: 90 }]);
    }

    #[test]
    fn forgetting_an_open_key_resolves_it() {
        let mut e = AlertEngine::new(2);
        let key = AlertKey::StarredOffline(DeviceId(1));
        e.evaluate(30, vec![reading(key.clone(), true, 30)], &on());
        assert_eq!(
            e.forget(&key, 60),
            Some(Decision::Resolve {
                key: key.clone(),
                at: 60
            })
        );
        assert_eq!(e.forget(&key, 90), None);
    }

    #[test]
    fn notifications_off_means_no_notify() {
        let off = RuleSettings {
            notifications: false,
            ..on()
        };
        let open = Decision::Open {
            key: AlertKey::RouterDown,
            title: String::new(),
            detail: String::new(),
            severity: Severity::Critical,
            suppressed: false,
            detected_at: 0,
            last_good_at: None,
            at: 0,
        };
        let unsuppress = Decision::Unsuppress {
            key: AlertKey::LatencyHigh,
            at: 0,
        };
        assert!(should_notify(&open, &on()));
        assert!(should_notify(&unsuppress, &on()));
        assert!(!should_notify(&open, &off));
        assert!(!should_notify(&unsuppress, &off));
    }

    #[test]
    fn keys_round_trip_as_text() {
        let cases = [
            (AlertKey::RouterDown, "router_down"),
            (AlertKey::InternetDown, "internet_down"),
            (AlertKey::LatencyHigh, "latency_high"),
            (AlertKey::PacketLoss(Target::Gateway), "packet_loss:gateway"),
            (
                AlertKey::PacketLoss(Target::Device(DeviceId(12))),
                "packet_loss:device:12",
            ),
            (AlertKey::StarredOffline(DeviceId(12)), "starred_offline:12"),
            (AlertKey::NewDevice(DeviceId(12)), "new_device:12"),
        ];
        for (key, text) in cases {
            assert_eq!(key.to_string(), text);
            assert_eq!(text.parse::<AlertKey>().unwrap(), key);
            assert_eq!(serde_json::to_value(&key).unwrap(), serde_json::json!(text));
        }
        for bad in [
            "",
            "router",
            "packet_loss:internet",
            "packet_loss:dns",
            "starred_offline:",
            "starred_offline:-1",
            "new_device:x",
        ] {
            assert!(bad.parse::<AlertKey>().is_err(), "{bad:?} parsed");
        }
    }

    #[test]
    fn rules_and_severities() {
        assert_eq!(AlertKey::RouterDown.rule(), Rule::InternetDown);
        assert_eq!(AlertKey::InternetDown.rule(), Rule::InternetDown);
        assert_eq!(AlertKey::LatencyHigh.severity(), Severity::Warning);
        assert_eq!(
            AlertKey::StarredOffline(DeviceId(1)).severity(),
            Severity::Critical
        );
        assert!(!AlertKey::RouterDown.depends_on_router());
        assert!(AlertKey::InternetDown.depends_on_router());
        assert!(AlertKey::PacketLoss(Target::Device(DeviceId(1))).depends_on_router());
        assert!(!AlertKey::PacketLoss(Target::Gateway).depends_on_router());
        assert_eq!(
            serde_json::to_value(Rule::StarredOffline).unwrap(),
            serde_json::json!("starred_offline")
        );
        let all_on = RuleSettings::default();
        assert!(all_on.notifications && all_on.enabled(Rule::NewDevice));
        let off = RuleSettings {
            packet_loss: false,
            ..all_on
        };
        assert!(!off.enabled(Rule::PacketLoss));
    }
}
