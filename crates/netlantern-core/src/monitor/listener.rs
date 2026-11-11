//! The monitor's reactions outside the tick: applying the engine's
//! decisions (store, `alert.changed`, notifications), and the bus listener
//! for devices that scans find and for finished diagnoses.

use std::sync::Arc;

use tokio::sync::broadcast;

use super::runner::Inner;
use crate::events::{Event, EventKind};
use crate::model::{Device, NetworkId};
use crate::monitor::{conditions, should_notify, Decision, RuleSettings};

impl Inner {
    /// Store the decisions, publish the changed alerts and notify.
    pub(super) async fn apply(
        &self,
        network: NetworkId,
        decisions: Vec<Decision>,
        rules: &RuleSettings,
    ) {
        if decisions.is_empty() {
            return;
        }
        let r = self.store.apply_decisions(network, decisions.clone()).await;
        let Some(alerts) = self.faults.ok("apply decisions", r) else {
            return;
        };
        for alert in &alerts {
            self.bus.publish(EventKind::AlertChanged {
                alert: alert.clone(),
            });
        }
        for d in decisions.iter().filter(|d| should_notify(d, rules)) {
            match d {
                Decision::Open { title, detail, .. } => self.notifier.notify(title, detail),
                Decision::Unsuppress { key, .. } => {
                    let key = key.to_string();
                    if let Some(a) = alerts.iter().rev().find(|a| a.key == key) {
                        self.notifier.notify(&a.title, &a.detail);
                    }
                }
                Decision::Resolve { .. } => {}
            }
        }
    }

    pub(super) async fn listen(self: Arc<Self>, mut rx: broadcast::Receiver<Event>) {
        loop {
            match rx.recv().await {
                Ok(event) => match event.kind {
                    EventKind::ScanDevice { device, .. } if device.is_new => {
                        self.on_new_device(device).await;
                    }
                    EventKind::DiagnoseDone {
                        verdict: Some(v), ..
                    } => self.on_diagnosis(&v.headline).await,
                    _ => {}
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// A scan found a device that is new since the network's first scan.
    async fn on_new_device(&self, device: Device) {
        let mut st = self.state.lock().await;
        let Some(network) = st.session.as_ref().map(|s| s.network) else {
            return;
        };
        let on_network = self.store.get_device(network, device.id).await;
        if !matches!(self.faults.ok("get device", on_network), Some(Some(_))) {
            return;
        }
        let first = self.store.first_scan_finished_at(network).await;
        let Some(first) = self.faults.ok("first scan", first).flatten() else {
            return;
        };
        if device.first_seen <= first {
            return;
        }
        let rules = self.store.rule_settings(network).await;
        let rules = self.faults.ok("rule settings", rules).unwrap_or_default();
        let now = self.clock.now_ms();
        let reading = conditions::new_device_reading(&device, now);
        let decisions = st.engine.evaluate(now, vec![reading], &rules);
        self.apply(network, decisions, &rules).await;
    }

    /// A diagnosis finished: note it on every open incident.
    async fn on_diagnosis(&self, headline: &str) {
        let current = self.scans.current_network().await;
        let Some(Some(network)) = self.faults.ok("current network", current) else {
            return;
        };
        let _st = self.state.lock().await;
        let now = self.clock.now_ms();
        let r = self
            .store
            .add_diagnosis_to_open_incidents(network, headline, now)
            .await;
        for id in self.faults.ok("add diagnosis", r).unwrap_or_default() {
            let incident = self.store.incident(network, id).await;
            if let Some(Some(incident)) = self.faults.ok("incident", incident) {
                self.bus.publish(EventKind::AlertChanged {
                    alert: incident.root,
                });
            }
        }
    }
}
