//! One tick's readings for the alert engine.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;

use super::refresh::grace_from;
use crate::model::{Device, DeviceId};
use crate::monitor::{
    conditions, presence, AlertEngine, AlertKey, Decision, LossWindow, MonitorConfig, Reading,
    Target, TickResult,
};

/// The readings for one tick. The router's comes first: the engine suppresses
/// a device alert only when `RouterDown` opened before it in the same call.
///
/// `devices` holds every device on the network with presence already updated
/// for this tick; `starred_since` the tick at which each device starred while
/// monitoring ran joined the watched set, so its offline grace counts from
/// then at the earliest; `created` holds `NewDevice` readings for devices the
/// presence refresh created this tick.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build(
    t: &TickResult,
    gateway: Ipv4Addr,
    loss: &LossWindow,
    engine: &AlertEngine,
    devices: &[Device],
    session_started_at: i64,
    starred_since: &HashMap<DeviceId, i64>,
    cfg: &MonitorConfig,
    created: Vec<Reading>,
) -> Vec<Reading> {
    let now = t.at;
    let mut out = vec![
        conditions::router_reading(t, gateway),
        conditions::internet_reading(t, cfg),
        conditions::latency_reading(t, cfg),
        conditions::loss_reading(Target::Gateway, loss, "", now, cfg),
    ];
    let starred = || devices.iter().filter(|d| d.starred);
    for d in starred().filter(|d| presence::loss_applies(d.device_type)) {
        out.push(conditions::loss_reading(
            Target::Device(d.id),
            loss,
            &d.display_name,
            now,
            cfg,
        ));
    }
    for d in starred() {
        let since = starred_since.get(&d.id).copied().unwrap_or(i64::MIN);
        out.push(conditions::offline_reading(
            d,
            grace_from(d, session_started_at).max(since),
            now,
            cfg,
        ));
    }
    out.extend(created);
    for key in engine.open_keys() {
        if let AlertKey::NewDevice(id) = key {
            if let Some(d) = devices.iter().find(|d| d.id == *id) {
                out.push(conditions::new_device_reading(d, now));
            }
        }
    }
    out
}

/// Stop watching devices that are no longer starred: those starred last tick
/// (`watched`) or with an open device alert. Returns the resolves; `watched`
/// becomes this tick's starred set.
pub(crate) fn forget_unstarred(
    engine: &mut AlertEngine,
    loss: &mut LossWindow,
    watched: &mut HashSet<DeviceId>,
    devices: &[Device],
    now: i64,
) -> Vec<Decision> {
    let starred: HashSet<DeviceId> = devices.iter().filter(|d| d.starred).map(|d| d.id).collect();
    let open_ids = engine.open_keys().filter_map(|k| match k {
        AlertKey::StarredOffline(id) | AlertKey::PacketLoss(Target::Device(id)) => Some(*id),
        _ => None,
    });
    let gone: HashSet<DeviceId> = watched
        .iter()
        .copied()
        .chain(open_ids)
        .filter(|id| !starred.contains(id))
        .collect();
    let mut out = Vec::new();
    for id in gone {
        out.extend(engine.forget(&AlertKey::StarredOffline(id), now));
        out.extend(engine.forget(&AlertKey::PacketLoss(Target::Device(id)), now));
        loss.remove(&Target::Device(id));
    }
    *watched = starred;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceId, DeviceType};
    use crate::monitor::{Decision, RuleSettings};

    fn printer(last_seen: i64) -> Device {
        Device {
            id: DeviceId(7),
            ip: Some("192.168.1.40".parse().unwrap()),
            mac: None,
            private_mac: false,
            vendor: None,
            hostname: None,
            name: Some("Office printer".into()),
            display_name: "Office printer".into(),
            device_type: DeviceType::Printer,
            type_confidence: 1.0,
            starred: true,
            is_new: false,
            online: true,
            answers_ping: true,
            rtt_ms: None,
            is_self: false,
            is_gateway: false,
            first_seen: 0,
            last_seen,
            services: vec![],
            open_ports: vec![],
            checks: vec![],
            presence_changed_at: None,
        }
    }

    fn router_down(at: i64) -> TickResult {
        TickResult {
            at,
            gateway_rtt: None,
            gateway_arp: Some(false),
            ..TickResult::default()
        }
    }

    #[test]
    fn a_device_alert_opening_with_router_down_is_suppressed() {
        let cfg = MonitorConfig::default();
        let gw: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let loss = LossWindow::new(30);
        let mut engine = AlertEngine::new(2);
        let rules = RuleSettings::default();

        // First failing router tick: the printer is still within its grace.
        let mut d = printer(0);
        let r = build(
            &router_down(100_000),
            gw,
            &loss,
            &engine,
            &[d.clone()],
            0,
            &HashMap::new(),
            &cfg,
            vec![],
        );
        assert!(engine.evaluate(100_000, r, &rules).is_empty());

        // Second tick: the router opens and the printer goes offline together.
        d.online = false;
        let r = build(
            &router_down(130_000),
            gw,
            &loss,
            &engine,
            &[d],
            0,
            &HashMap::new(),
            &cfg,
            vec![],
        );
        assert_eq!(r[0].key, AlertKey::RouterDown);
        let decisions = engine.evaluate(130_000, r, &rules);
        let opened: Vec<_> = decisions
            .iter()
            .filter_map(|d| match d {
                Decision::Open {
                    key, suppressed, ..
                } => Some((key.clone(), *suppressed)),
                _ => None,
            })
            .collect();
        assert_eq!(
            opened,
            [
                (AlertKey::RouterDown, false),
                (AlertKey::StarredOffline(DeviceId(7)), true)
            ]
        );
    }

    #[test]
    fn an_offline_device_must_be_seen_to_recover() {
        let cfg = MonitorConfig::default();
        let gw: Ipv4Addr = "192.168.1.1".parse().unwrap();
        let loss = LossWindow::new(30);
        let engine = AlertEngine::new(2);
        let mut d = printer(0);
        d.online = false;
        let t = TickResult {
            at: 1_000_000,
            gateway_rtt: Some(2.0),
            internet_rtt: Some(10.0),
            ..TickResult::default()
        };
        // A session that started just now does not bring it back unseen.
        let r = build(
            &t,
            gw,
            &loss,
            &engine,
            &[d],
            990_000,
            &HashMap::new(),
            &cfg,
            vec![],
        );
        let offline = r
            .iter()
            .find(|r| r.key == AlertKey::StarredOffline(DeviceId(7)))
            .unwrap();
        assert!(offline.failing);
    }
}
