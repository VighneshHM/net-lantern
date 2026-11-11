//! Alerts, incidents and their timelines.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{enum_parse, enum_text, Store, StoreError};
use crate::model::NetworkId;
use crate::monitor::{
    AckOutcome, Alert, AlertFilter, AlertId, AlertKey, AlertState, Decision, Incident, IncidentId,
    Severity, TimelineEvent, TimelineKind,
};

const ALERT_COLS: &str = "id, rule, key, severity, title, detail, suppressed, state, incident_id, \
                          opened_at, acked_at, resolved_at";

type RawAlert = (
    i64,
    String,
    String,
    String,
    String,
    String,
    bool,
    String,
    i64,
    i64,
    Option<i64>,
    Option<i64>,
);

fn raw_alert(r: &rusqlite::Row<'_>) -> rusqlite::Result<RawAlert> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
    ))
}

fn to_alert(raw: RawAlert) -> Result<Alert, StoreError> {
    Ok(Alert {
        id: AlertId(raw.0),
        rule: enum_parse(&raw.1)?,
        key: raw.2,
        severity: enum_parse(&raw.3)?,
        title: raw.4,
        detail: raw.5,
        suppressed: raw.6,
        state: enum_parse(&raw.7)?,
        incident_id: IncidentId(raw.8),
        opened_at: raw.9,
        acked_at: raw.10,
        resolved_at: raw.11,
    })
}

fn alert_by_id(
    conn: &Connection,
    network: NetworkId,
    id: i64,
) -> Result<Option<Alert>, StoreError> {
    conn.query_row(
        &format!("SELECT {ALERT_COLS} FROM alerts WHERE network_id = ?1 AND id = ?2"),
        params![network.0, id],
        raw_alert,
    )
    .optional()?
    .map(to_alert)
    .transpose()
}

/// The active (open or acked) alert for a key.
fn active_alert(
    conn: &Connection,
    network: NetworkId,
    key: &str,
) -> Result<Option<Alert>, StoreError> {
    conn.query_row(
        &format!(
            "SELECT {ALERT_COLS} FROM alerts
             WHERE network_id = ?1 AND key = ?2 AND state != 'resolved'"
        ),
        params![network.0, key],
        raw_alert,
    )
    .optional()?
    .map(to_alert)
    .transpose()
}

fn add_event(
    tx: &Transaction<'_>,
    incident: IncidentId,
    alert: Option<AlertId>,
    ts: i64,
    kind: TimelineKind,
    text: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO incident_events(incident_id, alert_id, ts, kind, text)
         VALUES(?1, ?2, ?3, ?4, ?5)",
        params![incident.0, alert.map(|a| a.0), ts, enum_text(&kind), text],
    )?;
    Ok(())
}

struct OpenArgs<'a> {
    key: &'a AlertKey,
    title: &'a str,
    detail: &'a str,
    severity: Severity,
    suppressed: bool,
    detected_at: i64,
    last_good_at: Option<i64>,
    at: i64,
}

fn apply_open(
    tx: &Transaction<'_>,
    network: NetworkId,
    a: OpenArgs<'_>,
) -> Result<Option<Alert>, StoreError> {
    let key_text = a.key.to_string();
    if active_alert(tx, network, &key_text)?.is_some() {
        return Ok(None);
    }
    // A suppressed alert joins the incident of the live router_down alert.
    let router_incident: Option<i64> = if a.suppressed {
        tx.query_row(
            "SELECT incident_id FROM alerts
             WHERE network_id = ?1 AND key = 'router_down'
               AND state != 'resolved' AND suppressed = 0",
            params![network.0],
            |r| r.get(0),
        )
        .optional()?
    } else {
        None
    };
    // The incident is created first (alerts.incident_id is NOT NULL) and gets
    // its root once the alert exists.
    let incident_id = match router_incident {
        Some(i) => IncidentId(i),
        None => IncidentId(tx.query_row(
            "INSERT INTO incidents(network_id, root_alert_id, opened_at)
             VALUES(?1, NULL, ?2) RETURNING id",
            params![network.0, a.at],
            |r| r.get(0),
        )?),
    };
    let alert_id = AlertId(tx.query_row(
        "INSERT INTO alerts(network_id, incident_id, rule, key, severity, title, detail,
                            suppressed, state, opened_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9) RETURNING id",
        params![
            network.0,
            incident_id.0,
            enum_text(&a.key.rule()),
            key_text,
            enum_text(&a.severity),
            a.title,
            a.detail,
            a.suppressed,
            a.at
        ],
        |r| r.get(0),
    )?);
    if router_incident.is_some() {
        add_event(
            tx,
            incident_id,
            Some(alert_id),
            a.at,
            TimelineKind::Affected,
            &format!("{} (held back while the router is down)", a.title),
        )?;
    } else {
        tx.execute(
            "UPDATE incidents SET root_alert_id = ?1 WHERE id = ?2",
            params![alert_id.0, incident_id.0],
        )?;
        let some = Some(alert_id);
        if let Some(t) = a.last_good_at {
            add_event(
                tx,
                incident_id,
                some,
                t,
                TimelineKind::LastGood,
                "Last good check",
            )?;
        }
        add_event(
            tx,
            incident_id,
            some,
            a.detected_at,
            TimelineKind::Detected,
            "Problem detected",
        )?;
        add_event(
            tx,
            incident_id,
            some,
            a.at,
            TimelineKind::Alerted,
            "Alert raised",
        )?;
    }
    alert_by_id(tx, network, alert_id.0)
}

fn apply_unsuppress(
    tx: &Transaction<'_>,
    network: NetworkId,
    key: &AlertKey,
    at: i64,
) -> Result<Option<Alert>, StoreError> {
    let Some(alert) = active_alert(tx, network, &key.to_string())? else {
        return Ok(None);
    };
    // A fallback root (suppressed with no live router incident) keeps its incident.
    let is_root: bool = tx.query_row(
        "SELECT root_alert_id = ?1 FROM incidents WHERE id = ?2",
        params![alert.id.0, alert.incident_id.0],
        |r| r.get(0),
    )?;
    if is_root {
        tx.execute(
            "UPDATE alerts SET suppressed = 0 WHERE id = ?1",
            params![alert.id.0],
        )?;
        add_event(
            tx,
            alert.incident_id,
            Some(alert.id),
            at,
            TimelineKind::Alerted,
            "Alert raised",
        )?;
        return alert_by_id(tx, network, alert.id.0);
    }
    let old_incident = alert.incident_id;
    let new_incident = IncidentId(tx.query_row(
        "INSERT INTO incidents(network_id, root_alert_id, opened_at)
         VALUES(?1, ?2, ?3) RETURNING id",
        params![network.0, alert.id.0, alert.opened_at],
        |r| r.get(0),
    )?);
    tx.execute(
        "UPDATE alerts SET suppressed = 0, incident_id = ?1 WHERE id = ?2",
        params![new_incident.0, alert.id.0],
    )?;
    let some = Some(alert.id);
    add_event(
        tx,
        new_incident,
        some,
        alert.opened_at,
        TimelineKind::Detected,
        "Problem detected",
    )?;
    add_event(
        tx,
        new_incident,
        some,
        at,
        TimelineKind::Alerted,
        "Alert raised",
    )?;
    add_event(
        tx,
        old_incident,
        some,
        at,
        TimelineKind::Affected,
        &format!(
            "{} is still failing after the router recovered",
            alert.title
        ),
    )?;
    alert_by_id(tx, network, alert.id.0)
}

fn apply_resolve(
    tx: &Transaction<'_>,
    network: NetworkId,
    key: &AlertKey,
    at: i64,
) -> Result<Option<Alert>, StoreError> {
    let Some(alert) = active_alert(tx, network, &key.to_string())? else {
        return Ok(None);
    };
    tx.execute(
        "UPDATE alerts SET state = 'resolved', resolved_at = ?1 WHERE id = ?2",
        params![at, alert.id.0],
    )?;
    add_event(
        tx,
        alert.incident_id,
        Some(alert.id),
        at,
        TimelineKind::Recovered,
        "Recovered",
    )?;
    tx.execute(
        "UPDATE incidents SET resolved_at = ?1 WHERE id = ?2 AND root_alert_id = ?3",
        params![at, alert.incident_id.0, alert.id.0],
    )?;
    alert_by_id(tx, network, alert.id.0)
}

impl Store {
    /// Apply the engine's decisions in one transaction. Returns every alert
    /// that changed, in order. Decisions with nothing to act on are skipped.
    pub async fn apply_decisions(
        &self,
        network: NetworkId,
        decisions: Vec<Decision>,
    ) -> Result<Vec<Alert>, StoreError> {
        self.db
            .call_store(move |c| {
                let tx = c.transaction()?;
                let mut changed = Vec::new();
                for d in &decisions {
                    let alert = match d {
                        Decision::Open {
                            key,
                            title,
                            detail,
                            severity,
                            suppressed,
                            detected_at,
                            last_good_at,
                            at,
                        } => apply_open(
                            &tx,
                            network,
                            OpenArgs {
                                key,
                                title,
                                detail,
                                severity: *severity,
                                suppressed: *suppressed,
                                detected_at: *detected_at,
                                last_good_at: *last_good_at,
                                at: *at,
                            },
                        )?,
                        Decision::Unsuppress { key, at } => {
                            apply_unsuppress(&tx, network, key, *at)?
                        }
                        Decision::Resolve { key, at } => apply_resolve(&tx, network, key, *at)?,
                    };
                    changed.extend(alert);
                }
                tx.commit()?;
                Ok(changed)
            })
            .await
    }

    /// Acknowledge an alert. Acking an acknowledged alert returns it unchanged.
    pub async fn ack_alert(
        &self,
        network: NetworkId,
        id: AlertId,
        ts: i64,
    ) -> Result<AckOutcome, StoreError> {
        self.db
            .call_store(move |c| {
                let tx = c.transaction()?;
                let Some(alert) = alert_by_id(&tx, network, id.0)? else {
                    return Ok(AckOutcome::NotFound);
                };
                match alert.state {
                    AlertState::Resolved => return Ok(AckOutcome::AlreadyResolved(alert)),
                    AlertState::Acked => return Ok(AckOutcome::Acked(alert)),
                    AlertState::Open => {}
                }
                tx.execute(
                    "UPDATE alerts SET state = 'acked', acked_at = ?1 WHERE id = ?2",
                    params![ts, id.0],
                )?;
                add_event(
                    &tx,
                    alert.incident_id,
                    Some(id),
                    ts,
                    TimelineKind::Acknowledged,
                    "Acknowledged",
                )?;
                let updated = alert_by_id(&tx, network, id.0)?.ok_or_else(|| {
                    StoreError::Sqlite("alert vanished during acknowledge".into())
                })?;
                tx.commit()?;
                Ok(AckOutcome::Acked(updated))
            })
            .await
    }

    /// Alerts matching `filter`, newest first.
    pub async fn list_alerts(
        &self,
        network: NetworkId,
        filter: AlertFilter,
        limit: u32,
    ) -> Result<Vec<Alert>, StoreError> {
        let clause = match filter {
            AlertFilter::All => "",
            AlertFilter::Active => "AND state != 'resolved'",
            AlertFilter::Open => "AND state = 'open'",
            AlertFilter::Acked => "AND state = 'acked'",
            AlertFilter::Resolved => "AND state = 'resolved'",
        };
        self.db
            .call_store(move |c| {
                let mut stmt = c.prepare(&format!(
                    "SELECT {ALERT_COLS} FROM alerts WHERE network_id = ?1 {clause}
                     ORDER BY id DESC LIMIT ?2"
                ))?;
                let rows = stmt
                    .query_map(params![network.0, limit], raw_alert)?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.into_iter().map(to_alert).collect()
            })
            .await
    }

    /// An incident with its alerts and timeline.
    pub async fn incident(
        &self,
        network: NetworkId,
        id: IncidentId,
    ) -> Result<Option<Incident>, StoreError> {
        self.db
            .call_store(move |c| {
                let row: Option<(i64, Option<i64>, Option<i64>)> = c
                    .query_row(
                        "SELECT opened_at, resolved_at, root_alert_id FROM incidents
                         WHERE network_id = ?1 AND id = ?2",
                        params![network.0, id.0],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((opened_at, resolved_at, Some(root_id))) = row else {
                    return Ok(None);
                };
                let mut stmt = c.prepare(&format!(
                    "SELECT {ALERT_COLS} FROM alerts WHERE network_id = ?1 AND incident_id = ?2
                     ORDER BY id"
                ))?;
                let alerts = stmt
                    .query_map(params![network.0, id.0], raw_alert)?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(to_alert)
                    .collect::<Result<Vec<_>, _>>()?;
                let root = alerts
                    .iter()
                    .find(|a| a.id.0 == root_id)
                    .cloned()
                    .ok_or_else(|| StoreError::Sqlite("incident root alert is missing".into()))?;
                let mut stmt = c.prepare(
                    "SELECT ts, kind, text, alert_id FROM incident_events
                     WHERE incident_id = ?1 ORDER BY ts, rowid",
                )?;
                let raw = stmt
                    .query_map(params![id.0], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, Option<i64>>(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                let timeline = raw
                    .into_iter()
                    .map(|(ts, kind, text, alert_id)| {
                        Ok(TimelineEvent {
                            ts,
                            kind: enum_parse(&kind)?,
                            text,
                            alert_id: alert_id.map(AlertId),
                        })
                    })
                    .collect::<Result<Vec<_>, StoreError>>()?;
                Ok(Some(Incident {
                    id,
                    opened_at,
                    resolved_at,
                    root,
                    alerts,
                    timeline,
                }))
            })
            .await
    }

    /// Keys of active alerts with their `suppressed` flag, oldest first.
    pub async fn active_alert_keys(
        &self,
        network: NetworkId,
    ) -> Result<Vec<(AlertKey, bool)>, StoreError> {
        self.db
            .call_store(move |c| {
                let mut stmt = c.prepare(
                    "SELECT key, suppressed FROM alerts
                     WHERE network_id = ?1 AND state != 'resolved' ORDER BY id",
                )?;
                let rows = stmt
                    .query_map(params![network.0], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.into_iter()
                    .map(|(k, s)| {
                        k.parse::<AlertKey>()
                            .map(|k| (k, s))
                            .map_err(|_| StoreError::Sqlite(format!("unexpected stored key {k:?}")))
                    })
                    .collect()
            })
            .await
    }

    /// Add a `Diagnosed` event to every unresolved incident; returns their ids.
    pub async fn add_diagnosis_to_open_incidents(
        &self,
        network: NetworkId,
        headline: &str,
        ts: i64,
    ) -> Result<Vec<IncidentId>, StoreError> {
        let text = format!("Diagnosis: {headline}");
        self.db
            .call_store(move |c| {
                let tx = c.transaction()?;
                let ids = tx
                    .prepare(
                        "SELECT id FROM incidents
                         WHERE network_id = ?1 AND resolved_at IS NULL ORDER BY id",
                    )?
                    .query_map(params![network.0], |r| r.get(0).map(IncidentId))?
                    .collect::<Result<Vec<_>, _>>()?;
                for id in &ids {
                    add_event(&tx, *id, None, ts, TimelineKind::Diagnosed, &text)?;
                }
                tx.commit()?;
                Ok(ids)
            })
            .await
    }
}
