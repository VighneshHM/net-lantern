use netlantern_core::{Db, Metric, Sample, Store, Target};

#[tokio::test]
async fn thirty_days_with_ten_starred_devices_stays_under_50_mb() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("budget.db");
    let db = Db::open(&p).unwrap();
    let store = Store::new(db.clone());
    let net = store
        .network_for("50:C7:BF:00:00:01".parse().unwrap(), "192.168.1.0/24", 0)
        .await
        .unwrap();

    let mut series: Vec<(String, Metric, i64)> = vec![
        ("gateway".into(), Metric::RttMs, 30_000),
        ("internet".into(), Metric::RttMs, 30_000),
        ("dns".into(), Metric::LookupMs, 60_000),
        ("wifi".into(), Metric::RssiDbm, 30_000),
        ("wifi".into(), Metric::LinkRateMbps, 30_000),
    ];
    for i in 1..=10 {
        series.push((format!("device:{i}"), Metric::RttMs, 30_000));
    }
    assert_eq!(series.len(), 15);

    let day = 86_400_000i64;
    let now = 30 * day;
    let mut raw = Vec::new();
    for (t, m, step) in &series {
        let target: Target = t.parse().unwrap();
        let mut ts = now - day;
        while ts < now {
            raw.push(Sample {
                target: target.clone(),
                metric: *m,
                ts,
                value: Some(3.0 + (ts % 17) as f64 * 0.37),
            });
            ts += step;
        }
    }
    store.insert_samples(net, raw).await.unwrap();

    let rollup_series = series.clone();
    db.call(move |c| {
        let tx = c.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO samples_rollup(network_id, target, metric, bucket_start, min, avg, max, count, lost) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )?;
            for (t, m, _) in &rollup_series {
                let name = serde_json::to_value(m).unwrap();
                let name = name.as_str().unwrap().to_string();
                let mut b = 0i64;
                while b < now {
                    let v = 3.0 + (b % 17) as f64 * 0.37;
                    ins.execute(rusqlite::params![
                        net.0,
                        t,
                        name,
                        b,
                        v - 1.0,
                        v,
                        v + 2.5,
                        10,
                        0
                    ])?;
                    b += 300_000;
                }
            }
        }
        for i in 1..=10 {
            tx.execute(
                "INSERT INTO devices(id, network_id, first_seen, last_seen) VALUES(?1, ?2, 0, 0)",
                rusqlite::params![i, net.0],
            )?;
        }
        {
            let mut ins =
                tx.prepare("INSERT INTO presence(device_id, ts, online) VALUES(?1,?2,?3)")?;
            for i in 1..=10i64 {
                for h in 0..(90 * 24) {
                    ins.execute(rusqlite::params![i, now - h * 3_600_000, h % 2])?;
                }
            }
        }
        tx.commit()?;
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    })
    .await
    .unwrap();

    let size = std::fs::metadata(&p).unwrap().len();
    println!(
        "storage budget database size: {} bytes ({:.1} MB)",
        size,
        size as f64 / 1_048_576.0
    );
    assert!(size < 50 * 1024 * 1024, "database is {size} bytes");
}
