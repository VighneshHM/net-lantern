//! "Delete all data": every row of every table goes, the schema stays.

use super::{Store, StoreError};

/// Every table the migrations create, children before their parents so no
/// foreign key is broken part-way. A table added to `migrations.rs` belongs
/// here too: `reset_all_empties_every_table_but_keeps_the_schema` fails
/// until it is.
const TABLES: &[&str] = &[
    // Rows of a device.
    "presence",
    "device_macs",
    "device_services",
    "device_ports",
    "checks",
    // Alerts and their incidents.
    "incident_events",
    "alerts",
    "incidents",
    // Monitoring history.
    "monitor_sessions",
    "samples",
    "samples_rollup",
    // Scans and devices, then the settings (global ones included: the
    // update question is asked again), then the networks they belong to.
    "scans",
    "devices",
    "settings",
    "networks",
];

impl Store {
    /// Deletes every row of every table in one transaction: all of it goes,
    /// or (on any error) none of it. The tables, indexes and schema version
    /// stay, so the store works as new afterwards.
    ///
    /// Deleted content is overwritten in the file (`secure_delete`) and the
    /// write-ahead log is folded in and truncated, so the old rows do not
    /// linger in free pages or in the log. Then the copy a schema upgrade
    /// left beside the file (`<name>.bak`, the whole old database) is
    /// removed.
    ///
    /// [`StoreError::BackupNotRemoved`] when that copy is there but cannot
    /// be removed: the rows are already gone, but the data is not, so this
    /// is not reported as success.
    pub async fn reset_all(&self) -> Result<(), StoreError> {
        let backup = self.db.backup_path();
        self.db
            .call_store(move |c| {
                let before: i64 = c.query_row("PRAGMA secure_delete", [], |r| r.get(0))?;
                c.execute_batch("PRAGMA secure_delete = ON")?;
                let deleted = (|| {
                    let tx = c.transaction()?;
                    for table in TABLES {
                        tx.execute(&format!("DELETE FROM {table}"), [])?;
                    }
                    tx.commit()
                })();
                // Restored whatever happened: this connection serves every
                // other call too.
                let restored = c.execute_batch(&format!("PRAGMA secure_delete = {before}"));
                deleted?;
                restored?;
                // Best effort: with a reader holding the log, not everything
                // can be folded in now; SQLite does it later.
                if let Err(error) = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
                    tracing::warn!(%error, "could not fold the log in after a reset");
                }
                match backup.map(std::fs::remove_file) {
                    None | Some(Ok(())) => Ok(()),
                    Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Some(Err(e)) => {
                        // The kind only: the message may carry the path.
                        tracing::warn!(kind = ?e.kind(), "could not remove the old database backup");
                        Err(StoreError::BackupNotRemoved)
                    }
                }
            })
            .await
    }
}
