//! Schema versions, tracked in `PRAGMA user_version`.

use std::path::Path;

use rusqlite::Connection;

use super::StoreError;

/// Entry `n` upgrades the schema from version `n` to `n + 1`.
pub(crate) const MIGRATIONS: &[&str] = &[SCHEMA_V1, SCHEMA_V2];

const SCHEMA_V1: &str = "
CREATE TABLE networks(
    id INTEGER PRIMARY KEY, gateway_mac TEXT NOT NULL, subnet TEXT NOT NULL, ssid_label TEXT,
    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, scan_consent INTEGER NOT NULL DEFAULT 0,
    UNIQUE(gateway_mac, subnet));
CREATE TABLE devices(
    id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id), last_ip TEXT,
    hostname TEXT, vendor TEXT, type TEXT NOT NULL DEFAULT 'unknown', type_confidence REAL NOT NULL DEFAULT 0,
    name TEXT, starred INTEGER NOT NULL DEFAULT 0, is_new INTEGER NOT NULL DEFAULT 1,
    answers_ping INTEGER NOT NULL DEFAULT 0, rtt_ms INTEGER, ttl INTEGER, is_self INTEGER NOT NULL DEFAULT 0,
    is_gateway INTEGER NOT NULL DEFAULT 0, last_scan_id INTEGER, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
    evidence TEXT);
CREATE TABLE device_macs(
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, mac TEXT NOT NULL,
    private_mac INTEGER NOT NULL, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
    PRIMARY KEY(device_id, mac));
CREATE TABLE device_services(
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, source TEXT NOT NULL,
    value TEXT NOT NULL, last_seen INTEGER NOT NULL, PRIMARY KEY(device_id, source, value));
CREATE TABLE device_ports(
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, port INTEGER NOT NULL,
    proto TEXT NOT NULL DEFAULT 'tcp', service TEXT, banner TEXT, seen_at INTEGER NOT NULL,
    PRIMARY KEY(device_id, port, proto));
CREATE TABLE scans(
    id INTEGER PRIMARY KEY, network_id INTEGER REFERENCES networks(id), mode TEXT NOT NULL, state TEXT NOT NULL,
    error TEXT, isolated INTEGER NOT NULL DEFAULT 0, started_at INTEGER NOT NULL, finished_at INTEGER,
    devices_found INTEGER NOT NULL DEFAULT 0);
CREATE TABLE settings(
    scope TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(scope, key));
CREATE INDEX devices_network ON devices(network_id);
CREATE INDEX device_macs_mac ON device_macs(mac);
CREATE INDEX scans_network ON scans(network_id, id);
";

/// Also moves Phase 1 timestamps, written in Unix seconds, to milliseconds
/// like every timestamp since. A value below 10^11 cannot be a millisecond
/// time after 1973, so rows already in milliseconds are left alone.
const SCHEMA_V2: &str = "
UPDATE networks SET first_seen = first_seen * 1000 WHERE first_seen < 100000000000;
UPDATE networks SET last_seen = last_seen * 1000 WHERE last_seen < 100000000000;
UPDATE devices SET first_seen = first_seen * 1000 WHERE first_seen < 100000000000;
UPDATE devices SET last_seen = last_seen * 1000 WHERE last_seen < 100000000000;
UPDATE device_macs SET first_seen = first_seen * 1000 WHERE first_seen < 100000000000;
UPDATE device_macs SET last_seen = last_seen * 1000 WHERE last_seen < 100000000000;
UPDATE device_services SET last_seen = last_seen * 1000 WHERE last_seen < 100000000000;
UPDATE device_ports SET seen_at = seen_at * 1000 WHERE seen_at < 100000000000;
UPDATE scans SET started_at = started_at * 1000 WHERE started_at < 100000000000;
UPDATE scans SET finished_at = finished_at * 1000 WHERE finished_at < 100000000000;
ALTER TABLE devices ADD COLUMN presence TEXT;
ALTER TABLE devices ADD COLUMN presence_changed_at INTEGER;
CREATE TABLE presence(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, ts INTEGER NOT NULL, online INTEGER NOT NULL);
CREATE INDEX presence_device ON presence(device_id, ts);
CREATE TABLE monitor_sessions(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id),
    started_at INTEGER NOT NULL, last_tick_at INTEGER NOT NULL, ended_at INTEGER, end_reason TEXT);
CREATE INDEX monitor_sessions_network ON monitor_sessions(network_id, started_at);
CREATE TABLE samples(network_id INTEGER NOT NULL, target TEXT NOT NULL, metric TEXT NOT NULL, ts INTEGER NOT NULL, value REAL);
CREATE INDEX samples_lookup ON samples(network_id, target, metric, ts);
CREATE TABLE samples_rollup(network_id INTEGER NOT NULL, target TEXT NOT NULL, metric TEXT NOT NULL, bucket_start INTEGER NOT NULL,
    min REAL, avg REAL, max REAL, count INTEGER NOT NULL, lost INTEGER NOT NULL,
    PRIMARY KEY(network_id, target, metric, bucket_start));
CREATE TABLE checks(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    kind TEXT NOT NULL, params TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, PRIMARY KEY(device_id, position));
CREATE TABLE incidents(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id), root_alert_id INTEGER,
    opened_at INTEGER NOT NULL, resolved_at INTEGER);
CREATE TABLE alerts(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id),
    incident_id INTEGER NOT NULL REFERENCES incidents(id), rule TEXT NOT NULL, key TEXT NOT NULL, severity TEXT NOT NULL,
    title TEXT NOT NULL, detail TEXT NOT NULL, suppressed INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL,
    opened_at INTEGER NOT NULL, acked_at INTEGER, resolved_at INTEGER);
CREATE INDEX alerts_network ON alerts(network_id, state, id);
CREATE UNIQUE INDEX alerts_one_active ON alerts(network_id, key) WHERE state != 'resolved';
CREATE TABLE incident_events(incident_id INTEGER NOT NULL REFERENCES incidents(id), alert_id INTEGER,
    ts INTEGER NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL);
CREATE INDEX incident_events_incident ON incident_events(incident_id, ts);
";

/// Fail with `NewerSchema` if the file is newer than `supported`.
pub(crate) fn check_version(conn: &Connection, supported: u32) -> Result<(), StoreError> {
    let found: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if found > supported {
        return Err(StoreError::NewerSchema { found, supported });
    }
    Ok(())
}

/// Bring the database up to `list.len()`. Refuses a newer file, and copies an
/// existing file to `<name>.bak` before upgrading it.
pub(crate) fn apply(
    conn: &mut Connection,
    path: Option<&Path>,
    list: &[&str],
) -> Result<(), StoreError> {
    let found: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = list.len() as u32;
    if found > supported {
        return Err(StoreError::NewerSchema { found, supported });
    }
    if found == supported {
        return Ok(());
    }
    if found > 0 {
        if let Some(p) = path {
            backup(conn, p)?;
        }
    }
    for (i, sql) in list.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1))?;
        tx.commit()?;
    }
    Ok(())
}

fn backup(conn: &Connection, path: &Path) -> Result<(), StoreError> {
    // Fold the write-ahead log into the main file so the copy is complete.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    std::fs::copy(path, &name)
        .map_err(|e| StoreError::Sqlite(format!("could not back up the database: {e}")))?;
    Ok(())
}

/// Scans that were still running when the program stopped can never finish.
pub(crate) fn fail_interrupted_scans(conn: &Connection) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE scans SET state = 'failed', error = 'Net Lantern stopped during the scan.' WHERE state = 'running'",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &[&str] = &["CREATE TABLE a(x)", "CREATE TABLE b(y)"];

    fn version(c: &Connection) -> u32 {
        c.query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn upgrade_backs_up_first() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let bak = dir.path().join("t.db.bak");
        let mut c = Connection::open(&p).unwrap();
        apply(&mut c, Some(&p), &LIST[..1]).unwrap();
        assert!(!bak.exists(), "a fresh file needs no backup");
        drop(c);
        let mut c = Connection::open(&p).unwrap();
        apply(&mut c, Some(&p), LIST).unwrap();
        assert!(bak.exists());
        assert_eq!(version(&c), 2);
    }

    #[test]
    fn fresh_file_has_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let mut c = Connection::open(&p).unwrap();
        apply(&mut c, Some(&p), LIST).unwrap();
        assert!(!dir.path().join("t.db.bak").exists());
        assert_eq!(version(&c), 2);
    }

    #[test]
    fn newer_schema_is_refused() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch("PRAGMA user_version = 3").unwrap();
        assert_eq!(
            apply(&mut c, None, LIST),
            Err(StoreError::NewerSchema {
                found: 3,
                supported: 2
            })
        );
    }
}
