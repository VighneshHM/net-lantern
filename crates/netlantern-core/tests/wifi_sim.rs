//! The Wi-Fi service against simulated networks, with a paused clock so the
//! simulated one-second radio scan costs no real time.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use netlantern_core::{CongestionConfig, CoreError, WifiService, WifiStatus};
use netlantern_platform::{
    ArpEntry, Interface, InterfaceKind, MacAddr, NearbyNetwork, PingReply, Platform, PlatformError,
    SimNetwork, TcpProbe, WifiConnection,
};
use std::net::Ipv4Addr;

/// A `SimNetwork` that counts radio scans and can fail `wifi_current`.
struct Counting {
    inner: SimNetwork,
    scans: AtomicUsize,
    current_err: Option<PlatformError>,
}

impl Counting {
    fn new(inner: SimNetwork) -> Arc<Counting> {
        Arc::new(Counting {
            inner,
            scans: AtomicUsize::new(0),
            current_err: None,
        })
    }

    /// Every `wifi_current` call fails with `err`.
    fn failing_current(inner: SimNetwork, err: PlatformError) -> Arc<Counting> {
        Arc::new(Counting {
            inner,
            scans: AtomicUsize::new(0),
            current_err: Some(err),
        })
    }

    fn scans(&self) -> usize {
        self.scans.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Platform for Counting {
    async fn active_interface(&self) -> Result<Interface, PlatformError> {
        self.inner.active_interface().await
    }
    async fn ping(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<PingReply>, PlatformError> {
        self.inner.ping(ip, timeout).await
    }
    async fn arp_resolve(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<MacAddr>, PlatformError> {
        self.inner.arp_resolve(ip, timeout).await
    }
    async fn arp_table(&self) -> Result<Vec<ArpEntry>, PlatformError> {
        self.inner.arp_table().await
    }
    async fn tcp_probe(
        &self,
        ip: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Result<TcpProbe, PlatformError> {
        self.inner.tcp_probe(ip, port, timeout).await
    }
    async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError> {
        if let Some(err) = &self.current_err {
            return Err(err.clone());
        }
        self.inner.wifi_current().await
    }
    async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.inner.wifi_scan().await
    }
}

fn scenario_text(name: &str) -> String {
    let path = format!("{}/../../scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

/// The home scenario with a Wi-Fi block: connected on channel 6 with two
/// neighbours, and location access as given.
fn home_wifi(permission: &str) -> SimNetwork {
    let mut json: serde_json::Value = serde_json::from_str(&scenario_text("home")).unwrap();
    json["wifi"] = serde_json::json!({
        "permission": permission,
        "current": {
            "ssid": "Lantern Home", "bssid": "02:00:00:00:00:01", "rssi_dbm": -52,
            "channel": 6, "band": "2.4ghz", "width_mhz": 20, "phy": "Wi-Fi 5 (802.11ac)",
            "rx_rate_mbps": 144, "tx_rate_mbps": 144, "security": "WPA2-Personal"
        },
        "nearby": [
            { "ssid": "Neighbour", "bssid": "02:00:00:00:00:02", "rssi_dbm": -70,
              "channel": 6, "band": "2.4ghz", "width_mhz": 20 },
            { "ssid": "Cafe", "bssid": "02:00:00:00:00:03", "rssi_dbm": -45,
              "channel": 1, "band": "2.4ghz", "width_mhz": 20 }
        ]
    });
    SimNetwork::from_json(&json.to_string()).unwrap()
}

fn service(platform: &Arc<Counting>) -> WifiService {
    WifiService::new(platform.clone(), CongestionConfig::default())
}

#[tokio::test(start_paused = true)]
async fn nearby_reuses_a_fresh_scan_and_rescans_a_stale_one() {
    let platform = Counting::new(home_wifi("granted"));
    let wifi = service(&platform);
    let ten = Duration::from_secs(10);

    let first = wifi.nearby(ten).await.unwrap();
    let second = wifi.nearby(ten).await.unwrap();
    assert_eq!(platform.scans(), 1);
    assert_eq!(first, second);
    // Strongest first, and the congestion saw the connected network.
    assert_eq!(first.networks[0].ssid, "Cafe");
    assert_eq!(first.congestion.current.as_ref().unwrap().overlapping, 1);

    tokio::time::advance(Duration::from_secs(11)).await;
    wifi.nearby(ten).await.unwrap();
    assert_eq!(platform.scans(), 2);
}

#[tokio::test(start_paused = true)]
async fn concurrent_nearby_calls_share_one_radio_scan() {
    let platform = Counting::new(home_wifi("granted"));
    let wifi = service(&platform);
    let ten = Duration::from_secs(10);

    let (a, b) = tokio::join!(wifi.nearby(ten), wifi.nearby(ten));
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(platform.scans(), 1);
}

#[tokio::test(start_paused = true)]
async fn status_passes_location_permission_through() {
    let platform = Counting::new(home_wifi("denied"));
    let err = service(&platform).status().await.unwrap_err();
    assert_eq!(
        err,
        CoreError::Platform(PlatformError::PermissionNeeded {
            what: "location".into()
        })
    );
}

#[tokio::test(start_paused = true)]
async fn status_when_not_connected_has_no_interface_kind() {
    let offline = SimNetwork::from_json(&scenario_text("offline")).unwrap();
    let platform = Counting::new(offline);
    let status = service(&platform).status().await.unwrap();
    assert_eq!(
        status,
        WifiStatus {
            interface_kind: None,
            connection: None
        }
    );
}

fn ethernet_home() -> SimNetwork {
    let mut json: serde_json::Value = serde_json::from_str(&scenario_text("home")).unwrap();
    json["interface"]["kind"] = serde_json::json!("ethernet");
    SimNetwork::from_json(&json.to_string()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn status_keeps_the_interface_kind_when_wifi_is_unavailable() {
    // A desktop with no Wi-Fi (or the radio off) on a cable.
    let unavailable = PlatformError::Unavailable {
        reason: "Wi-Fi is turned off or not ready".into(),
    };
    let platform = Counting::failing_current(ethernet_home(), unavailable);
    let status = service(&platform).status().await.unwrap();
    assert_eq!(
        status,
        WifiStatus {
            interface_kind: Some(InterfaceKind::Ethernet),
            connection: None
        }
    );
}

#[tokio::test(start_paused = true)]
async fn status_still_passes_location_permission_through_on_a_cable() {
    let denied = PlatformError::PermissionNeeded {
        what: "location".into(),
    };
    let platform = Counting::failing_current(ethernet_home(), denied.clone());
    let err = service(&platform).status().await.unwrap_err();
    assert_eq!(err, CoreError::Platform(denied));
}

#[tokio::test(start_paused = true)]
async fn unavailable_wifi_on_a_wifi_interface_is_an_error() {
    let unavailable = PlatformError::Unavailable {
        reason: "Wi-Fi is turned off or not ready".into(),
    };
    let platform = Counting::failing_current(home_wifi("granted"), unavailable.clone());
    let err = service(&platform).status().await.unwrap_err();
    assert_eq!(err, CoreError::Platform(unavailable));
}

#[tokio::test(start_paused = true)]
async fn unavailable_wifi_on_ethernet_is_no_connection() {
    let unavailable = PlatformError::Unavailable {
        reason: "no adapter".into(),
    };
    let platform = Counting::failing_current(ethernet_home(), unavailable);
    assert_eq!(
        service(&platform).status().await.unwrap(),
        WifiStatus {
            interface_kind: Some(InterfaceKind::Ethernet),
            connection: None
        }
    );
}
