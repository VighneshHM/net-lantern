//! Devices: identity by MAC, evidence merging, and the device read model.

use std::net::Ipv4Addr;

use netlantern_platform::MacAddr;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{enum_parse, enum_text, Store, StoreError};
use crate::identify::classify::{classify, Signals};
use crate::identify::names::announced_name;
use crate::identify::oui;
use crate::model::{Device, DeviceId, DeviceType, NetworkId, ScanId, Service, Sighting};

/// Changes a user can make to a device. `None` leaves a field alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DevicePatch {
    /// `Some(None)` and a blank name both clear the name.
    pub name: Option<Option<String>>,
    pub starred: Option<bool>,
    pub is_new: Option<bool>,
}

fn json_err(e: serde_json::Error) -> StoreError {
    StoreError::Sqlite(format!("stored evidence is unreadable: {e}"))
}

fn push_unique<T: PartialEq>(into: &mut Vec<T>, items: impl IntoIterator<Item = T>) {
    for item in items {
        if !into.contains(&item) {
            into.push(item);
        }
    }
}

/// Fold a new sighting's evidence into what is stored. Lists are unions;
/// NetBIOS and reverse DNS prefer the newest answer; the flags come from now.
fn merge_evidence(stored: Option<Signals>, now: Signals) -> Signals {
    let Some(mut m) = stored else { return now };
    // Newest entries first, so `announced_name` prefers fresh data. An entry
    // is the same service whatever address it was seen at.
    let old_mdns = std::mem::take(&mut m.mdns);
    m.mdns = now.mdns;
    for o in old_mdns {
        if !m
            .mdns
            .iter()
            .any(|n| (&n.service_type, &n.instance) == (&o.service_type, &o.instance))
        {
            m.mdns.push(o);
        }
    }
    let old_ssdp = std::mem::take(&mut m.ssdp);
    m.ssdp = now.ssdp;
    for o in old_ssdp {
        if !m
            .ssdp
            .iter()
            .any(|n| (&n.0.st, &n.0.location) == (&o.0.st, &o.0.location))
        {
            m.ssdp.push(o);
        }
    }
    push_unique(&mut m.open_ports, now.open_ports);
    m.open_ports.sort_unstable();
    m.netbios = now.netbios.or(m.netbios);
    m.rdns = now.rdns.or(m.rdns);
    m.vendor = now.vendor.or(m.vendor);
    m.ttl = now.ttl;
    m.private_mac = now.private_mac;
    m.is_gateway = now.is_gateway;
    m.is_self = now.is_self;
    m
}

/// The `(source, value)` rows shown for a device.
fn service_rows(s: &Signals) -> Vec<Service> {
    let mut out = Vec::new();
    let mut add = |source: &str, value: &str| {
        let value = value.trim();
        if !value.is_empty() {
            push_unique(
                &mut out,
                [Service {
                    source: source.into(),
                    value: value.into(),
                }],
            );
        }
    };
    for m in &s.mdns {
        add("mdns", &m.service_type);
        add("mdns", m.friendly_name.as_deref().unwrap_or(&m.instance));
    }
    for (r, d) in &s.ssdp {
        add("ssdp", &r.st);
        if let Some(d) = d {
            add("ssdp", d.friendly_name.as_deref().unwrap_or(""));
            add("ssdp", d.model_name.as_deref().unwrap_or(""));
        }
    }
    if let Some(n) = &s.netbios {
        add("netbios", &n.name);
    }
    if let Some(r) = &s.rdns {
        add("dns", r);
    }
    out
}

fn parse_mac(s: &str) -> Result<MacAddr, StoreError> {
    s.parse()
        .map_err(|_| StoreError::Sqlite(format!("stored MAC {s:?} is invalid")))
}

fn query_ids(
    tx: &Transaction,
    sql: &str,
    args: impl rusqlite::Params,
) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = tx.prepare_cached(sql)?;
    let rows = stmt.query_map(args, |r| r.get(0))?;
    rows.collect()
}

/// Find the stored device a sighting belongs to, if any (rules 1 to 3).
fn find_match(
    tx: &Transaction,
    network: NetworkId,
    scan: ScanId,
    s: &Sighting,
    private: bool,
    name: Option<&str>,
) -> rusqlite::Result<Option<i64>> {
    let net = network.0;
    if let Some(mac) = s.mac {
        let owner = query_ids(
            tx,
            "SELECT d.id FROM device_macs m JOIN devices d ON d.id = m.device_id
             WHERE d.network_id = ?1 AND m.mac = ?2",
            params![net, mac.to_string()],
        )?;
        if let Some(id) = owner.first() {
            return Ok(Some(*id));
        }
        if private {
            let Some(name) = name else { return Ok(None) };
            //             // Every name match counts toward "exactly one"; only then is the
            // single match rejected if it was already seen in this scan.
            let same_name = query_ids(
                tx,
                "SELECT d.id FROM devices d
                 WHERE d.network_id = ?1 AND lower(d.hostname) = lower(?2)",
                params![net, name],
            )?;
            let [only] = same_name.as_slice() else {
                return Ok(None);
            };
            let seen_here: bool = tx.query_row(
                "SELECT d.last_scan_id IS ?2
                        AND EXISTS (SELECT 1 FROM device_macs m WHERE m.device_id = d.id)
                 FROM devices d WHERE d.id = ?1",
                params![only, scan.0 as i64],
                |r| r.get(0),
            )?;
            return Ok((!seen_here).then_some(*only));
        }
        return Ok(None);
    }
    let by_ip = query_ids(
        tx,
        "SELECT d.id FROM devices d
         WHERE d.network_id = ?1 AND d.last_ip = ?2
           AND NOT EXISTS (SELECT 1 FROM device_macs m WHERE m.device_id = d.id)",
        params![net, s.ip.to_string()],
    )?;
    Ok(by_ip.first().copied())
}

fn record(
    conn: &mut Connection,
    network: NetworkId,
    scan: ScanId,
    s: Sighting,
    now: i64,
) -> Result<Device, StoreError> {
    let tx = conn.transaction()?;
    let private = s.mac.is_some_and(|m| m.is_locally_administered()) || s.signals.private_mac;

    // This sighting's own evidence.
    let mut sig = s.signals.clone();
    sig.vendor = s.mac.and_then(oui::vendor).map(str::to_string);
    sig.private_mac = private;
    sig.is_gateway = s.is_gateway;
    sig.is_self = s.is_self;
    sig.ttl = s.ttl;
    push_unique(&mut sig.open_ports, s.open_ports.iter().copied());

    let id = match find_match(
        &tx,
        network,
        scan,
        &s,
        private,
        announced_name(&sig, s.ip).as_deref(),
    )? {
        Some(id) => id,
        None => {
            tx.execute(
                "INSERT INTO devices(network_id, first_seen, last_seen) VALUES(?1, ?2, ?2)",
                params![network.0, now],
            )?;
            tx.last_insert_rowid()
        }
    };

    if let Some(mac) = s.mac {
        tx.execute(
            "INSERT INTO device_macs(device_id, mac, private_mac, first_seen, last_seen)
             VALUES(?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(device_id, mac) DO UPDATE SET last_seen = excluded.last_seen",
            params![id, mac.to_string(), private, now],
        )?;
    }

    let stored: Option<String> =
        tx.query_row("SELECT evidence FROM devices WHERE id = ?1", [id], |r| {
            r.get(0)
        })?;
    let stored = stored
        .map(|j| serde_json::from_str::<Signals>(&j))
        .transpose()
        .map_err(json_err)?;
    let merged = merge_evidence(stored, sig);
    let hostname = announced_name(&merged, s.ip);
    let class = classify(&merged, hostname.as_deref());
    let evidence = serde_json::to_string(&merged).map_err(json_err)?;

    tx.execute(
        "UPDATE devices SET last_ip = ?1, hostname = ?2, vendor = ?3, type = ?4, type_confidence = ?5,
             answers_ping = ?6, rtt_ms = ?7, ttl = ?8, is_self = ?9, is_gateway = ?10,
             last_scan_id = ?11, last_seen = ?12, evidence = ?13
         WHERE id = ?14",
        params![
            s.ip.to_string(),
            hostname,
            merged.vendor,
            enum_text(&class.device_type),
            f64::from(class.confidence),
            s.answers_ping,
            s.rtt_ms,
            s.ttl,
            s.is_self,
            s.is_gateway,
            scan.0 as i64,
            now,
            evidence,
            id
        ],
    )?;

    for svc in service_rows(&merged) {
        tx.execute(
            "INSERT INTO device_services(device_id, source, value, last_seen) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(device_id, source, value) DO UPDATE SET last_seen = excluded.last_seen",
            params![id, svc.source, svc.value, now],
        )?;
    }
    for port in &merged.open_ports {
        tx.execute(
            "INSERT INTO device_ports(device_id, port, seen_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(device_id, port, proto) DO UPDATE SET seen_at = excluded.seen_at",
            params![id, port, now],
        )?;
    }

    let device = load(&tx, network, DeviceId(id))?
        .ok_or_else(|| StoreError::Sqlite("device vanished while recording".into()))?;
    tx.commit()?;
    Ok(device)
}

/// The id of the network's newest scan in the given state.
fn newest_scan(
    conn: &Connection,
    network: NetworkId,
    state: &str,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT MAX(id) FROM scans WHERE network_id = ?1 AND state = ?2",
        params![network.0, state],
        |r| r.get(0),
    )
}

/// Ids of the network's scans that are still running.
fn query_running(conn: &Connection, network: NetworkId) -> rusqlite::Result<Vec<i64>> {
    conn.prepare_cached("SELECT id FROM scans WHERE network_id = ?1 AND state = 'running'")?
        .query_map([network.0], |r| r.get(0))?
        .collect()
}

const DEVICE_COLUMNS: &str =
    "id, last_ip, hostname, vendor, type, type_confidence, name, starred, is_new,
    answers_ping, rtt_ms, ttl, is_self, is_gateway, last_scan_id, first_seen, last_seen";

fn load(conn: &Connection, network: NetworkId, id: DeviceId) -> Result<Option<Device>, StoreError> {
    let done = newest_scan(conn, network, "done")?;
    let running = query_running(conn, network)?;
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {DEVICE_COLUMNS} FROM devices WHERE network_id = ?1 AND id = ?2"
    ))?;
    let device = stmt
        .query_row(params![network.0, id.0], |r| {
            row_to_device(r, done, &running)
        })
        .optional()?;
    device.map(|d| fill(conn, d)).transpose()
}

type Raw = Result<Device, StoreError>;

fn row_to_device(r: &rusqlite::Row, done: Option<i64>, running: &[i64]) -> rusqlite::Result<Raw> {
    let ip: Option<String> = r.get("last_ip")?;
    let hostname: Option<String> = r.get("hostname")?;
    let name: Option<String> = r.get("name")?;
    let ty: String = r.get("type")?;
    let last_scan: Option<i64> = r.get("last_scan_id")?;
    let ip: Option<Ipv4Addr> = ip.and_then(|s| s.parse().ok());
    let display_name = name
        .clone()
        .or_else(|| hostname.clone())
        .or_else(|| ip.map(|i| i.to_string()))
        .unwrap_or_else(|| "Unknown device".into());
    let online = last_scan.is_some_and(|l| done.is_some_and(|d| l >= d) || running.contains(&l));
    let mut device = Device {
        id: DeviceId(r.get("id")?),
        ip,
        mac: None,
        private_mac: false,
        vendor: r.get("vendor")?,
        hostname,
        name,
        display_name,
        device_type: DeviceType::Unknown,
        type_confidence: r.get::<_, f64>("type_confidence")? as f32,
        starred: r.get("starred")?,
        is_new: r.get("is_new")?,
        online,
        answers_ping: r.get("answers_ping")?,
        rtt_ms: r.get("rtt_ms")?,
        is_self: r.get("is_self")?,
        is_gateway: r.get("is_gateway")?,
        first_seen: r.get("first_seen")?,
        last_seen: r.get("last_seen")?,
        services: Vec::new(),
        open_ports: Vec::new(),
    };
    Ok(enum_parse::<DeviceType>(&ty).map(|t| {
        device.device_type = t;
        device
    }))
}

/// Add the MAC, services and ports to a device read from `devices`.
fn fill(conn: &Connection, raw: Raw) -> Result<Device, StoreError> {
    let mut d = raw?;
    let latest: Option<(String, bool)> = conn
        .query_row(
            "SELECT mac, private_mac FROM device_macs WHERE device_id = ?1
             ORDER BY last_seen DESC, rowid DESC LIMIT 1",
            [d.id.0],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((mac, private)) = latest {
        d.mac = Some(parse_mac(&mac)?);
        d.private_mac = private;
    }
    d.services = conn
        .prepare_cached(
            "SELECT source, value FROM device_services WHERE device_id = ?1 ORDER BY source, value",
        )?
        .query_map([d.id.0], |r| {
            Ok(Service {
                source: r.get(0)?,
                value: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    d.open_ports = conn
        .prepare_cached("SELECT port FROM device_ports WHERE device_id = ?1 ORDER BY port")?
        .query_map([d.id.0], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(d)
}

fn list(conn: &Connection, network: NetworkId) -> Result<Vec<Device>, StoreError> {
    let ids: Vec<i64> = conn
        .prepare_cached("SELECT id FROM devices WHERE network_id = ?1")?
        .query_map([network.0], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.extend(load(conn, network, DeviceId(id))?);
    }
    out.sort_by_key(|d| (!d.online, d.ip.is_none(), d.ip, d.id));
    Ok(out)
}

fn update(
    conn: &mut Connection,
    network: NetworkId,
    id: DeviceId,
    patch: DevicePatch,
) -> Result<Option<Device>, StoreError> {
    let tx = conn.transaction()?;
    let exists = tx
        .query_row(
            "SELECT 1 FROM devices WHERE network_id = ?1 AND id = ?2",
            params![network.0, id.0],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(None);
    }
    if let Some(name) = patch.name {
        let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        tx.execute(
            "UPDATE devices SET name = ?1, is_new = CASE WHEN ?1 IS NULL THEN is_new ELSE 0 END WHERE id = ?2",
            params![name, id.0],
        )?;
    }
    if let Some(starred) = patch.starred {
        tx.execute(
            "UPDATE devices SET starred = ?1 WHERE id = ?2",
            params![starred, id.0],
        )?;
    }
    if let Some(is_new) = patch.is_new {
        tx.execute(
            "UPDATE devices SET is_new = ?1 WHERE id = ?2",
            params![is_new, id.0],
        )?;
    }
    let device = load(&tx, network, id)?;
    tx.commit()?;
    Ok(device)
}

impl Store {
    /// Match the sighting to a device (or create one), merge what it revealed,
    /// and return the device as stored.
    pub async fn record_sighting(
        &self,
        network: NetworkId,
        scan: ScanId,
        sighting: Sighting,
        now: i64,
    ) -> Result<Device, StoreError> {
        self.db
            .call_store(move |c| record(c, network, scan, sighting, now))
            .await
    }

    /// Whether a device on `network` already owns `mac`.
    pub async fn mac_known(&self, network: NetworkId, mac: MacAddr) -> Result<bool, StoreError> {
        let mac = mac.to_string();
        self.db
            .call(move |c| {
                c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM device_macs m JOIN devices d ON d.id = m.device_id
                     WHERE d.network_id = ?1 AND m.mac = ?2)",
                    params![network.0, mac],
                    |r| r.get(0),
                )
            })
            .await
    }

    /// Online devices first, then by address; devices with no address last.
    pub async fn list_devices(&self, network: NetworkId) -> Result<Vec<Device>, StoreError> {
        self.db.call_store(move |c| list(c, network)).await
    }

    pub async fn get_device(
        &self,
        network: NetworkId,
        id: DeviceId,
    ) -> Result<Option<Device>, StoreError> {
        self.db.call_store(move |c| load(c, network, id)).await
    }

    pub async fn update_device(
        &self,
        network: NetworkId,
        id: DeviceId,
        patch: DevicePatch,
    ) -> Result<Option<Device>, StoreError> {
        self.db
            .call_store(move |c| update(c, network, id, patch))
            .await
    }
}
