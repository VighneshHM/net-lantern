//! Monitoring samples, five-minute rollups and retention.

use rusqlite::params;

use super::{enum_text, Store, StoreError};
use crate::model::NetworkId;
use crate::monitor::{Metric, MetricPoint, MetricSeries, Resolution, Retention, Sample, Target};

const BUCKET_MS: i64 = 300_000;
const DAY_MS: i64 = 86_400_000;

impl Store {
    /// Store samples in one transaction.
    pub async fn insert_samples(
        &self,
        network: NetworkId,
        samples: Vec<Sample>,
    ) -> Result<(), StoreError> {
        if samples.is_empty() {
            return Ok(());
        }
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                {
                    let mut ins = tx.prepare(
                        "INSERT INTO samples(network_id, target, metric, ts, value) VALUES(?1, ?2, ?3, ?4, ?5)",
                    )?;
                    for s in &samples {
                        ins.execute(params![
                            network.0,
                            s.target.to_string(),
                            enum_text(&s.metric),
                            s.ts,
                            s.value
                        ])?;
                    }
                }
                tx.commit()
            })
            .await
    }

    /// Fold raw samples of every finished five-minute bucket into rollups.
    pub async fn rollup(&self, now: i64) -> Result<(), StoreError> {
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT OR REPLACE INTO samples_rollup(network_id, target, metric, bucket_start, min, avg, max, count, lost)
                     SELECT network_id, target, metric, ts - ts % ?1, MIN(value), AVG(value), MAX(value),
                            COUNT(*), SUM(value IS NULL)
                     FROM samples
                     WHERE ts - ts % ?1 + ?1 <= ?2
                     GROUP BY network_id, target, metric, ts - ts % ?1",
                    params![BUCKET_MS, now],
                )?;
                Ok(())
            })
            .await
    }

    /// Delete data older than the retention periods.
    pub async fn prune(&self, now: i64, r: &Retention) -> Result<(), StoreError> {
        let raw = now - r.raw.as_millis() as i64;
        let rollups = now - r.rollups.as_millis() as i64;
        let presence = now - r.presence.as_millis() as i64;
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                tx.execute("DELETE FROM samples WHERE ts < ?1", [raw])?;
                tx.execute(
                    "DELETE FROM samples_rollup WHERE bucket_start < ?1",
                    [rollups],
                )?;
                tx.execute("DELETE FROM presence WHERE ts < ?1", [presence])?;
                tx.execute(
                    "DELETE FROM monitor_sessions WHERE ended_at IS NOT NULL AND ended_at < ?1",
                    [rollups],
                )?;
                tx.commit()
            })
            .await
    }

    /// A metric's history between `from` and `to`, raw for up to a day, else rollups.
    pub async fn metrics(
        &self,
        network: NetworkId,
        target: Target,
        metric: Metric,
        from: i64,
        to: i64,
    ) -> Result<MetricSeries, StoreError> {
        let (target, metric) = (target.to_string(), enum_text(&metric));
        self.db
            .call_store(move |c| {
                let raw = to - from <= DAY_MS;
                let points: Vec<MetricPoint> = if raw {
                    let mut st = c.prepare(
                        "SELECT ts, value FROM samples
                         WHERE network_id = ?1 AND target = ?2 AND metric = ?3 AND ts >= ?4 AND ts <= ?5
                         ORDER BY ts",
                    )?;
                    let rows = st.query_map(params![network.0, target, metric, from, to], |r| {
                        let v: Option<f64> = r.get(1)?;
                        Ok(MetricPoint {
                            ts: r.get(0)?,
                            min: v,
                            avg: v,
                            max: v,
                            count: 1,
                            lost: u32::from(v.is_none()),
                        })
                    })?;
                    rows.collect::<Result<_, _>>()?
                } else {
                    let mut st = c.prepare(
                        "SELECT bucket_start, min, avg, max, count, lost FROM samples_rollup
                         WHERE network_id = ?1 AND target = ?2 AND metric = ?3 AND bucket_start >= ?4 AND bucket_start <= ?5
                         ORDER BY bucket_start",
                    )?;
                    let rows = st.query_map(params![network.0, target, metric, from, to], |r| {
                        Ok(MetricPoint {
                            ts: r.get(0)?,
                            min: r.get(1)?,
                            avg: r.get(2)?,
                            max: r.get(3)?,
                            count: r.get(4)?,
                            lost: r.get(5)?,
                        })
                    })?;
                    rows.collect::<Result<_, _>>()?
                };
                let sessions = super::sessions::sessions_in(c, network, from, to)?;
                Ok(MetricSeries {
                    resolution: if raw {
                        Resolution::Raw
                    } else {
                        Resolution::FiveMin
                    },
                    points,
                    sessions,
                })
            })
            .await
    }
}
