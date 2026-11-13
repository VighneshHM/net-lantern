//! Device presence: online/offline as monitoring sees it, with history.

use std::net::Ipv4Addr;

use netlantern_platform::MacAddr;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::devices::{json_err, load};
use super::{enum_text, Store, StoreError};
use crate::identify::classify::{classify, Signals};
use crate::identify::oui;
use crate::model::{Device, DeviceId, NetworkId};

/// What recording a presence sighting did.
#[derive(Clone, Debug, PartialEq)]
pub struct PresenceSighting {
    pub device: Device,
    /// The MAC was new on this network, so a device was created for it.
    pub created: bool,
}

/// Set a device's presence; returns whether it changed. Writes a `presence`
/// row only on a change.
pub(super) fn apply(tx: &Transaction, id: i64, online: bool, ts: i64) -> rusqlite::Result<bool> {
    let want = if online { "online" } else { "offline" };
    let stored: Option<String> =
        tx.query_row("SELECT presence FROM devices WHERE id = ?1", [id], |r| {
            r.get(0)
        })?;
    if stored.as_deref() == Some(want) {
        return Ok(false);
    }
    tx.execute(
        "UPDATE devices SET presence = ?1, presence_changed_at = ?2 WHERE id = ?3",
        params![want, ts, id],
    )?;
    tx.execute(
        "INSERT INTO presence(device_id, ts, online) VALUES(?1, ?2, ?3)",
        params![id, ts, online],
    )?;
    Ok(true)
}

fn set_presence(
    conn: &mut Connection,
    network: NetworkId,
    device: DeviceId,
    online: bool,
    ts: i64,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    let exists = tx
        .query_row(
            "SELECT 1 FROM devices WHERE network_id = ?1 AND id = ?2",
            params![network.0, device.0],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(false);
    }
    let changed = apply(&tx, device.0, online, ts)?;
    tx.commit()?;
    Ok(changed)
}

fn mark_seen(
    conn: &Connection,
    network: NetworkId,
    device: DeviceId,
    ip: Ipv4Addr,
    ts: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE devices SET last_seen = ?1, last_ip = ?2 WHERE network_id = ?3 AND id = ?4",
        params![ts, ip.to_string(), network.0, device.0],
    )?;
    Ok(())
}

fn record(
    conn: &mut Connection,
    network: NetworkId,
    ip: Ipv4Addr,
    mac: MacAddr,
    ts: i64,
) -> Result<PresenceSighting, StoreError> {
    let tx = conn.transaction()?;
    let known: Option<i64> = tx
        .query_row(
            "SELECT d.id FROM device_macs m JOIN devices d ON d.id = m.device_id
             WHERE d.network_id = ?1 AND m.mac = ?2",
            params![network.0, mac.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let (id, created) = match known {
        Some(id) => {
            mark_seen(&tx, network, DeviceId(id), ip, ts)?;
            tx.execute(
                "UPDATE device_macs SET last_seen = ?1 WHERE device_id = ?2 AND mac = ?3",
                params![ts, id, mac.to_string()],
            )?;
            (id, false)
        }
        None => {
            let private = mac.is_locally_administered();
            let signals = Signals {
                vendor: oui::vendor(mac).map(str::to_string),
                private_mac: private,
                ..Signals::default()
            };
            let class = classify(&signals, None);
            let evidence = serde_json::to_string(&signals).map_err(json_err)?;
            tx.execute(
                "INSERT INTO devices(network_id, last_ip, vendor, type, type_confidence, is_new,
                     first_seen, last_seen, evidence)
                 VALUES(?1, ?2, ?3, ?4, ?5, 1, ?6, ?6, ?7)",
                params![
                    network.0,
                    ip.to_string(),
                    signals.vendor,
                    enum_text(&class.device_type),
                    f64::from(class.confidence),
                    ts,
                    evidence
                ],
            )?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO device_macs(device_id, mac, private_mac, first_seen, last_seen)
                 VALUES(?1, ?2, ?3, ?4, ?4)",
                params![id, mac.to_string(), private, ts],
            )?;
            apply(&tx, id, true, ts)?;
            (id, true)
        }
    };
    let device = load(&tx, network, DeviceId(id))?
        .ok_or_else(|| StoreError::Sqlite("device vanished while recording".into()))?;
    tx.commit()?;
    Ok(PresenceSighting { device, created })
}

impl Store {
    /// Set presence; `true` when it changed (and a history row was written).
    pub async fn set_presence(
        &self,
        network: NetworkId,
        device: DeviceId,
        online: bool,
        ts: i64,
    ) -> Result<bool, StoreError> {
        self.db
            .call(move |c| set_presence(c, network, device, online, ts))
            .await
    }

    /// The device answered at `ip`: refresh `last_seen` and `last_ip`.
    pub async fn mark_seen(
        &self,
        network: NetworkId,
        device: DeviceId,
        ip: Ipv4Addr,
        ts: i64,
    ) -> Result<(), StoreError> {
        self.db
            .call(move |c| mark_seen(c, network, device, ip, ts))
            .await
    }

    /// An ARP answer from `mac` at `ip`: refresh the device that owns the MAC,
    /// or create a new (online, `is_new`) device for it.
    pub async fn record_presence_sighting(
        &self,
        network: NetworkId,
        ip: Ipv4Addr,
        mac: MacAddr,
        ts: i64,
    ) -> Result<PresenceSighting, StoreError> {
        self.db
            .call_store(move |c| record(c, network, ip, mac, ts))
            .await
    }
}
