use std::net::Ipv4Addr;
use std::time::Duration;

use netlantern_platform::sim::SimNetwork;
use netlantern_platform::{
    tcp_connect_probe, ArpEntry, InterfaceKind, MacAddr, NeighborState, NetIo, Platform,
    PlatformError, ScenarioError, TcpProbe,
};
use tokio::time::Instant;

const HOME: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/home.json");
const ISOLATED: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/isolated.json");
const OFFLINE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scenarios/offline.json");

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn mac(s: &str) -> MacAddr {
    s.parse().unwrap()
}

fn home() -> SimNetwork {
    SimNetwork::load(HOME).unwrap()
}

#[tokio::test(start_paused = true)]
async fn home_interface_matches_the_scenario() {
    let sim = home();
    assert_eq!(sim.name(), "home");

    let iface = sim.active_interface().await.unwrap();
    assert_eq!(iface.ip, ip("192.168.1.23"));
    assert_eq!(iface.prefix, 24);
    assert_eq!(iface.gateway, ip("192.168.1.1"));
    assert_eq!(iface.dns_servers, vec![ip("192.168.1.1")]);
    assert_eq!(iface.kind, InterfaceKind::Wifi);
    assert_eq!(iface.mac, mac("F4:8E:38:9A:20:7C"));
}

#[tokio::test(start_paused = true)]
async fn home_ping_arp_and_tcp() {
    let sim = home();

    // ARP table: every device is present and reachable, in address order.
    let table = sim.arp_table().await.unwrap();
    assert_eq!(table.len(), 13);
    assert_eq!(
        table[0],
        ArpEntry {
            ip: ip("192.168.1.1"),
            mac: mac("50:C7:BF:3A:11:02"),
            state: NeighborState::Reachable,
        }
    );
    assert!(table.iter().all(|e| e.state == NeighborState::Reachable));
    assert!(table.windows(2).all(|w| w[0].ip < w[1].ip));
    // The printer (.40) and the Xiaomi (.114) are offline.
    assert!(!table.iter().any(|e| e.ip == ip("192.168.1.40")));
    assert!(!table.iter().any(|e| e.ip == ip("192.168.1.114")));

    // Ping: a live device answers after its rtt with its TTL.
    let r = sim.ping(ip("192.168.1.1"), ms(800)).await.unwrap().unwrap();
    assert_eq!((r.rtt, r.ttl), (ms(2), 64));
    let r = sim
        .ping(ip("192.168.1.120"), ms(800))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((r.rtt, r.ttl), (ms(9), 128));
    // A device that is absent, or present but silent, never answers.
    assert!(sim
        .ping(ip("192.168.1.40"), ms(800))
        .await
        .unwrap()
        .is_none());
    assert!(sim
        .ping(ip("192.168.1.88"), ms(800))
        .await
        .unwrap()
        .is_none());

    // ARP resolve works even for the device that ignores ping.
    assert_eq!(
        sim.arp_resolve(ip("192.168.1.88"), ms(800)).await.unwrap(),
        Some(mac("24:6F:28:3A:9F:21"))
    );
    assert_eq!(
        sim.arp_resolve(ip("192.168.1.40"), ms(800)).await.unwrap(),
        None
    );

    // TCP: listed port -> open, present device -> closed, absent device -> filtered.
    assert_eq!(
        sim.tcp_probe(ip("192.168.1.70"), 5000, ms(800))
            .await
            .unwrap(),
        TcpProbe::Open
    );
    assert_eq!(
        sim.tcp_probe(ip("192.168.1.70"), 9999, ms(800))
            .await
            .unwrap(),
        TcpProbe::Closed
    );
    assert_eq!(
        sim.tcp_probe(ip("192.168.1.88"), 80, ms(800))
            .await
            .unwrap(),
        TcpProbe::Open
    );
    assert_eq!(
        sim.tcp_probe(ip("192.168.1.40"), 80, ms(800))
            .await
            .unwrap(),
        TcpProbe::Filtered
    );
}

#[tokio::test(start_paused = true)]
async fn ping_takes_simulated_time() {
    let sim = home();

    // A live device answers after exactly its rtt...
    let start = Instant::now();
    sim.ping(ip("192.168.1.102"), ms(800)).await.unwrap();
    assert_eq!(start.elapsed(), ms(41));

    // ...a silent or absent one costs the full timeout...
    let start = Instant::now();
    sim.ping(ip("192.168.1.88"), ms(800)).await.unwrap();
    assert_eq!(start.elapsed(), ms(800));
    let start = Instant::now();
    sim.ping(ip("192.168.1.40"), ms(300)).await.unwrap();
    assert_eq!(start.elapsed(), ms(300));

    // ...and a reply slower than the timeout is not a reply.
    let start = Instant::now();
    let reply = sim.ping(ip("192.168.1.102"), ms(20)).await.unwrap();
    assert!(reply.is_none());
    assert_eq!(start.elapsed(), ms(20));
}

#[tokio::test(start_paused = true)]
async fn home_dns_and_http() {
    let sim = home();

    assert_eq!(
        sim.dns_resolve(ip("1.1.1.1"), "google.com").await.unwrap(),
        vec![ip("142.250.183.14")]
    );
    assert!(matches!(
        sim.dns_resolve(ip("1.1.1.1"), "nope.invalid").await,
        Err(PlatformError::Io { .. })
    ));

    let resp = sim
        .http_get("http://www.msftconnecttest.com/connecttest.txt", false)
        .await
        .unwrap();
    assert_eq!(resp.status, 200);
    assert_eq!(resp.body_prefix, "Microsoft Connect Test");
    assert!(matches!(
        sim.http_get("http://unknown.test/", false).await,
        Err(PlatformError::Io { .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn offline_platform_calls_are_not_connected() {
    let offline = SimNetwork::load(OFFLINE).unwrap();
    assert_eq!(offline.name(), "offline");

    assert!(matches!(
        offline.active_interface().await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.ping(ip("192.168.1.1"), ms(800)).await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.arp_resolve(ip("192.168.1.1"), ms(800)).await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.arp_table().await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.tcp_probe(ip("192.168.1.1"), 80, ms(800)).await,
        Err(PlatformError::NotConnected)
    ));
}

#[tokio::test(start_paused = true)]
async fn scenario_defaults_and_silent_arp() {
    // ttl defaults to 64, open_ports to [], arp to true; `arp: false` hides a device.
    let sim = SimNetwork::from_json(
        r#"{
            "name": "tiny",
            "interface": { "ip": "10.0.0.2", "prefix": 8, "gateway": "10.0.0.1",
                           "dns": [], "kind": "ethernet", "mac": "02:00:00:00:00:02" },
            "devices": [
                { "ip": "10.0.0.1", "mac": "02:00:00:00:00:01", "ping_ms": 1 },
                { "ip": "10.0.0.9", "mac": "02:00:00:00:00:09", "ping_ms": 3, "arp": false }
            ],
            "dns": {},
            "http": { "http://x.test/": { "status": 302 } }
        }"#,
    )
    .unwrap();

    assert_eq!(
        sim.active_interface().await.unwrap().kind,
        InterfaceKind::Ethernet
    );
    let reply = sim.ping(ip("10.0.0.1"), ms(100)).await.unwrap().unwrap();
    assert_eq!(reply.ttl, 64);
    assert_eq!(
        sim.tcp_probe(ip("10.0.0.1"), 80, ms(100)).await.unwrap(),
        TcpProbe::Closed
    );

    // The silent-ARP device answers ping but is invisible to ARP.
    assert!(sim.ping(ip("10.0.0.9"), ms(100)).await.unwrap().is_some());
    let table = sim.arp_table().await.unwrap();
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].ip, ip("10.0.0.1"));
    assert_eq!(
        sim.arp_resolve(ip("10.0.0.9"), ms(100)).await.unwrap(),
        None
    );

    // A body-less HTTP entry has an empty body.
    let resp = sim.http_get("http://x.test/", false).await.unwrap();
    assert_eq!((resp.status, resp.body_prefix.as_str()), (302, ""));
}

#[test]
fn bad_scenarios_report_errors() {
    assert!(matches!(
        SimNetwork::from_json("{ not json"),
        Err(ScenarioError::Json(_))
    ));
    // Valid JSON, but a field has the wrong shape (bad MAC address).
    assert!(matches!(
        SimNetwork::from_json(
            r#"{ "name": "x", "interface": null, "dns": {}, "http": {},
                 "devices": [ { "ip": "10.0.0.1", "mac": "nope", "ping_ms": 1 } ] }"#
        ),
        Err(ScenarioError::Json(_))
    ));
    assert!(matches!(
        SimNetwork::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/no-such-scenario.json"
        )),
        Err(ScenarioError::Io(_))
    ));
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[tokio::test(start_paused = true)]
async fn home_discovery_methods() {
    let home = home();
    let offline = SimNetwork::load(OFFLINE).unwrap();

    let svcs = home.mdns_browse(ip("192.168.1.23"), secs(3)).await.unwrap();
    assert!(svcs.iter().any(
        |s| s.ip == ip("192.168.1.31") && s.friendly_name.as_deref() == Some("Kitchen speaker")
    ));
    assert!(svcs.windows(2).all(|w| w[0].ip <= w[1].ip));
    let ssdp = home.ssdp_search(ip("192.168.1.23"), secs(3)).await.unwrap();
    assert!(ssdp
        .iter()
        .any(|r| r.ip == ip("192.168.1.52") && r.st.contains("MediaRenderer")));
    assert_eq!(
        home.netbios_status(ip("192.168.1.70"), secs(1))
            .await
            .unwrap()
            .unwrap()
            .name,
        "DISKSTATION"
    );
    assert_eq!(
        home.netbios_status(ip("192.168.1.31"), secs(1))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        home.reverse_dns(ip("192.168.1.1"), ip("192.168.1.61"))
            .await
            .unwrap()
            .as_deref(),
        Some("echo-dot.lan")
    );
    let xml = home
        .http_get("http://192.168.1.52:9197/dmr", false)
        .await
        .unwrap();
    assert!(xml.body_prefix.contains("[TV] Living Room"));

    assert!(matches!(
        offline.mdns_browse(ip("0.0.0.0"), secs(3)).await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.ssdp_search(ip("0.0.0.0"), secs(3)).await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.netbios_status(ip("192.168.1.70"), secs(1)).await,
        Err(PlatformError::NotConnected)
    ));
    assert!(matches!(
        offline.reverse_dns(ip("1.1.1.1"), ip("192.168.1.61")).await,
        Err(PlatformError::NotConnected)
    ));
}

#[tokio::test(start_paused = true)]
async fn unknown_device_waits_for_the_timeout() {
    let home = home();
    let start = Instant::now();
    assert_eq!(
        home.netbios_status(ip("192.168.1.200"), secs(1))
            .await
            .unwrap(),
        None
    );
    assert_eq!(start.elapsed(), secs(1));
}

#[tokio::test(start_paused = true)]
async fn isolated_scenario_has_gateway_and_laptop_only() {
    let iso = SimNetwork::load(ISOLATED).unwrap();
    assert_eq!(iso.name(), "isolated");
    let table = iso.arp_table().await.unwrap();
    let ips: Vec<_> = table.iter().map(|e| e.ip).collect();
    assert_eq!(ips, vec![ip("192.168.1.1"), ip("192.168.1.23")]);
}

#[tokio::test]
async fn tcp_connect_probe_on_real_sockets() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    assert_eq!(
        tcp_connect_probe(ip("127.0.0.1"), port, ms(800)).await,
        TcpProbe::Open
    );
    drop(l);
    // Windows reports a refused loopback connect only after ~1 s of SYN retries,
    // so give the closed case a generous timeout.
    assert_eq!(
        tcp_connect_probe(ip("127.0.0.1"), port, secs(5)).await,
        TcpProbe::Closed
    );
}
