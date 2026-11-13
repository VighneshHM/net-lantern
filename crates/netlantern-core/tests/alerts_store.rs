use netlantern_core::{
    AckOutcome, Alert, AlertFilter, AlertId, AlertKey, AlertState, Db, Decision, DeviceId,
    IncidentId, Rule, Severity, Store, TimelineKind,
};
use netlantern_platform::MacAddr;

async fn setup() -> (Store, netlantern_core::NetworkId) {
    let store = Store::new(Db::open_in_memory().unwrap());
    let mac: MacAddr = "50:C7:BF:00:00:01".parse().unwrap();
    let net = store.network_for(mac, "192.168.1.0/24", 1).await.unwrap();
    (store, net)
}

fn open(key: AlertKey, suppressed: bool, at: i64) -> Decision {
    let severity = key.severity();
    Decision::Open {
        title: format!("{key} title"),
        detail: "detail".into(),
        key,
        severity,
        suppressed,
        detected_at: at - 10,
        last_good_at: Some(at - 20),
        at,
    }
}

fn kinds(i: &netlantern_core::Incident) -> Vec<TimelineKind> {
    i.timeline.iter().map(|e| e.kind).collect()
}

#[tokio::test]
async fn opening_an_alert_opens_an_incident_with_its_timeline() {
    let (s, n) = setup().await;
    let out = s
        .apply_decisions(n, vec![open(AlertKey::RouterDown, false, 100)])
        .await
        .unwrap();
    assert_eq!(out.len(), 1);
    let a = &out[0];
    assert_eq!(a.state, AlertState::Open);
    assert_eq!(a.rule, Rule::InternetDown);
    let inc = s.incident(n, a.incident_id).await.unwrap().unwrap();
    assert_eq!(inc.root.id, a.id);
    assert_eq!(inc.opened_at, 100);
    assert_eq!(
        kinds(&inc),
        [
            TimelineKind::LastGood,
            TimelineKind::Detected,
            TimelineKind::Alerted
        ]
    );
    assert_eq!(inc.timeline[0].text, "Last good check");
    assert_eq!(inc.timeline[1].ts, 90);
    assert_eq!(inc.alerts.len(), 1);
}

#[tokio::test]
async fn a_suppressed_alert_joins_the_router_incident() {
    let (s, n) = setup().await;
    let key = AlertKey::StarredOffline(DeviceId(7));
    let out = s
        .apply_decisions(
            n,
            vec![open(AlertKey::RouterDown, false, 100), open(key, true, 110)],
        )
        .await
        .unwrap();
    assert_eq!(out[0].incident_id, out[1].incident_id);
    assert!(out[1].suppressed);
    let inc = s.incident(n, out[0].incident_id).await.unwrap().unwrap();
    assert_eq!(inc.alerts.len(), 2);
    let last = inc.timeline.last().unwrap();
    assert_eq!(last.kind, TimelineKind::Affected);
    assert_eq!(
        last.text,
        "starred_offline:7 title (held back while the router is down)"
    );
    assert_eq!(last.alert_id, Some(out[1].id));
}

#[tokio::test]
async fn unsuppress_gives_the_alert_its_own_incident() {
    let (s, n) = setup().await;
    let key = AlertKey::StarredOffline(DeviceId(7));
    let out = s
        .apply_decisions(
            n,
            vec![
                open(AlertKey::RouterDown, false, 100),
                open(key.clone(), true, 110),
            ],
        )
        .await
        .unwrap();
    let res = s
        .apply_decisions(
            n,
            vec![
                Decision::Resolve {
                    key: AlertKey::RouterDown,
                    at: 200,
                },
                Decision::Unsuppress { key, at: 200 },
            ],
        )
        .await
        .unwrap();
    assert_eq!(res.len(), 2);
    let un = &res[1];
    assert!(!un.suppressed);
    assert_ne!(un.incident_id, out[0].incident_id);
    let own = s.incident(n, un.incident_id).await.unwrap().unwrap();
    assert_eq!(own.root.id, un.id);
    assert_eq!(kinds(&own), [TimelineKind::Detected, TimelineKind::Alerted]);
    assert_eq!(own.timeline[0].ts, 110);
    let router = s.incident(n, out[0].incident_id).await.unwrap().unwrap();
    assert_eq!(router.resolved_at, Some(200));
    assert!(router
        .timeline
        .iter()
        .any(|e| e.kind == TimelineKind::Affected
            && e.text == "starred_offline:7 title is still failing after the router recovered"));
}

#[tokio::test]
async fn resolving_the_root_resolves_the_incident() {
    let (s, n) = setup().await;
    let out = s
        .apply_decisions(n, vec![open(AlertKey::RouterDown, false, 100)])
        .await
        .unwrap();
    let res = s
        .apply_decisions(
            n,
            vec![Decision::Resolve {
                key: AlertKey::RouterDown,
                at: 300,
            }],
        )
        .await
        .unwrap();
    assert_eq!(res[0].state, AlertState::Resolved);
    assert_eq!(res[0].resolved_at, Some(300));
    let inc = s.incident(n, out[0].incident_id).await.unwrap().unwrap();
    assert_eq!(inc.resolved_at, Some(300));
    let last = inc.timeline.last().unwrap();
    assert_eq!(
        (last.kind, last.text.as_str()),
        (TimelineKind::Recovered, "Recovered")
    );
}

#[tokio::test]
async fn a_second_open_for_an_active_key_is_skipped() {
    let (s, n) = setup().await;
    s.apply_decisions(n, vec![open(AlertKey::LatencyHigh, false, 100)])
        .await
        .unwrap();
    let out = s
        .apply_decisions(
            n,
            vec![
                open(AlertKey::LatencyHigh, false, 200),
                open(AlertKey::LatencyHigh, false, 210),
            ],
        )
        .await
        .unwrap();
    assert!(out.is_empty());
    assert_eq!(
        s.list_alerts(n, AlertFilter::All, 10).await.unwrap().len(),
        1
    );
    // Resolve / Unsuppress without an active alert are skipped too.
    let out = s
        .apply_decisions(
            n,
            vec![
                Decision::Resolve {
                    key: AlertKey::InternetDown,
                    at: 5,
                },
                Decision::Unsuppress {
                    key: AlertKey::InternetDown,
                    at: 5,
                },
            ],
        )
        .await
        .unwrap();
    assert!(out.is_empty());
}

#[tokio::test]
async fn ack_then_resolve() {
    let (s, n) = setup().await;
    let out = s
        .apply_decisions(n, vec![open(AlertKey::RouterDown, false, 100)])
        .await
        .unwrap();
    let id = out[0].id;
    let AckOutcome::Acked(a) = s.ack_alert(n, id, 150).await.unwrap() else {
        panic!("expected Acked")
    };
    assert_eq!((a.state, a.acked_at), (AlertState::Acked, Some(150)));
    // Acking again changes nothing.
    let AckOutcome::Acked(b) = s.ack_alert(n, id, 160).await.unwrap() else {
        panic!("expected Acked")
    };
    assert_eq!(b.acked_at, Some(150));
    let inc = s.incident(n, a.incident_id).await.unwrap().unwrap();
    assert_eq!(
        inc.timeline
            .iter()
            .filter(|e| e.kind == TimelineKind::Acknowledged)
            .count(),
        1
    );
    s.apply_decisions(
        n,
        vec![Decision::Resolve {
            key: AlertKey::RouterDown,
            at: 200,
        }],
    )
    .await
    .unwrap();
    let AckOutcome::AlreadyResolved(r) = s.ack_alert(n, id, 210).await.unwrap() else {
        panic!("expected AlreadyResolved")
    };
    assert_eq!(r.state, AlertState::Resolved);
    assert_eq!(
        s.ack_alert(n, AlertId(999), 1).await.unwrap(),
        AckOutcome::NotFound
    );
}

#[tokio::test]
async fn list_filters_and_orders_newest_first() {
    let (s, n) = setup().await;
    s.apply_decisions(
        n,
        vec![
            open(AlertKey::LatencyHigh, false, 100),
            open(AlertKey::InternetDown, false, 110),
            open(AlertKey::NewDevice(DeviceId(2)), false, 120),
        ],
    )
    .await
    .unwrap();
    s.apply_decisions(
        n,
        vec![Decision::Resolve {
            key: AlertKey::InternetDown,
            at: 130,
        }],
    )
    .await
    .unwrap();
    let all = s.list_alerts(n, AlertFilter::All, 10).await.unwrap();
    let keys: Vec<String> = all.iter().map(|a| a.key.clone()).collect();
    assert_eq!(keys, ["new_device:2", "internet_down", "latency_high"]);
    let active = s.list_alerts(n, AlertFilter::Active, 10).await.unwrap();
    assert_eq!(active.len(), 2);
    assert!(active.iter().all(|a| a.state != AlertState::Resolved));
    assert_eq!(
        s.list_alerts(n, AlertFilter::Resolved, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.list_alerts(n, AlertFilter::Open, 10).await.unwrap().len(),
        2
    );
    assert_eq!(
        s.list_alerts(n, AlertFilter::Acked, 10)
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        s.list_alerts(n, AlertFilter::All, 1).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn active_keys_feed_resume() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.db");
    let s = Store::new(Db::open(&p).unwrap());
    let mac: MacAddr = "50:C7:BF:00:00:01".parse().unwrap();
    let n = s.network_for(mac, "192.168.1.0/24", 1).await.unwrap();
    s.apply_decisions(
        n,
        vec![
            open(AlertKey::RouterDown, false, 100),
            open(AlertKey::StarredOffline(DeviceId(3)), true, 110),
            open(AlertKey::LatencyHigh, false, 120),
        ],
    )
    .await
    .unwrap();
    s.apply_decisions(
        n,
        vec![Decision::Resolve {
            key: AlertKey::LatencyHigh,
            at: 130,
        }],
    )
    .await
    .unwrap();
    drop(s);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let s = Store::new(Db::open(&p).unwrap());
    let keys = s.active_alert_keys(n).await.unwrap();
    assert_eq!(
        keys,
        vec![
            (AlertKey::RouterDown, false),
            (AlertKey::StarredOffline(DeviceId(3)), true)
        ]
    );
}

#[tokio::test]
async fn diagnosis_lands_on_open_incidents_only() {
    let (s, n) = setup().await;
    let out = s
        .apply_decisions(
            n,
            vec![
                open(AlertKey::RouterDown, false, 100),
                open(AlertKey::LatencyHigh, false, 105),
            ],
        )
        .await
        .unwrap();
    s.apply_decisions(
        n,
        vec![Decision::Resolve {
            key: AlertKey::LatencyHigh,
            at: 110,
        }],
    )
    .await
    .unwrap();
    let ids = s
        .add_diagnosis_to_open_incidents(n, "Router is down", 120)
        .await
        .unwrap();
    assert_eq!(ids, vec![out[0].incident_id]);
    let open_inc = s.incident(n, out[0].incident_id).await.unwrap().unwrap();
    let last = open_inc.timeline.last().unwrap();
    assert_eq!(
        (last.kind, last.text.as_str()),
        (TimelineKind::Diagnosed, "Diagnosis: Router is down")
    );
    let done = s.incident(n, out[1].incident_id).await.unwrap().unwrap();
    assert!(!kinds(&done).contains(&TimelineKind::Diagnosed));
    assert_eq!(s.incident(n, IncidentId(999)).await.unwrap(), None);
}

#[test]
fn alert_json_shape() {
    let a = Alert {
        id: AlertId(1),
        rule: Rule::InternetDown,
        key: "router_down".into(),
        severity: Severity::Critical,
        title: "t".into(),
        detail: "d".into(),
        suppressed: false,
        state: AlertState::Open,
        incident_id: IncidentId(2),
        opened_at: 5,
        acked_at: None,
        resolved_at: None,
    };
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        r#"{"id":1,"rule":"internet_down","key":"router_down","severity":"critical","title":"t","detail":"d","suppressed":false,"state":"open","incident_id":2,"opened_at":5,"acked_at":null,"resolved_at":null}"#
    );
    let e = serde_json::json!(netlantern_core::TimelineEvent {
        ts: 1,
        kind: TimelineKind::LastGood,
        text: "x".into(),
        alert_id: None
    });
    assert_eq!(e["kind"], "last_good");
}

#[tokio::test]
async fn unsuppressing_a_fallback_root_keeps_its_incident() {
    let (s, n) = setup().await;
    let key = AlertKey::StarredOffline(DeviceId(4));
    let out = s
        .apply_decisions(n, vec![open(key.clone(), true, 100)])
        .await
        .unwrap();
    let id = out[0].incident_id;
    let un = s
        .apply_decisions(
            n,
            vec![Decision::Unsuppress {
                key: key.clone(),
                at: 150,
            }],
        )
        .await
        .unwrap();
    assert_eq!(un[0].incident_id, id);
    assert!(!un[0].suppressed);
    let inc = s.incident(n, id).await.unwrap().unwrap();
    assert_eq!(inc.root.id, out[0].id);
    assert_eq!(inc.timeline.last().unwrap().kind, TimelineKind::Alerted);
    assert!(!kinds(&inc).contains(&TimelineKind::Affected));
    s.apply_decisions(n, vec![Decision::Resolve { key, at: 200 }])
        .await
        .unwrap();
    let inc = s.incident(n, id).await.unwrap().unwrap();
    assert_eq!(inc.resolved_at, Some(200));
    assert_eq!(inc.timeline.last().unwrap().kind, TimelineKind::Recovered);
}

#[tokio::test]
async fn alerts_are_scoped_per_network() {
    let (s, a) = setup().await;
    let mac: MacAddr = "50:C7:BF:00:00:02".parse().unwrap();
    let b = s.network_for(mac, "10.0.0.0/24", 1).await.unwrap();
    let out = s
        .apply_decisions(a, vec![open(AlertKey::RouterDown, false, 100)])
        .await
        .unwrap();
    assert!(s
        .list_alerts(b, AlertFilter::All, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(s.incident(b, out[0].incident_id).await.unwrap().is_none());
    assert_eq!(
        s.ack_alert(b, out[0].id, 5).await.unwrap(),
        AckOutcome::NotFound
    );
    assert!(s.active_alert_keys(b).await.unwrap().is_empty());
}
