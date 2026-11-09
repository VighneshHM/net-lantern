//! Channel congestion analysis and the Wi-Fi service that caches radio scans.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use netlantern_platform::{
    InterfaceKind, MacAddr, NearbyNetwork, Platform, PlatformError, WifiBand, WifiConnection,
};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::CoreError;

/// Channels worth suggesting on 2.4 GHz: the three that don't overlap.
const CHANNELS_2_4: [u32; 3] = [1, 6, 11];
/// Channels worth suggesting on 5 GHz.
const CHANNELS_5: [u32; 8] = [36, 40, 44, 48, 149, 153, 157, 161];

/// How many other networks make a channel crowded.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CongestionConfig {
    /// Overlapping networks that make a 2.4 GHz channel crowded.
    pub crowded_2_4: u32,
    /// Networks on the same channel that make a 5 or 6 GHz channel crowded.
    pub crowded_other: u32,
}

impl Default for CongestionConfig {
    fn default() -> Self {
        CongestionConfig {
            crowded_2_4: 5,
            crowded_other: 3,
        }
    }
}

/// Distinct access points on exactly one channel.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChannelLoad {
    pub band: WifiBand,
    pub channel: u32,
    pub networks: u32,
}

/// How busy the connected network's channel is.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CurrentLoad {
    pub band: WifiBand,
    pub channel: u32,
    /// Other access points whose channel overlaps this one.
    pub overlapping: u32,
    pub crowded: bool,
    /// The same network is also visible on 5 or 6 GHz while connected on 2.4 GHz.
    pub own_5ghz_visible: bool,
    pub suggested_channel: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Congestion {
    /// Sorted by band, then channel.
    pub channels: Vec<ChannelLoad>,
    pub current: Option<CurrentLoad>,
    pub advice: String,
}

/// A radio scan with its analysis.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NearbyScan {
    /// Milliseconds since the Unix epoch.
    pub scanned_at: u64,
    /// Strongest first.
    pub networks: Vec<NearbyNetwork>,
    pub congestion: Congestion,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WifiStatus {
    /// `None` when the laptop is not connected to any network.
    pub interface_kind: Option<InterfaceKind>,
    pub connection: Option<WifiConnection>,
}

fn band_rank(band: WifiBand) -> u8 {
    match band {
        WifiBand::Ghz2_4 => 0,
        WifiBand::Ghz5 => 1,
        WifiBand::Ghz6 => 2,
    }
}

/// Whether networks on `a` and `b` interfere. Channel width is ignored.
fn overlaps(band: WifiBand, a: u32, b: u32) -> bool {
    match band {
        WifiBand::Ghz2_4 => a.abs_diff(b) < 5,
        _ => a == b,
    }
}

/// Distinct BSSIDs in `nearby`, other than `skip`, that overlap `channel`.
fn count_overlapping(nearby: &[NearbyNetwork], band: WifiBand, channel: u32, skip: MacAddr) -> u32 {
    let seen: BTreeSet<MacAddr> = nearby
        .iter()
        .filter(|n| n.band == band && overlaps(band, n.channel, channel))
        .map(|n| n.bssid)
        .filter(|b| *b != skip)
        .collect();
    seen.len() as u32
}

/// The candidate channel with the fewest overlapping networks; ties go low.
fn suggest(
    nearby: &[NearbyNetwork],
    band: WifiBand,
    skip: MacAddr,
    candidates: &[u32],
) -> Option<u32> {
    candidates
        .iter()
        .copied()
        .min_by_key(|&c| (count_overlapping(nearby, band, c, skip), c))
}

/// Work out how crowded the connected network's channel is.
pub fn analyze_congestion(
    current: Option<&WifiConnection>,
    nearby: &[NearbyNetwork],
    cfg: &CongestionConfig,
) -> Congestion {
    let mut per_channel: BTreeMap<(u8, u32), (WifiBand, BTreeSet<MacAddr>)> = BTreeMap::new();
    for n in nearby {
        per_channel
            .entry((band_rank(n.band), n.channel))
            .or_insert_with(|| (n.band, BTreeSet::new()))
            .1
            .insert(n.bssid);
    }
    let channels = per_channel
        .into_iter()
        .map(|((_, channel), (band, set))| ChannelLoad {
            band,
            channel,
            networks: set.len() as u32,
        })
        .collect();

    let Some(cur) = current else {
        return Congestion {
            channels,
            current: None,
            advice: "Not on Wi-Fi, so channel congestion doesn't affect this connection.".into(),
        };
    };

    let overlapping = count_overlapping(nearby, cur.band, cur.channel, cur.bssid);
    let threshold = if cur.band == WifiBand::Ghz2_4 {
        cfg.crowded_2_4
    } else {
        cfg.crowded_other
    };
    let crowded = overlapping >= threshold;
    let own_5ghz_visible = cur.band == WifiBand::Ghz2_4
        && nearby
            .iter()
            .any(|n| n.ssid == cur.ssid && n.band != WifiBand::Ghz2_4);
    let suggested_channel = if !crowded {
        None
    } else {
        match cur.band {
            WifiBand::Ghz2_4 => suggest(nearby, cur.band, cur.bssid, &CHANNELS_2_4),
            WifiBand::Ghz5 => suggest(nearby, cur.band, cur.bssid, &CHANNELS_5),
            WifiBand::Ghz6 => None,
        }
    };

    let ch = cur.channel;
    let shared = format!("Channel {ch} is shared with {overlapping} other networks.");
    let advice = if !crowded {
        format!("Your channel isn't crowded ({overlapping} other networks share it).")
    } else if own_5ghz_visible {
        format!("{shared} Your router also broadcasts on 5 GHz — join that for less interference.")
    } else {
        match (cur.band, suggested_channel) {
            (WifiBand::Ghz2_4, Some(s)) => format!(
                "{shared} In your router's settings, try channel {s} (the least crowded of 1, 6 and 11)."
            ),
            (WifiBand::Ghz5, Some(s)) => {
                format!("{shared} In your router's settings, try channel {s}.")
            }
            _ => shared,
        }
    };

    Congestion {
        channels,
        current: Some(CurrentLoad {
            band: cur.band,
            channel: cur.channel,
            overlapping,
            crowded,
            own_5ghz_visible,
            suggested_channel,
        }),
        advice,
    }
}

struct Cached {
    at: Instant,
    scan: NearbyScan,
}

/// Reads Wi-Fi state and keeps the latest radio scan so callers share it.
pub struct WifiService {
    platform: Arc<dyn Platform>,
    cfg: CongestionConfig,
    cache: StdMutex<Option<Cached>>,
    /// Held for the whole radio scan so only one runs at a time.
    scan_gate: Mutex<()>,
}

impl WifiService {
    pub fn new(platform: Arc<dyn Platform>, cfg: CongestionConfig) -> Self {
        WifiService {
            platform,
            cfg,
            cache: StdMutex::new(None),
            scan_gate: Mutex::new(()),
        }
    }

    /// The interface kind and the current Wi-Fi connection, if any.
    pub async fn status(&self) -> Result<WifiStatus, CoreError> {
        let interface_kind = match self.platform.active_interface().await {
            Ok(iface) => Some(iface.kind),
            Err(PlatformError::NotConnected) => None,
            Err(e) => return Err(e.into()),
        };
        let connection = match self.platform.wifi_current().await {
            Ok(c) => c,
            Err(PlatformError::NotConnected) => None,
            Err(e) => return Err(e.into()),
        };
        Ok(WifiStatus {
            interface_kind,
            connection,
        })
    }

    /// The latest scan if it is younger than `max_age`, else a fresh one.
    pub async fn nearby(&self, max_age: Duration) -> Result<NearbyScan, CoreError> {
        if let Some(scan) = self.fresh(max_age) {
            return Ok(scan);
        }
        let _gate = self.scan_gate.lock().await;
        // Another call may have scanned while this one waited.
        if let Some(scan) = self.fresh(max_age) {
            return Ok(scan);
        }
        let mut networks = self.platform.wifi_scan().await?;
        networks.sort_by(|a, b| b.rssi_dbm.cmp(&a.rssi_dbm).then(a.bssid.cmp(&b.bssid)));
        let current = self.platform.wifi_current().await.ok().flatten();
        let congestion = analyze_congestion(current.as_ref(), &networks, &self.cfg);
        let scan = NearbyScan {
            scanned_at: unix_millis(),
            networks,
            congestion,
        };
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = Some(Cached {
            at: Instant::now(),
            scan: scan.clone(),
        });
        Ok(scan)
    }

    fn fresh(&self, max_age: Duration) -> Option<NearbyScan> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        let cached = cache.as_ref()?;
        (cached.at.elapsed() < max_age).then(|| cached.scan.clone())
    }
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(n: u8) -> MacAddr {
        format!("02:00:00:00:00:{n:02X}").parse().unwrap()
    }

    fn current(ssid: &str, channel: u32, band: WifiBand) -> WifiConnection {
        WifiConnection {
            ssid: ssid.into(),
            bssid: mac(0),
            rssi_dbm: -55,
            channel,
            band,
            width_mhz: Some(20),
            phy: "Wi-Fi 5 (802.11ac)".into(),
            rx_rate_mbps: None,
            tx_rate_mbps: None,
            security: "WPA2-Personal".into(),
        }
    }

    fn net(n: u8, ssid: &str, channel: u32, band: WifiBand) -> NearbyNetwork {
        NearbyNetwork {
            ssid: ssid.into(),
            bssid: mac(n),
            rssi_dbm: -60,
            channel,
            band,
            width_mhz: Some(20),
        }
    }

    fn cfg() -> CongestionConfig {
        CongestionConfig::default()
    }

    #[test]
    fn nine_overlapping_networks_on_channel_6_is_crowded_with_own_5ghz_advice() {
        let cur = current("Lantern Home", 6, WifiBand::Ghz2_4);
        let mut nearby: Vec<_> = (0..9u8)
            .map(|i| net(i + 1, "Other", 4 + u32::from(i % 5), WifiBand::Ghz2_4))
            .collect();
        nearby.push(net(20, "Lantern Home", 36, WifiBand::Ghz5));
        let c = analyze_congestion(Some(&cur), &nearby, &cfg());
        let load = c.current.unwrap();
        assert_eq!(
            (load.overlapping, load.crowded, load.own_5ghz_visible),
            (9, true, true)
        );
        assert!(c.advice.starts_with(
            "Channel 6 is shared with 9 other networks. Your router also broadcasts on 5 GHz"
        ));
    }

    #[test]
    fn channel_11_does_not_overlap_channel_6() {
        let cur = current("Home", 6, WifiBand::Ghz2_4);
        let nearby = vec![net(1, "Other", 11, WifiBand::Ghz2_4)];
        let load = analyze_congestion(Some(&cur), &nearby, &cfg())
            .current
            .unwrap();
        assert_eq!(load.overlapping, 0);
        assert!(!load.crowded);
    }

    #[test]
    fn suggests_least_crowded_of_1_6_11_ties_low() {
        let cur = current("Home", 6, WifiBand::Ghz2_4);
        let nearby: Vec<_> = (1..=5)
            .map(|i| net(i, "Other", 6, WifiBand::Ghz2_4))
            .collect();
        let c = analyze_congestion(Some(&cur), &nearby, &cfg());
        let load = c.current.unwrap();
        assert!(load.crowded);
        assert_eq!(load.suggested_channel, Some(1));
        assert_eq!(
            c.advice,
            "Channel 6 is shared with 5 other networks. In your router's settings, try channel 1 (the least crowded of 1, 6 and 11)."
        );
    }

    #[test]
    fn five_ghz_counts_same_channel_only() {
        let cur = current("Home", 36, WifiBand::Ghz5);
        let mut nearby: Vec<_> = (1..=3)
            .map(|i| net(i, "Other", 36, WifiBand::Ghz5))
            .collect();
        nearby.extend((4..=9).map(|i| net(i, "Other", 40, WifiBand::Ghz5)));
        let c = analyze_congestion(Some(&cur), &nearby, &cfg());
        let load = c.current.unwrap();
        assert_eq!((load.overlapping, load.crowded), (3, true));
        assert_eq!(load.suggested_channel, Some(44));
        assert_eq!(
            c.advice,
            "Channel 36 is shared with 3 other networks. In your router's settings, try channel 44."
        );
    }

    #[test]
    fn connected_bssid_is_not_counted() {
        let cur = current("Home", 6, WifiBand::Ghz2_4);
        let mut own = net(0, "Home", 6, WifiBand::Ghz2_4);
        own.bssid = cur.bssid;
        let nearby = vec![own, net(1, "", 6, WifiBand::Ghz2_4)];
        let load = analyze_congestion(Some(&cur), &nearby, &cfg())
            .current
            .unwrap();
        assert_eq!(load.overlapping, 1);
    }

    #[test]
    fn no_current_connection_gives_not_on_wifi_advice() {
        let nearby = vec![net(1, "Other", 6, WifiBand::Ghz2_4)];
        let c = analyze_congestion(None, &nearby, &cfg());
        assert!(c.current.is_none());
        assert_eq!(
            c.advice,
            "Not on Wi-Fi, so channel congestion doesn't affect this connection."
        );
        assert_eq!(c.channels.len(), 1);
    }

    #[test]
    fn channels_are_sorted_by_band_then_channel_and_count_distinct_bssids() {
        let nearby = vec![
            net(1, "A", 36, WifiBand::Ghz5),
            net(2, "B", 11, WifiBand::Ghz2_4),
            net(3, "C", 1, WifiBand::Ghz2_4),
            net(3, "C", 1, WifiBand::Ghz2_4),
        ];
        let c = analyze_congestion(None, &nearby, &cfg());
        let got: Vec<_> = c
            .channels
            .iter()
            .map(|l| (l.band, l.channel, l.networks))
            .collect();
        assert_eq!(
            got,
            vec![
                (WifiBand::Ghz2_4, 1, 1),
                (WifiBand::Ghz2_4, 11, 1),
                (WifiBand::Ghz5, 36, 1)
            ]
        );
    }

    #[test]
    fn not_crowded_advice_counts_other_networks() {
        let cur = current("Home", 6, WifiBand::Ghz2_4);
        let nearby = vec![net(1, "Other", 7, WifiBand::Ghz2_4)];
        let c = analyze_congestion(Some(&cur), &nearby, &cfg());
        assert_eq!(
            c.advice,
            "Your channel isn't crowded (1 other networks share it)."
        );
    }

    #[test]
    fn six_ghz_crowded_has_no_suggestion() {
        let cur = current("Home", 5, WifiBand::Ghz6);
        let nearby: Vec<_> = (1..=3).map(|i| net(i, "O", 5, WifiBand::Ghz6)).collect();
        let c = analyze_congestion(Some(&cur), &nearby, &cfg());
        assert_eq!(c.current.unwrap().suggested_channel, None);
        assert_eq!(c.advice, "Channel 5 is shared with 3 other networks.");
    }
}
