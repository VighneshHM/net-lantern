use std::net::Ipv4Addr;

use netlantern_core::{
    Check, Db, DeviceId, DevicePatch, DeviceType, EndReason, Metric, MonitorConfig, Resolution,
    RuleSettings, Sample, ScanMode, ScanState, Sighting, Signals, Store, StoreError, Target,
};
use netlantern_platform::{MacAddr, MdnsService, NetbiosInfo};

const T0: i64 = 1_000_000;
const T1: i64 = 2_000_000;
const MAC_A: &str = "00:11:32:AB:CD:01";
const MAC_B: &str = "00:11:32:AB:CD:02";
const GW_HOME: &str = "50:C7:BF:00:00:01";
const GW_CAFE: &str = "50:C7:BF:00:00:02";

fn mac(s: &str) -> MacAddr {
    s.parse().unwrap()
}

struct B(Sighting);

fn sighting(ip: &str, m: &str) -> B {
    let ip: Ipv4Addr = if let Some(n) = ip.strip_prefix('.') {
        format!("192.168.1.{n}").parse().unwrap()
    } else {
        ip.parse().unwrap()
    };
    B(Sighting {
        ip,
        mac: Some(mac(m)),
        answers_ping: true,
        rtt_ms: Some(3),
        ttl: Some(64),
        is_self: false,
        is_gateway: false,
        open_ports: vec![],
        signals: Signals::default(),
    })
}

impl B {
    fn netbios(mut self, name: &str) -> Sighting {
        self.0.signals.netbios = Some(NetbiosInfo {
            name: name.into(),
            workgroup: None,
        });
        self.0
    }
    fn rdns(mut self, name: &str) -> Sighting {
        self.0.signals.rdns = Some(name.into());
        self.0
    }
    fn mdns_host(mut self, host: &str) -> Sighting {
        self.0.signals.mdns.push(MdnsService {
            ip: self.0.ip,
            service_type: "_device-info._tcp".into(),
            instance: "x".into(),
            hostname: Some(host.into()),
            friendly_name: None,
            model: None,
        });
        self.0
    }
    fn ports(mut self, p: &[u16]) -> Sighting {
        self.0.open_ports = p.to_vec();
        self.0
    }
}

impl From<B> for Sighting {
    fn from(b: B) -> Sighting {
        b.0
    }
}

fn patch_name(n: &str) -> DevicePatch {
    DevicePatch {
        name: Some(Some(n.into())),
        ..Default::default()
    }
}

struct Fx {
    db: Db,
    store: Store,
    net: netlantern_core::NetworkId,
    scan1: netlantern_core::ScanId,
    scan2: netlantern_core::ScanId,
}

async fn fx() -> Fx {
    let db = Db::open_in_memory().unwrap();
    let store = Store::new(db.clone());
    let net = store
        .network_for(mac(GW_HOME), "192.168.1.0/24", T0)
        .await
        .unwrap();
    let scan1 = store.insert_scan(ScanMode::Quick, T0).await.unwrap();
    store.set_scan_network(scan1, net).await.unwrap();
    let scan2 = store.insert_scan(ScanMode::Quick, T1).await.unwrap();
    store.set_scan_network(scan2, net).await.unwrap();
    Fx {
        db,
        store,
        net,
        scan1,
        scan2,
    }
}

#[tokio::test]
async fn first_sighting_creates_a_new_identified_device() {
    let f = fx().await;
    let d = f
        .store
        .record_sighting(
            f.net,
            f.scan1,
            sighting("192.168.1.70", "00:11:32:AB:CD:70").netbios("DISKSTATION"),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(
        (
            d.vendor.as_deref(),
            d.hostname.as_deref(),
            d.device_type,
            d.is_new
        ),
        (Some("Synology"), Some("DISKSTATION"), DeviceType::Nas, true)
    );
    assert_eq!(d.display_name, "DISKSTATION");
    assert!(d.online);
    assert_eq!(d.services[0].source, "netbios");
}

#[tokio::test]
async fn name_follows_the_mac_to_a_new_ip_and_old_ip_owner_goes_offline() {
    let f = fx().await;
    let s = &f.store;
    let a = s
        .record_sighting(f.net, f.scan1, sighting(".50", MAC_A).into(), T0)
        .await
        .unwrap();
    s.update_device(f.net, a.id, patch_name("Desk plug"))
        .await
        .unwrap();
    s.finish_scan(f.scan1, ScanState::Done, None, false, 1, T0)
        .await
        .unwrap();
    let b = s
        .record_sighting(f.net, f.scan2, sighting(".50", MAC_B).into(), T1)
        .await
        .unwrap();
    let a2 = s
        .record_sighting(f.net, f.scan2, sighting(".51", MAC_A).into(), T1)
        .await
        .unwrap();
    s.finish_scan(f.scan2, ScanState::Done, None, false, 2, T1)
        .await
        .unwrap();
    assert_eq!((a2.id, a2.name.as_deref()), (a.id, Some("Desk plug")));
    assert_eq!(a2.ip, Some("192.168.1.51".parse().unwrap()));
    assert_eq!(s.list_devices(f.net).await.unwrap().len(), 2);
    assert_ne!(b.id, a.id);
}

#[tokio::test]
async fn a_device_missing_from_the_newest_done_scan_is_offline() {
    let f = fx().await;
    let s = &f.store;
    let a = s
        .record_sighting(f.net, f.scan1, sighting(".50", MAC_A).into(), T0)
        .await
        .unwrap();
    s.record_sighting(f.net, f.scan1, sighting(".51", MAC_B).into(), T0)
        .await
        .unwrap();
    s.finish_scan(f.scan1, ScanState::Done, None, false, 2, T0)
        .await
        .unwrap();
    // scan2 is running and has seen only B so far: A is still online (from scan1).
    s.record_sighting(f.net, f.scan2, sighting(".51", MAC_B).into(), T1)
        .await
        .unwrap();
    assert!(s.get_device(f.net, a.id).await.unwrap().unwrap().online);
    s.finish_scan(f.scan2, ScanState::Done, None, false, 1, T1)
        .await
        .unwrap();
    let list = s.list_devices(f.net).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].online && !list[1].online);
    assert_eq!(list[1].id, a.id);
}

#[tokio::test]
async fn private_mac_rotation_keeps_the_device() {
    let f = fx().await;
    let s = &f.store;
    let p = s
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".44", "DA:A1:19:5C:2E:01").rdns("Pixel-7.lan"),
            T0,
        )
        .await
        .unwrap();
    assert!(p.private_mac);
    let again = s
        .record_sighting(
            f.net,
            f.scan2,
            sighting(".45", "DE:11:22:33:44:55").rdns("Pixel-7.lan"),
            T1,
        )
        .await
        .unwrap();
    assert_eq!(again.id, p.id);
    assert_eq!(again.mac, Some(mac("DE:11:22:33:44:55")));
    assert_eq!(s.list_devices(f.net).await.unwrap().len(), 1);
}

#[tokio::test]
async fn two_phones_with_the_same_name_in_one_scan_stay_apart() {
    let f = fx().await;
    let s = &f.store;
    let a = s
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".44", "DA:00:00:00:00:01").mdns_host("iPhone.local"),
            T0,
        )
        .await
        .unwrap();
    let b = s
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".45", "DA:00:00:00:00:02").mdns_host("iPhone.local"),
            T0,
        )
        .await
        .unwrap();
    assert_ne!(a.id, b.id);
}

#[tokio::test]
async fn devices_are_scoped_per_network() {
    let f = fx().await;
    let s = &f.store;
    let cafe = s
        .network_for(mac(GW_CAFE), "10.0.0.0/24", T0)
        .await
        .unwrap();
    s.record_sighting(f.net, f.scan1, sighting("192.168.1.70", MAC_A).into(), T0)
        .await
        .unwrap();
    assert!(s.list_devices(cafe).await.unwrap().is_empty());
    assert_eq!(
        s.network_for(mac(GW_HOME), "192.168.1.0/24", T1)
            .await
            .unwrap(),
        f.net
    );
    assert_eq!(
        s.find_network(mac(GW_CAFE), "10.0.0.0/24").await.unwrap(),
        Some(cafe)
    );
    assert_eq!(
        s.find_network(mac(GW_CAFE), "10.9.9.0/24").await.unwrap(),
        None
    );
}

#[tokio::test]
async fn evidence_accumulates_across_sightings() {
    let f = fx().await;
    let s = &f.store;
    let first = s
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".70", MAC_A).netbios("DISKSTATION"),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(first.device_type, DeviceType::Nas);
    let second = s
        .record_sighting(f.net, f.scan2, sighting(".70", MAC_A).ports(&[445, 22]), T1)
        .await
        .unwrap();
    assert_eq!(second.hostname.as_deref(), Some("DISKSTATION"));
    assert_eq!(second.open_ports, vec![22, 445]);
    assert_eq!(second.first_seen, T0);
    assert_eq!(second.last_seen, T1);
    assert_eq!(second.id, first.id);
}

#[tokio::test]
async fn clearing_the_name_returns_to_the_default() {
    let f = fx().await;
    let s = &f.store;
    let d = s
        .record_sighting(f.net, f.scan1, sighting(".70", MAC_A).netbios("NAS1"), T0)
        .await
        .unwrap();
    let named = s
        .update_device(f.net, d.id, patch_name("  Kitchen NAS "))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(named.display_name, "Kitchen NAS");
    let cleared = s
        .update_device(
            f.net,
            d.id,
            DevicePatch {
                name: Some(Some("   ".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cleared.name, None);
    assert_eq!(cleared.display_name, "NAS1");
}

#[tokio::test]
async fn naming_clears_new() {
    let f = fx().await;
    let s = &f.store;
    let d = s
        .record_sighting(f.net, f.scan1, sighting(".70", MAC_A).into(), T0)
        .await
        .unwrap();
    assert!(d.is_new);
    let d = s
        .update_device(f.net, d.id, patch_name("Plug"))
        .await
        .unwrap()
        .unwrap();
    assert!(!d.is_new);
    let d = s
        .update_device(
            f.net,
            d.id,
            DevicePatch {
                starred: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert!(d.starred);
    // Wrong network: not found.
    let other = s
        .network_for(mac(GW_CAFE), "10.0.0.0/24", T0)
        .await
        .unwrap();
    assert!(s
        .update_device(other, d.id, patch_name("x"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn list_orders_online_first_then_by_ip_with_no_ip_last() {
    let f = fx().await;
    let s = &f.store;
    for (ip, m) in [(".60", MAC_A), (".9", MAC_B)] {
        s.record_sighting(f.net, f.scan1, sighting(ip, m).into(), T0)
            .await
            .unwrap();
    }
    let ips: Vec<_> = s
        .list_devices(f.net)
        .await
        .unwrap()
        .iter()
        .map(|d| d.ip.unwrap().to_string())
        .collect();
    assert_eq!(ips, ["192.168.1.9", "192.168.1.60"]);
}

#[tokio::test]
async fn scans_round_trip() {
    let f = fx().await;
    let s = &f.store;
    s.finish_scan(f.scan1, ScanState::Failed, Some("boom".into()), true, 4, T1)
        .await
        .unwrap();
    let snap = s.scan(f.scan1).await.unwrap().unwrap();
    assert_eq!((snap.state, snap.devices_found), (ScanState::Failed, 4));
    assert_eq!((snap.error.as_deref(), snap.isolated), (Some("boom"), true));
    assert_eq!(s.latest_scan().await.unwrap().unwrap().id, f.scan2);
    assert!(s.scan(netlantern_core::ScanId(99)).await.unwrap().is_none());
}

#[test]
fn refuses_a_newer_schema() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("n.db");
    drop(Db::open(&p).unwrap());
    rusqlite::Connection::open(&p)
        .unwrap()
        .execute_batch("PRAGMA user_version = 99")
        .unwrap();
    match Db::open(&p) {
        Err(StoreError::NewerSchema { found: 99, .. }) => {}
        other => panic!("expected NewerSchema, got {:?}", other.err()),
    }
}

#[tokio::test]
async fn interrupted_scans_are_marked_failed_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("i.db");
    let store = Store::new(Db::open(&p).unwrap());
    let id = store.insert_scan(ScanMode::Quick, T0).await.unwrap();
    drop(store);
    // Let the writer thread stop and release the file.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let store = Store::new(Db::open(&p).unwrap());
    let snap = store.scan(id).await.unwrap().unwrap();
    assert_eq!(snap.state, ScanState::Failed);
    assert_eq!(
        snap.error.as_deref(),
        Some("Net Lantern stopped during the scan.")
    );
}

fn mdns_named(mut b: B, name: &str, ip: &str) -> Sighting {
    b.0.signals.mdns.push(MdnsService {
        ip: ip.parse().unwrap(),
        service_type: "_http._tcp".into(),
        instance: "web".into(),
        hostname: None,
        friendly_name: Some(name.into()),
        model: None,
    });
    b.0
}

#[tokio::test]
async fn hostname_follows_a_changed_mdns_name_and_ip() {
    let f = fx().await;
    let s = &f.store;
    let d = s
        .record_sighting(
            f.net,
            f.scan1,
            mdns_named(sighting(".70", MAC_A), "Old name", "192.168.1.70"),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(d.hostname.as_deref(), Some("Old name"));
    let d2 = s
        .record_sighting(
            f.net,
            f.scan2,
            mdns_named(sighting(".71", MAC_A), "New name", "192.168.1.71"),
            T1,
        )
        .await
        .unwrap();
    assert_eq!(d2.id, d.id);
    assert_eq!(d2.hostname.as_deref(), Some("New name"));
}

#[tokio::test]
async fn a_name_shared_by_two_stored_devices_never_adopts_a_new_private_mac() {
    let f = fx().await;
    let s = &f.store;
    let a = s
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".44", "DA:00:00:00:00:01").netbios("iPhone"),
            T0,
        )
        .await
        .unwrap();
    let b = s
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".45", "DA:00:00:00:00:02").netbios("iPhone2"),
            T0,
        )
        .await
        .unwrap();
    // Make B's stored name equal A's, as two same-named phones would be.
    s.record_sighting(
        f.net,
        f.scan1,
        sighting(".45", "DA:00:00:00:00:02").netbios("iPhone"),
        T0,
    )
    .await
    .unwrap();
    // Scan 2: A is seen first with its known MAC, then a guest with a new private MAC.
    s.record_sighting(
        f.net,
        f.scan2,
        sighting(".44", "DA:00:00:00:00:01").netbios("iPhone"),
        T1,
    )
    .await
    .unwrap();
    let guest = s
        .record_sighting(
            f.net,
            f.scan2,
            sighting(".46", "DA:00:00:00:00:09").netbios("iPhone"),
            T1,
        )
        .await
        .unwrap();
    assert_ne!(guest.id, a.id);
    assert_ne!(guest.id, b.id);
}

#[tokio::test]
async fn mac_known_is_per_network() {
    let f = fx().await;
    let s = &f.store;
    let cafe = s
        .network_for(mac(GW_CAFE), "10.0.0.0/24", T0)
        .await
        .unwrap();
    assert!(!s.mac_known(f.net, mac(MAC_A)).await.unwrap());
    s.record_sighting(f.net, f.scan1, sighting(".70", MAC_A).into(), T0)
        .await
        .unwrap();
    assert!(s.mac_known(f.net, mac(MAC_A)).await.unwrap());
    assert!(!s.mac_known(f.net, mac(MAC_B)).await.unwrap());
    assert!(!s.mac_known(cafe, mac(MAC_A)).await.unwrap());
}

#[tokio::test]
async fn latest_scan_for_is_per_network() {
    let f = fx().await;
    let s = &f.store;
    let cafe = s
        .network_for(mac(GW_CAFE), "10.0.0.0/24", T0)
        .await
        .unwrap();
    assert_eq!(s.latest_scan_for(f.net).await.unwrap().unwrap().id, f.scan2);
    assert_eq!(s.latest_scan_for(cafe).await.unwrap(), None);
    // A newer scan elsewhere, or one with no network yet, is not home's latest.
    let there = s.insert_scan(ScanMode::Quick, T1).await.unwrap();
    s.set_scan_network(there, cafe).await.unwrap();
    s.insert_scan(ScanMode::Quick, T1).await.unwrap();
    assert_eq!(s.latest_scan_for(f.net).await.unwrap().unwrap().id, f.scan2);
    assert_eq!(s.latest_scan_for(cafe).await.unwrap().unwrap().id, there);
}

fn rtt(ts: i64, v: Option<f64>) -> Sample {
    Sample {
        target: Target::Gateway,
        metric: Metric::RttMs,
        ts,
        value: v,
    }
}

async fn count(db: &Db, sql: &'static str) -> i64 {
    db.call(move |c| c.query_row(sql, [], |r| r.get(0)))
        .await
        .unwrap()
}

async fn net_with_db() -> (Db, Store, netlantern_core::NetworkId) {
    let db = Db::open_in_memory().unwrap();
    let store = Store::new(db.clone());
    let net = store
        .network_for(mac(GW_HOME), "192.168.1.0/24", T0)
        .await
        .unwrap();
    (db, store, net)
}

#[tokio::test]
async fn a_v1_database_upgrades_to_v2_with_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("v1.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(
            "CREATE TABLE networks(id INTEGER PRIMARY KEY, gateway_mac TEXT NOT NULL, subnet TEXT NOT NULL, ssid_label TEXT,
                first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, scan_consent INTEGER NOT NULL DEFAULT 0, UNIQUE(gateway_mac, subnet));
             CREATE TABLE devices(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id), last_ip TEXT,
                name TEXT, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL);
             CREATE TABLE scans(id INTEGER PRIMARY KEY, network_id INTEGER, mode TEXT NOT NULL, state TEXT NOT NULL, error TEXT,
                isolated INTEGER NOT NULL DEFAULT 0, started_at INTEGER NOT NULL, finished_at INTEGER, devices_found INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE device_macs(device_id INTEGER NOT NULL, mac TEXT NOT NULL, private_mac INTEGER NOT NULL,
                first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, PRIMARY KEY(device_id, mac));
             CREATE TABLE device_services(device_id INTEGER NOT NULL, source TEXT NOT NULL, value TEXT NOT NULL,
                last_seen INTEGER NOT NULL, PRIMARY KEY(device_id, source, value));
             CREATE TABLE device_ports(device_id INTEGER NOT NULL, port INTEGER NOT NULL, proto TEXT NOT NULL DEFAULT 'tcp',
                service TEXT, banner TEXT, seen_at INTEGER NOT NULL, PRIMARY KEY(device_id, port, proto));
             INSERT INTO networks(id, gateway_mac, subnet, first_seen, last_seen) VALUES(1, 'aa', '10.0.0.0/24', 1, 1);
             INSERT INTO devices(id, network_id, last_ip, name, first_seen, last_seen) VALUES(7, 1, '10.0.0.7', 'kept', 1, 1);
             PRAGMA user_version = 1;",
        )
        .unwrap();
    }
    let db = Db::open(&p).unwrap();
    assert_eq!(count(&db, "PRAGMA user_version").await, 2);
    assert!(dir.path().join("v1.db.bak").exists());
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM devices WHERE name = 'kept' AND presence IS NULL"
        )
        .await,
        1
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples").await, 0);
}

/// The Phase 1 schema as it was written, so upgrades run on a real v1 file.
const V1_SCHEMA: &str = "
CREATE TABLE networks(
    id INTEGER PRIMARY KEY, gateway_mac TEXT NOT NULL, subnet TEXT NOT NULL, ssid_label TEXT,
    first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, scan_consent INTEGER NOT NULL DEFAULT 0,
    UNIQUE(gateway_mac, subnet));
CREATE TABLE devices(
    id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id), last_ip TEXT,
    hostname TEXT, vendor TEXT, type TEXT NOT NULL DEFAULT 'unknown', type_confidence REAL NOT NULL DEFAULT 0,
    name TEXT, starred INTEGER NOT NULL DEFAULT 0, is_new INTEGER NOT NULL DEFAULT 1,
    answers_ping INTEGER NOT NULL DEFAULT 0, rtt_ms INTEGER, ttl INTEGER, is_self INTEGER NOT NULL DEFAULT 0,
    is_gateway INTEGER NOT NULL DEFAULT 0, last_scan_id INTEGER, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
    evidence TEXT);
CREATE TABLE device_macs(
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, mac TEXT NOT NULL,
    private_mac INTEGER NOT NULL, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL,
    PRIMARY KEY(device_id, mac));
CREATE TABLE device_services(
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, source TEXT NOT NULL,
    value TEXT NOT NULL, last_seen INTEGER NOT NULL, PRIMARY KEY(device_id, source, value));
CREATE TABLE device_ports(
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, port INTEGER NOT NULL,
    proto TEXT NOT NULL DEFAULT 'tcp', service TEXT, banner TEXT, seen_at INTEGER NOT NULL,
    PRIMARY KEY(device_id, port, proto));
CREATE TABLE scans(
    id INTEGER PRIMARY KEY, network_id INTEGER REFERENCES networks(id), mode TEXT NOT NULL, state TEXT NOT NULL,
    error TEXT, isolated INTEGER NOT NULL DEFAULT 0, started_at INTEGER NOT NULL, finished_at INTEGER,
    devices_found INTEGER NOT NULL DEFAULT 0);
CREATE TABLE settings(
    scope TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(scope, key));
";

#[tokio::test]
async fn upgrading_to_v2_moves_second_timestamps_to_milliseconds() {
    const S1: i64 = 1_762_000_000; // seconds, as Phase 1 wrote them
    const S2: i64 = 1_762_000_500;
    const MS: i64 = 1_762_000_900_000; // already milliseconds
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("v1-seconds.db");
    {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(V1_SCHEMA).unwrap();
        c.execute_batch(&format!(
            "INSERT INTO networks(id, gateway_mac, subnet, first_seen, last_seen) VALUES(1, 'aa', '10.0.0.0/24', {S1}, {S2});
             INSERT INTO devices(id, network_id, first_seen, last_seen) VALUES(7, 1, {S1}, {S2});
             INSERT INTO devices(id, network_id, first_seen, last_seen) VALUES(8, 1, {MS}, {MS});
             INSERT INTO device_macs(device_id, mac, private_mac, first_seen, last_seen) VALUES(7, 'AA:BB:CC:DD:EE:FF', 0, {S1}, {S2});
             INSERT INTO device_services(device_id, source, value, last_seen) VALUES(7, 'mdns', 'x', {S2});
             INSERT INTO device_ports(device_id, port, seen_at) VALUES(7, 80, {S2});
             INSERT INTO scans(id, network_id, mode, state, started_at, finished_at) VALUES(1, 1, 'quick', 'done', {S1}, {S2});
             INSERT INTO scans(id, network_id, mode, state, started_at, finished_at) VALUES(2, 1, 'quick', 'failed', {S2}, NULL);
             PRAGMA user_version = 1;"
        ))
        .unwrap();
    }
    let db = Db::open(&p).unwrap();
    let all: Vec<i64> = db
        .call(|c| {
            c.query_row(
                "SELECT n.first_seen, n.last_seen, d.first_seen, d.last_seen, m.first_seen, m.last_seen,
                        s.last_seen, p.seen_at, sc.started_at, sc.finished_at
                 FROM networks n, devices d, device_macs m, device_services s, device_ports p, scans sc
                 WHERE n.id = 1 AND d.id = 7 AND m.device_id = 7 AND s.device_id = 7 AND p.device_id = 7 AND sc.id = 1",
                [],
                |r| (0..10).map(|i| r.get(i)).collect(),
            )
        })
        .await
        .unwrap();
    let (a, b) = (S1 * 1000, S2 * 1000);
    assert_eq!(all, [a, b, a, b, a, b, b, b, a, b]);
    assert!(all[0] < all[1], "ordering is kept");
    assert_eq!(
        count(&db, "SELECT first_seen FROM devices WHERE id = 8").await,
        MS
    );
    assert_eq!(
        count(&db, "SELECT started_at FROM scans WHERE id = 2").await,
        S2 * 1000
    );
    assert_eq!(
        count(
            &db,
            "SELECT COUNT(*) FROM scans WHERE id = 2 AND finished_at IS NULL"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn rollup_folds_finished_buckets_only_and_is_idempotent() {
    let (db, store, net) = net_with_db().await;
    store
        .insert_samples(
            net,
            vec![
                rtt(0, Some(10.0)),
                rtt(60_000, None),
                rtt(120_000, Some(30.0)),
                rtt(299_000, None),
                rtt(300_000, Some(99.0)),
            ],
        )
        .await
        .unwrap();
    store.rollup(301_000).await.unwrap();
    store.rollup(301_000).await.unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples_rollup").await, 1);
    let row: (i64, f64, f64, f64, i64, i64) = db
        .call(|c| {
            c.query_row(
                "SELECT bucket_start, min, avg, max, count, lost FROM samples_rollup",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
        })
        .await
        .unwrap();
    assert_eq!(row, (0, 10.0, 20.0, 30.0, 4, 2));
}

#[tokio::test]
async fn metrics_use_raw_points_up_to_a_day_and_rollups_beyond() {
    let (_db, store, net) = net_with_db().await;
    let day = 86_400_000;
    let now = 7 * day;
    store
        .insert_samples(
            net,
            vec![
                rtt(now - 3_000_000, Some(5.0)),
                rtt(now - 2_000_000, None),
                rtt(now - 1_000_000, Some(7.0)),
            ],
        )
        .await
        .unwrap();
    store.rollup(now).await.unwrap();
    let hour = store
        .metrics(net, Target::Gateway, Metric::RttMs, now - 3_600_000, now)
        .await
        .unwrap();
    assert_eq!(hour.resolution, Resolution::Raw);
    assert_eq!(hour.points.len(), 3);
    assert_eq!(hour.points[0].avg, Some(5.0));
    let p = &hour.points[1];
    assert_eq!((p.avg, p.lost, p.count), (None, 1, 1));
    let week = store
        .metrics(net, Target::Gateway, Metric::RttMs, now - 7 * day, now)
        .await
        .unwrap();
    assert_eq!(week.resolution, Resolution::FiveMin);
    assert_eq!(week.points.iter().map(|p| p.count).sum::<u32>(), 3);
    assert_eq!(week.points.iter().map(|p| p.lost).sum::<u32>(), 1);
}

#[tokio::test]
async fn metrics_list_the_sessions_in_range() {
    let (_db, store, net) = net_with_db().await;
    let a = store.open_session(net, 1_000).await.unwrap();
    store.end_session(a, 2_000, EndReason::Sleep).await.unwrap();
    let b = store.open_session(net, 5_000).await.unwrap();
    store.touch_session(b, 6_000).await.unwrap();
    let m = store
        .metrics(net, Target::Gateway, Metric::RttMs, 0, 10_000)
        .await
        .unwrap();
    assert_eq!(m.sessions.len(), 2);
    assert_eq!(m.sessions[0].started_at, 1_000);
    assert_eq!(m.sessions[0].ended_at, Some(2_000));
    assert_eq!(m.sessions[0].end_reason, Some(EndReason::Sleep));
    assert_eq!(m.sessions[1].started_at, 5_000);
    assert_eq!(m.sessions[1].ended_at, None);
    assert_eq!(m.sessions[1].end_reason, None);
}

#[tokio::test]
async fn prune_keeps_what_retention_says() {
    let (db, store, net) = net_with_db().await;
    let h = 3_600_000;
    let day = 24 * h;
    let now = 200 * day;
    store
        .insert_samples(
            net,
            vec![rtt(now - 25 * h, Some(1.0)), rtt(now - 23 * h, Some(2.0))],
        )
        .await
        .unwrap();
    db.call(move |c| {
        c.execute(
            "INSERT INTO devices(id, network_id, first_seen, last_seen) VALUES(1, ?1, 0, 0)",
            [net.0],
        )?;
        c.execute(
            "INSERT INTO samples_rollup VALUES(?1,'gateway','rtt_ms',?2,1,1,1,1,0), (?1,'gateway','rtt_ms',?3,1,1,1,1,0)",
            rusqlite::params![net.0, now - 31 * day, now - 29 * day],
        )?;
        c.execute(
            "INSERT INTO presence VALUES(1,?1,1), (1,?2,0)",
            rusqlite::params![now - 91 * day, now - 89 * day],
        )?;
        c.execute(
            "INSERT INTO monitor_sessions(network_id, started_at, last_tick_at, ended_at, end_reason) VALUES(?1,1,1,2,'quit')",
            [net.0],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    store
        .prune(now, &MonitorConfig::default().retention)
        .await
        .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples").await, 1);
    assert_eq!(
        count(&db, "SELECT CAST(value AS INTEGER) FROM samples").await,
        2
    );
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples_rollup").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM presence").await, 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM monitor_sessions").await, 0);
}

#[tokio::test]
async fn a_session_left_open_is_closed_as_a_crash_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.db");
    let store = Store::new(Db::open(&p).unwrap());
    let net = store
        .network_for(mac(GW_HOME), "192.168.1.0/24", T0)
        .await
        .unwrap();
    let id = store.open_session(net, 1_000).await.unwrap();
    store.touch_session(id, 5_000).await.unwrap();
    drop(store);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let store = Store::new(Db::open(&p).unwrap());
    let m = store
        .metrics(net, Target::Gateway, Metric::RttMs, 0, 10_000)
        .await
        .unwrap();
    assert_eq!(m.sessions.len(), 1);
    assert_eq!(m.sessions[0].end_reason, Some(EndReason::Crash));
    assert_eq!(m.sessions[0].ended_at, Some(5_000));
}

#[test]
fn targets_round_trip_as_text() {
    assert_eq!(
        "device:12".parse::<Target>(),
        Ok(Target::Device(DeviceId(12)))
    );
    assert_eq!(Target::Device(DeviceId(12)).to_string(), "device:12");
    for (s, t) in [
        ("gateway", Target::Gateway),
        ("internet", Target::Internet),
        ("dns", Target::Dns),
        ("wifi", Target::Wifi),
    ] {
        assert_eq!(s.parse::<Target>(), Ok(t.clone()));
        assert_eq!(t.to_string(), s);
    }
    assert!("bogus".parse::<Target>().is_err());
    assert!("device:x".parse::<Target>().is_err());
    assert!("device:-5".parse::<Target>().is_err());
    assert!("device:+5".parse::<Target>().is_err());
    assert!("device:".parse::<Target>().is_err());
    assert_eq!(
        serde_json::to_string(&Target::Device(DeviceId(3))).unwrap(),
        "\"device:3\""
    );
}

#[test]
fn scaled_config_divides_durations_but_not_retention() {
    let d = MonitorConfig::default();
    let s = d.scaled(60);
    assert_eq!(s.tick, std::time::Duration::from_millis(500));
    assert_eq!(s.offline_grace, std::time::Duration::from_secs(2));
    assert_eq!(s.retention, d.retention);
    assert!(s.arp_timeout >= std::time::Duration::from_millis(10));
}

#[tokio::test]
async fn rollup_after_prune_does_not_shrink_a_bucket() {
    let (db, store, net) = net_with_db().await;
    let day = 86_400_000;
    let now = 200 * day + 150_000;
    let bucket = 199 * day;
    store
        .insert_samples(
            net,
            vec![
                rtt(bucket + 10_000, Some(10.0)),
                rtt(bucket + 200_000, Some(30.0)),
            ],
        )
        .await
        .unwrap();
    store.rollup(now).await.unwrap();
    store
        .prune(now, &MonitorConfig::default().retention)
        .await
        .unwrap();
    store.rollup(now).await.unwrap();
    let row: (i64, f64, f64, f64) = db
        .call(|c| {
            c.query_row("SELECT count, min, avg, max FROM samples_rollup", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
        })
        .await
        .unwrap();
    assert_eq!(row, (2, 10.0, 20.0, 30.0));
    assert_eq!(count(&db, "SELECT COUNT(*) FROM samples").await, 2);
}

#[tokio::test]
async fn long_range_metrics_include_the_bucket_holding_an_unaligned_from() {
    let (_db, store, net) = net_with_db().await;
    let day = 86_400_000;
    store
        .insert_samples(net, vec![rtt(50_000, Some(4.0))])
        .await
        .unwrap();
    store.rollup(10 * 300_000).await.unwrap();
    let m = store
        .metrics(
            net,
            Target::Gateway,
            Metric::RttMs,
            100_000,
            100_000 + 2 * day,
        )
        .await
        .unwrap();
    assert_eq!(m.resolution, Resolution::FiveMin);
    assert_eq!(m.points.len(), 1);
    assert_eq!(m.points[0].ts, 0);
}

async fn presence_rows(db: &Db) -> i64 {
    count(db, "SELECT COUNT(*) FROM presence").await
}

#[tokio::test]
async fn presence_overrides_the_scan_rule_once_set() {
    let f = fx().await;
    let d = f
        .store
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".70", "00:11:32:AB:CD:70").into(),
            T0,
        )
        .await
        .unwrap();
    f.store
        .finish_scan(f.scan1, ScanState::Done, None, false, 1, T0)
        .await
        .unwrap();
    let id = d.id;
    assert!(d.online);
    assert_eq!(d.presence_changed_at, None);
    assert_eq!(presence_rows(&f.db).await, 0);
    assert!(f
        .store
        .set_presence(f.net, id, false, T0 + 10)
        .await
        .unwrap());
    let got = f.store.get_device(f.net, id).await.unwrap().unwrap();
    assert!(!got.online);
    assert_eq!(got.presence_changed_at, Some(T0 + 10));
    assert_eq!(presence_rows(&f.db).await, 1);
    assert!(!f
        .store
        .set_presence(f.net, id, false, T0 + 20)
        .await
        .unwrap());
    assert_eq!(presence_rows(&f.db).await, 1);
    let got = f.store.get_device(f.net, id).await.unwrap().unwrap();
    assert_eq!(got.presence_changed_at, Some(T0 + 10));
}

#[tokio::test]
async fn a_new_sighting_turns_an_offline_device_online() {
    let f = fx().await;
    let d = f
        .store
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".70", "00:11:32:AB:CD:70").into(),
            T0,
        )
        .await
        .unwrap();
    f.store
        .set_presence(f.net, d.id, false, T0 + 5)
        .await
        .unwrap();
    let again = f
        .store
        .record_sighting(
            f.net,
            f.scan2,
            sighting(".70", "00:11:32:AB:CD:70").into(),
            T1,
        )
        .await
        .unwrap();
    assert!(again.online);
    assert_eq!(again.presence_changed_at, Some(T1));
}

#[tokio::test]
async fn an_unknown_mac_in_presence_creates_a_new_device() {
    let f = fx().await;
    let p = f
        .store
        .record_presence_sighting(
            f.net,
            "192.168.1.88".parse().unwrap(),
            mac("00:11:32:AB:CD:88"),
            T1,
        )
        .await
        .unwrap();
    assert!(p.created);
    assert!(p.device.is_new);
    assert!(p.device.online);
    assert_eq!(p.device.vendor.as_deref(), Some("Synology"));
    assert_eq!(p.device.ip, Some("192.168.1.88".parse().unwrap()));
    assert_eq!(p.device.presence_changed_at, Some(T1));
    let again = f
        .store
        .record_presence_sighting(
            f.net,
            "192.168.1.88".parse().unwrap(),
            mac("00:11:32:AB:CD:88"),
            T1 + 1,
        )
        .await
        .unwrap();
    assert!(!again.created);
    assert_eq!(again.device.id, p.device.id);
}

#[tokio::test]
async fn a_known_mac_at_a_new_ip_moves_the_device() {
    let f = fx().await;
    let d = f
        .store
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".70", "00:11:32:AB:CD:70").into(),
            T0,
        )
        .await
        .unwrap();
    let p = f
        .store
        .record_presence_sighting(
            f.net,
            "192.168.1.99".parse().unwrap(),
            mac("00:11:32:AB:CD:70"),
            T1,
        )
        .await
        .unwrap();
    assert!(!p.created);
    assert_eq!(p.device.id, d.id);
    assert_eq!(p.device.ip, Some("192.168.1.99".parse().unwrap()));
    assert_eq!(p.device.last_seen, T1);
}

#[tokio::test]
async fn checks_round_trip_in_order() {
    let f = fx().await;
    let d = f
        .store
        .record_sighting(
            f.net,
            f.scan1,
            sighting(".70", "00:11:32:AB:CD:70").into(),
            T0,
        )
        .await
        .unwrap();
    let checks = vec![
        Check::Tcp { port: 80 },
        Check::Http {
            port: 8080,
            path: "/status".into(),
            https: false,
        },
    ];
    let got = f
        .store
        .set_checks(f.net, d.id, checks.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.checks, checks);
    let again = f.store.get_device(f.net, d.id).await.unwrap().unwrap();
    assert_eq!(again.checks, checks);
    let json = serde_json::to_value(&again.checks[0]).unwrap();
    assert_eq!(json, serde_json::json!({"kind": "tcp", "port": 80}));
    // Replacing shrinks the list.
    let one = f
        .store
        .set_checks(f.net, d.id, vec![Check::Tcp { port: 22 }])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(one.checks, vec![Check::Tcp { port: 22 }]);
    assert!(f
        .store
        .set_checks(f.net, DeviceId(9999), vec![])
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn rule_settings_default_on_and_persist_per_network() {
    let (_db, store, a) = net_with_db().await;
    let b = store
        .network_for(mac(GW_CAFE), "10.0.0.0/24", T0)
        .await
        .unwrap();
    assert_eq!(
        store.rule_settings(a).await.unwrap(),
        RuleSettings::default()
    );
    let off = RuleSettings {
        latency_high: false,
        notifications: false,
        ..RuleSettings::default()
    };
    store.set_rule_settings(a, off).await.unwrap();
    assert_eq!(store.rule_settings(a).await.unwrap(), off);
    assert_eq!(
        store.rule_settings(b).await.unwrap(),
        RuleSettings::default()
    );
    store
        .set_rule_settings(a, RuleSettings::default())
        .await
        .unwrap();
    assert_eq!(
        store.rule_settings(a).await.unwrap(),
        RuleSettings::default()
    );
}

#[tokio::test]
async fn first_scan_finished_at_is_the_oldest_done_scan() {
    let (_db, store, net) = net_with_db().await;
    assert_eq!(store.first_scan_finished_at(net).await.unwrap(), None);
    let failed = store.insert_scan(ScanMode::Quick, 10).await.unwrap();
    store.set_scan_network(failed, net).await.unwrap();
    store
        .finish_scan(failed, ScanState::Failed, Some("x".into()), false, 0, 50)
        .await
        .unwrap();
    let s1 = store.insert_scan(ScanMode::Quick, 60).await.unwrap();
    store.set_scan_network(s1, net).await.unwrap();
    store
        .finish_scan(s1, ScanState::Done, None, false, 0, 100)
        .await
        .unwrap();
    let s2 = store.insert_scan(ScanMode::Quick, 150).await.unwrap();
    store.set_scan_network(s2, net).await.unwrap();
    store
        .finish_scan(s2, ScanState::Done, None, false, 0, 200)
        .await
        .unwrap();
    assert_eq!(store.first_scan_finished_at(net).await.unwrap(), Some(100));
}
