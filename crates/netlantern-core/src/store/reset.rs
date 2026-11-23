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
    /// linger in free pages or in the log.
    pub async fn reset_all(&self) -> Result<(), StoreError> {
        self.db
            .call(|c| {
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
                // Best effort: in memory, or with a reader holding the log,
                // there is nothing (or not everything) to fold in.
                if let Err(error) = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
                    tracing::debug!(%error, "could not fold the log in after a reset");
                }
                Ok(())
            })
            .await
    }
}
