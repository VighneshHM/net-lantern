//! Presence as the monitor keeps it: marking devices seen, flipping them
//! online or offline, and the light presence refresh every few minutes.

use std::net::Ipv4Addr;

use netlantern_platform::{Interface, MacAddr, NeighborState, Platform};

use super::sampler::{self, Faults};
use crate::events::{EventBus, EventKind};
use crate::model::{Device, DeviceId, NetworkId};
use crate::monitor::{conditions, presence, MonitorConfig, Reading, RuleSettings};
use crate::store::Store;

/// What every presence step needs.
pub(crate) struct Ctx<'a> {
    pub store: &'a Store,
    pub bus: &'a EventBus,
    pub faults: &'a Faults,
    pub cfg: &'a MonitorConfig,
    pub network: NetworkId,
    pub session_started_at: i64,
    pub now: i64,
}

/// The time grace counts from: the later of `last_seen` and the session
/// start for a device that is online, so time asleep never counts against
/// it. A device already offline has to be seen again to come back, so a
/// restart or a wake-up does not "recover" it unseen.
pub(crate) fn grace_from(d: &Device, session_started_at: i64) -> i64 {
    if d.online {
        session_started_at
    } else {
        i64::MIN
    }
}

/// Whether `d` counts as online now.
pub(crate) fn online_now(
    d: &Device,
    session_started_at: i64,
    now: i64,
    cfg: &MonitorConfig,
) -> bool {
    presence::is_online(
        d.last_seen,
        grace_from(d, session_started_at),
        now,
        presence::grace(d.device_type, cfg),
    )
}

/// Record that each `(id, ip)` answered just now.
pub(crate) async fn mark_seen(
    ctx: &Ctx<'_>,
    devices: &mut [Device],
    seen: &[(DeviceId, Ipv4Addr)],
) {
    for (id, ip) in seen {
        let r = ctx.store.mark_seen(ctx.network, *id, *ip, ctx.now).await;
        ctx.faults.ok("mark seen", r);
        if let Some(d) = devices.iter_mut().find(|d| d.id == *id) {
            d.last_seen = ctx.now;
        }
    }
}

/// Recompute presence for each device (only starred ones when
/// `starred_only`; this laptop never), store the
/// changes and publish `device.presence` for each.
pub(crate) async fn update_presence(ctx: &Ctx<'_>, devices: &mut [Device], starred_only: bool) {
    for d in devices.iter_mut() {
        if d.is_self || (starred_only && !d.starred) {
            continue;
        }
        let online = online_now(d, ctx.session_started_at, ctx.now, ctx.cfg);
        if online == d.online {
            continue;
        }
        let r = ctx
            .store
            .set_presence(ctx.network, d.id, online, ctx.now)
            .await;
        if ctx.faults.ok("set presence", r) == Some(true) {
            ctx.bus.publish(EventKind::DevicePresence {
                device_id: d.id,
                online,
                at: ctx.now,
            });
        }
        d.online = online;
    }
}

fn in_subnet(ip: Ipv4Addr, iface: &Interface) -> bool {
    let prefix = u32::from(iface.prefix.min(32));
    let mask = u32::MAX.checked_shl(32 - prefix).unwrap_or(0);
    u32::from(ip) & mask == u32::from(iface.ip) & mask
}

/// The presence refresh: ping (then ARP) every other device with an address
/// that was seen within `sweep_max_unseen`,
/// read the ARP table for devices that are new or moved, and update presence
/// for everyone. Returns a `NewDevice` reading for each device created after
/// the network's first scan, when that rule is on.
pub(crate) async fn refresh(
    ctx: &Ctx<'_>,
    platform: &dyn Platform,
    iface: &Interface,
    devices: &mut [Device],
    rules: &RuleSettings,
    first_scan_finished_at: Option<i64>,
) -> Vec<Reading> {
    let max_unseen = i64::try_from(ctx.cfg.sweep_max_unseen.as_millis()).unwrap_or(i64::MAX);
    let unseen_cutoff = ctx.now.saturating_sub(max_unseen);
    let targets: Vec<(DeviceId, Ipv4Addr)> = devices
        .iter()
        .filter(|d| !d.starred && !d.is_self && d.last_seen >= unseen_cutoff)
        .filter_map(|d| d.ip.map(|ip| (d.id, ip)))
        .collect();
    let seen = sampler::sweep(platform, ctx.cfg, ctx.faults, targets).await;
    mark_seen(ctx, devices, &seen).await;

    let mut readings = Vec::new();
    let table = ctx.faults.ok("arp table", platform.arp_table().await);
    for entry in table.unwrap_or_default() {
        if entry.state != NeighborState::Reachable
            || !in_subnet(entry.ip, iface)
            || entry.ip == iface.gateway
            || entry.ip == iface.ip
            || entry.mac == MacAddr([0; 6])
        {
            continue;
        }
        let r = ctx
            .store
            .record_presence_sighting(ctx.network, entry.ip, entry.mac, ctx.now)
            .await;
        let Some(sighting) = ctx.faults.ok("record presence sighting", r) else {
            continue;
        };
        let device = sighting.device;
        if !sighting.created {
            if let Some(d) = devices.iter_mut().find(|d| d.id == device.id) {
                d.last_seen = ctx.now;
            }
            continue;
        }
        ctx.bus.publish(EventKind::DevicePresence {
            device_id: device.id,
            online: true,
            at: ctx.now,
        });
        if rules.new_device && first_scan_finished_at.is_some_and(|t| t < device.first_seen) {
            readings.push(conditions::new_device_reading(&device, ctx.now));
        }
    }
    update_presence(ctx, devices, false).await;
    readings
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlantern_platform::InterfaceKind;

    #[test]
    fn subnet_membership() {
        let iface = Interface {
            ip: "192.168.1.23".parse().unwrap(),
            prefix: 24,
            gateway: "192.168.1.1".parse().unwrap(),
            dns_servers: vec![],
            kind: InterfaceKind::Wifi,
            mac: MacAddr([1, 2, 3, 4, 5, 6]),
        };
        assert!(in_subnet("192.168.1.77".parse().unwrap(), &iface));
        assert!(!in_subnet("192.168.2.77".parse().unwrap(), &iface));
    }
}
