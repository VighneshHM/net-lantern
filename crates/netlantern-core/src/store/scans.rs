//! Scan history rows.

use rusqlite::{params, Connection, OptionalExtension};

use super::{enum_parse, enum_text, Store, StoreError};
use crate::model::{NetworkId, ScanId, ScanMode, ScanSnapshot, ScanStage, ScanState};

const SELECT: &str = "SELECT id, mode, state, devices_found, error, isolated FROM scans";

fn snapshot(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<Option<ScanSnapshot>, StoreError> {
    let row = conn
        .query_row(sql, args, |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .optional()?;
    row.map(|(id, mode, state, devices_found, error, isolated)| {
        Ok(ScanSnapshot {
            id: ScanId(id as u64),
            mode: enum_parse::<ScanMode>(&mode)?,
            state: enum_parse::<ScanState>(&state)?,
            stage: ScanStage::Merge,
            done: 0,
            total: 0,
            devices_found,
            error,
            isolated,
        })
    })
    .transpose()
}

impl Store {
    /// Record a new scan in state `running`.
    pub async fn insert_scan(&self, mode: ScanMode, now: i64) -> Result<ScanId, StoreError> {
        let mode = enum_text(&mode);
        self.db
            .call(move |c| {
                c.query_row(
                    "INSERT INTO scans(mode, state, started_at) VALUES(?1, 'running', ?2) RETURNING id",
                    params![mode, now],
                    |r| r.get::<_, i64>(0).map(|i| ScanId(i as u64)),
                )
            })
            .await
    }

    pub async fn set_scan_network(&self, id: ScanId, network: NetworkId) -> Result<(), StoreError> {
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE scans SET network_id = ?1 WHERE id = ?2",
                    params![network.0, id.0 as i64],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn finish_scan(
        &self,
        id: ScanId,
        state: ScanState,
        error: Option<String>,
        isolated: bool,
        devices_found: u32,
        now: i64,
    ) -> Result<(), StoreError> {
        let state = enum_text(&state);
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE scans SET state = ?1, error = ?2, isolated = ?3, devices_found = ?4, finished_at = ?5
                     WHERE id = ?6",
                    params![state, error, isolated, devices_found, now, id.0 as i64],
                )
                .map(|_| ())
            })
            .await
    }

    pub async fn scan(&self, id: ScanId) -> Result<Option<ScanSnapshot>, StoreError> {
        self.db
            .call_store(move |c| snapshot(c, &format!("{SELECT} WHERE id = ?1"), [id.0 as i64]))
            .await
    }

    pub async fn latest_scan(&self) -> Result<Option<ScanSnapshot>, StoreError> {
        self.db
            .call_store(|c| snapshot(c, &format!("{SELECT} ORDER BY id DESC LIMIT 1"), []))
            .await
    }

    /// The most recent scan of `network`.
    pub async fn latest_scan_for(
        &self,
        network: NetworkId,
    ) -> Result<Option<ScanSnapshot>, StoreError> {
        self.db
            .call_store(move |c| {
                snapshot(
                    c,
                    &format!("{SELECT} WHERE network_id = ?1 ORDER BY id DESC LIMIT 1"),
                    [network.0],
                )
            })
            .await
    }
}
