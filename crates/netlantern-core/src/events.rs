use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::diagnose::{DiagnosisId, LayerResult, RunState, Verdict};
use crate::model::{Device, ScanId, ScanStage, ScanState, API_VERSION};

/// A sequenced message streamed to subscribers.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub seq: u64,
    pub api_version: u32,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum EventKind {
    #[serde(rename = "scan.progress")]
    ScanProgress {
        scan_id: ScanId,
        done: u32,
        total: u32,
        stage: ScanStage,
    },
    #[serde(rename = "scan.device")]
    ScanDevice {
        scan_id: ScanId,
        /// The device as stored after this sighting.
        device: Device,
    },
    #[serde(rename = "scan.done")]
    ScanDone {
        scan_id: ScanId,
        state: ScanState,
        /// Why the scan failed, when it did.
        error: Option<String>,
        isolated: bool,
        devices_found: u32,
    },
    /// One layer of a diagnosis has been judged (or skipped).
    #[serde(rename = "diagnose.step")]
    DiagnoseStep {
        run_id: DiagnosisId,
        step: LayerResult,
    },
    /// A diagnosis has finished; always the run's last event.
    #[serde(rename = "diagnose.done")]
    DiagnoseDone {
        run_id: DiagnosisId,
        state: RunState,
        /// `None` when the run failed.
        verdict: Option<Verdict>,
        /// Why the run failed, when it did.
        error: Option<String>,
    },
}

/// Fan-out bus that stamps every event with a gap-free sequence number.
///
/// Sequence numbers start at 1. The sequence is advanced and the event is
/// sent while holding one lock, so the delivery order seen by any subscriber
/// always equals sequence order, even with several concurrent publishers.
/// Consumers treat a skipped sequence number as a gap and resync.
///
/// To avoid gaps when attaching a consumer, call [`EventBus::subscribe`]
/// first and read [`EventBus::last_seq`] afterwards.
pub struct EventBus {
    tx: broadcast::Sender<Event>,
    publish_lock: Mutex<()>,
    seq: AtomicU64,
}

impl EventBus {
    /// Creates a bus whose subscribers may lag by up to `capacity` events.
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            publish_lock: Mutex::new(()),
            seq: AtomicU64::new(0),
        }
    }

    /// Stamps `kind` with the next sequence number and broadcasts it.
    pub fn publish(&self, kind: EventKind) -> Event {
        // The guarded data is `()`, so a poisoned lock holds no broken state.
        let _guard = self.publish_lock.lock().unwrap_or_else(|e| e.into_inner());
        let seq = self.seq.load(Ordering::SeqCst) + 1;
        self.seq.store(seq, Ordering::SeqCst);
        let event = Event {
            seq,
            api_version: API_VERSION,
            kind,
        };
        // An error only means there are no receivers right now.
        let _ = self.tx.send(event.clone());
        event
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Sequence number of the most recently published event (0 if none).
    pub fn last_seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ScanId, ScanState};

    fn done(id: u64) -> EventKind {
        EventKind::ScanDone {
            scan_id: ScanId(id),
            state: ScanState::Done,
            error: None,
            isolated: false,
            devices_found: 13,
        }
    }

    #[test]
    fn seq_increments_from_one() {
        let bus = EventBus::new(16);
        let a = bus.publish(done(1));
        let b = bus.publish(done(1));
        assert_eq!((a.seq, b.seq, bus.last_seq()), (1, 2, 2));
    }

    #[test]
    fn event_json_shape() {
        let bus = EventBus::new(16);
        let v = serde_json::to_value(bus.publish(done(7))).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"seq":1,"api_version":1,"type":"scan.done",
                "data":{"scan_id":7,"state":"done","error":null,"isolated":false,"devices_found":13}})
        );
    }

    #[test]
    fn concurrent_publishers_deliver_in_seq_order() {
        use std::sync::Arc;
        let bus = Arc::new(EventBus::new(2048));
        let mut rx = bus.subscribe();
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let bus = Arc::clone(&bus);
                std::thread::spawn(move || {
                    for _ in 0..500 {
                        bus.publish(done(1));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let mut seqs = Vec::new();
        for _ in 0..1000 {
            seqs.push(rx.try_recv().unwrap().seq);
        }
        assert_eq!(seqs, (1..=1000).collect::<Vec<u64>>());
        assert_eq!(bus.last_seq(), 1000);
    }

    #[test]
    fn diagnose_event_json_shape() {
        use crate::diagnose::{Finding, LayerStatus};
        let bus = EventBus::new(16);
        let v = serde_json::to_value(bus.publish(EventKind::DiagnoseDone {
            run_id: DiagnosisId(1),
            state: RunState::Done,
            verdict: Some(Verdict {
                finding: Finding::Healthy,
                status: LayerStatus::Pass,
                layer: None,
                headline: "Everything looks fine.".into(),
                fix: None,
            }),
            error: None,
        }))
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"seq":1,"api_version":1,"type":"diagnose.done",
                "data":{"run_id":1,"state":"done","verdict":{"finding":"healthy","status":"pass",
                    "layer":null,"headline":"Everything looks fine.","fix":null},"error":null}})
        );
    }

    #[test]
    fn diagnose_step_json_shape() {
        let bus = EventBus::new(16);
        let v = serde_json::to_value(bus.publish(EventKind::DiagnoseStep {
            run_id: DiagnosisId(2),
            step: crate::diagnose::skipped(crate::diagnose::Layer::Dns),
        }))
        .unwrap();
        assert_eq!(v["type"], "diagnose.step");
        assert_eq!(v["data"]["run_id"], 2);
        assert_eq!(v["data"]["step"]["layer"], "dns");
        assert_eq!(v["data"]["step"]["status"], "skipped");
    }

    #[test]
    fn event_json_round_trips() {
        let bus = EventBus::new(16);
        let event = bus.publish(EventKind::ScanProgress {
            scan_id: ScanId(3),
            done: 10,
            total: 254,
            stage: ScanStage::Sweep,
        });
        let json = serde_json::to_string(&event).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), event);
    }
}
