//! Monitoring sessions: spans during which the monitor was watching a network.

use rusqlite::{params, Connection};

use super::{enum_parse, enum_text, Store, StoreError};
use crate::model::NetworkId;
use crate::monitor::{EndReason, SessionId, SessionSpan};

impl Store {
    pub async fn open_session(
        &self,
        network: NetworkId,
        now: i64,
    ) -> Result<SessionId, StoreError> {
        self.db
            .call(move |c| {
                c.query_row(
                    "INSERT INTO monitor_sessions(network_id, started_at, last_tick_at) VALUES(?1, ?2, ?2) RETURNING id",
                    params![network.0, now],
                    |r| r.get(0).map(SessionId),
                )
            })
            .await
    }

    pub async fn touch_session(&self, id: SessionId, now: i64) -> Result<(), StoreError> {
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE monitor_sessions SET last_tick_at = ?2 WHERE id = ?1",
                    params![id.0, now],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn end_session(
        &self,
        id: SessionId,
        at: i64,
        reason: EndReason,
    ) -> Result<(), StoreError> {
        let reason = enum_text(&reason);
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE monitor_sessions SET ended_at = ?2, end_reason = ?3 WHERE id = ?1",
                    params![id.0, at, reason],
                )?;
                Ok(())
            })
            .await
    }
}

/// Sessions that were still open when the program stopped ended in a crash,
/// at the last time they were seen alive.
pub(crate) fn end_dangling_sessions(conn: &Connection) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE monitor_sessions SET ended_at = last_tick_at, end_reason = 'crash' WHERE ended_at IS NULL",
        [],
    )?;
    Ok(())
}

/// Sessions overlapping `[from, to]`, oldest first.
pub(crate) fn sessions_in(
    c: &Connection,
    network: NetworkId,
    from: i64,
    to: i64,
) -> Result<Vec<SessionSpan>, StoreError> {
    let mut st = c.prepare(
        "SELECT started_at, ended_at, end_reason FROM monitor_sessions
         WHERE network_id = ?1 AND started_at <= ?3 AND COALESCE(ended_at, last_tick_at) >= ?2
         ORDER BY started_at, id",
    )?;
    let rows = st.query_map(params![network.0, from, to], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (started_at, ended_at, reason) = row?;
        out.push(SessionSpan {
            started_at,
            ended_at,
            end_reason: reason.as_deref().map(enum_parse).transpose()?,
        });
    }
    Ok(out)
}
