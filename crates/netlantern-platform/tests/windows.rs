#![cfg(windows)]

use std::net::{Ipv4Addr, TcpListener};
use std::time::Duration;

use netlantern_platform::win::{read_adapters, read_neighbors, WindowsPlatform};
use netlantern_platform::{choose_interface, Platform, PlatformError, TcpProbe};

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}
fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn reads_adapters_without_admin() {
    let a = read_adapters().unwrap();
    assert!(!a.is_empty());
}

#[test]
fn neighbor_table_reads() {
    if let Some((_, idx)) = choose_interface(&read_adapters().unwrap()) {
        read_neighbors(idx).unwrap();
    }
}

#[tokio::test]
async fn pings_loopback() {
    let r = WindowsPlatform::new()
        .ping(ip("127.0.0.1"), ms(800))
        .await
        .unwrap()
        .unwrap();
    assert!(r.rtt < ms(800));
}

#[tokio::test]
async fn unanswered_ping_is_none_not_an_error() {
    // TEST-NET-1 is never on-link; the OS reports unreachable or a timeout.
    assert_eq!(
        WindowsPlatform::new()
            .ping(ip("192.0.2.1"), ms(300))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn tcp_probe_open_and_closed() {
    let p = WindowsPlatform::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    assert_eq!(
        p.tcp_probe(ip("127.0.0.1"), port, secs(5)).await.unwrap(),
        TcpProbe::Open
    );
    drop(listener);
    // A refused loopback connect takes about 1 s to report on Windows, so a
    // short timeout would read Filtered; give it room.
    assert_eq!(
        p.tcp_probe(ip("127.0.0.1"), port, secs(5)).await.unwrap(),
        TcpProbe::Closed
    );
}

#[tokio::test]
async fn arp_resolve_never_errors() {
    let p = WindowsPlatform::new();
    assert!(p.arp_resolve(ip("192.0.2.1"), secs(3)).await.is_ok());
}

#[tokio::test]
async fn interface_or_not_connected() {
    match WindowsPlatform::new().active_interface().await {
        Ok(i) => assert!((1..=32).contains(&i.prefix)),
        Err(e) => assert_eq!(e, PlatformError::NotConnected),
    }
}

#[tokio::test]
async fn arp_table_reads() {
    let r = WindowsPlatform::new().arp_table().await;
    assert!(r.is_ok() || r == Err(PlatformError::NotConnected));
}

#[tokio::test]
#[ignore = "needs a real LAN; run manually"]
async fn real_lan_gateway_answers() {
    let p = WindowsPlatform::new();
    let i = p.active_interface().await.unwrap();
    assert!(p.arp_resolve(i.gateway, secs(3)).await.unwrap().is_some());
}

#[tokio::test]
#[ignore = "needs a Wi-Fi adapter"]
async fn wifi_current_reads_the_connection_or_asks_for_location() {
    match WindowsPlatform::new().wifi_current().await {
        Ok(Some(c)) => {
            assert!((-100..=0).contains(&c.rssi_dbm));
            assert!(c.channel > 0);
            println!("{c:?}");
        }
        Ok(None) => println!("not connected to Wi-Fi"),
        Err(PlatformError::PermissionNeeded { what }) => assert_eq!(what, "location"),
        Err(e) => panic!("unexpected: {e}"),
    }
}

#[tokio::test]
#[ignore = "needs a Wi-Fi adapter"]
async fn wifi_scan_lists_networks_or_asks_for_location() {
    match WindowsPlatform::new().wifi_scan().await {
        Ok(list) => {
            assert!(list.iter().all(|n| n.channel > 0));
            println!("{} networks", list.len());
            for n in &list {
                println!("{n:?}");
            }
        }
        Err(PlatformError::PermissionNeeded { what }) => assert_eq!(what, "location"),
        Err(e) => panic!("unexpected: {e}"),
    }
}
