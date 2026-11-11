//! Service checks on starred devices.

use rusqlite::{params, Connection, OptionalExtension};

use super::devices::load;
use super::{Store, StoreError};
use crate::model::{Device, DeviceId, NetworkId};
use crate::monitor::Check;

pub(super) fn load_checks(conn: &Connection, id: DeviceId) -> Result<Vec<Check>, StoreError> {
    let rows: Vec<String> = conn
        .prepare_cached("SELECT params FROM checks WHERE device_id = ?1 ORDER BY position")?
        .query_map([id.0], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    rows.iter()
        .map(|p| {
            serde_json::from_str(p)
                .map_err(|e| StoreError::Sqlite(format!("stored check is unreadable: {e}")))
        })
        .collect()
}

fn kind(check: &Check) -> &'static str {
    match check {
        Check::Tcp { .. } => "tcp",
        Check::Http { .. } => "http",
    }
}

fn replace(
    conn: &mut Connection,
    network: NetworkId,
    id: DeviceId,
    checks: Vec<Check>,
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
    tx.execute("DELETE FROM checks WHERE device_id = ?1", [id.0])?;
    for (position, check) in checks.iter().enumerate() {
        let json = serde_json::to_string(check)
            .map_err(|e| StoreError::Sqlite(format!("check cannot be stored: {e}")))?;
        tx.execute(
            "INSERT INTO checks(device_id, position, kind, params) VALUES(?1, ?2, ?3, ?4)",
            params![id.0, position as i64, kind(check), json],
        )?;
    }
    let device = load(&tx, network, id)?;
    tx.commit()?;
    Ok(device)
}

impl Store {
    /// Replace the device's checks; `None` when the device is not on `network`.
    pub async fn set_checks(
        &self,
        network: NetworkId,
        device: DeviceId,
        checks: Vec<Check>,
    ) -> Result<Option<Device>, StoreError> {
        self.db
            .call_store(move |c| replace(c, network, device, checks))
            .await
    }
}
