//! Per-network alert rule settings, kept in the `settings` table.

use rusqlite::{params, Connection, OptionalExtension};

use super::{Store, StoreError};
use crate::model::NetworkId;
use crate::monitor::RuleSettings;

fn scope(network: NetworkId) -> String {
    format!("network:{}", network.0)
}

fn fields(s: &mut RuleSettings) -> [(&'static str, &mut bool); 6] {
    [
        ("rule.starred_offline", &mut s.starred_offline),
        ("rule.internet_down", &mut s.internet_down),
        ("rule.latency_high", &mut s.latency_high),
        ("rule.packet_loss", &mut s.packet_loss),
        ("rule.new_device", &mut s.new_device),
        ("notifications", &mut s.notifications),
    ]
}

fn read(conn: &Connection, network: NetworkId) -> rusqlite::Result<RuleSettings> {
    let scope = scope(network);
    let mut settings = RuleSettings::default();
    for (key, slot) in fields(&mut settings) {
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE scope = ?1 AND key = ?2",
                params![scope, key],
                |r| r.get(0),
            )
            .optional()?;
        // A missing key reads as on.
        *slot = value.as_deref() != Some("off");
    }
    Ok(settings)
}

fn write(
    conn: &mut Connection,
    network: NetworkId,
    mut settings: RuleSettings,
) -> rusqlite::Result<()> {
    let scope = scope(network);
    let tx = conn.transaction()?;
    for (key, on) in fields(&mut settings) {
        tx.execute(
            "INSERT INTO settings(scope, key, value) VALUES(?1, ?2, ?3)
             ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value",
            params![scope, key, if *on { "on" } else { "off" }],
        )?;
    }
    tx.commit()
}

impl Store {
    pub async fn rule_settings(&self, network: NetworkId) -> Result<RuleSettings, StoreError> {
        self.db.call(move |c| read(c, network)).await
    }

    pub async fn set_rule_settings(
        &self,
        network: NetworkId,
        settings: RuleSettings,
    ) -> Result<(), StoreError> {
        self.db.call(move |c| write(c, network, settings)).await
    }
}
