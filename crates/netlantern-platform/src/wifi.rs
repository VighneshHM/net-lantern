//! Wi-Fi types for the [`crate::Platform`] seam, plus pure helpers that turn
//! raw Windows WLAN numbers and 802.11 information elements into labels.

use serde::{Deserialize, Serialize};

use crate::MacAddr;

/// The `what` of [`crate::PlatformError::PermissionNeeded`] when Windows hides
/// Wi-Fi details until location access is granted.
pub const LOCATION_PERMISSION: &str = "location";

/// The radio band a Wi-Fi network uses.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum WifiBand {
    #[serde(rename = "2.4ghz")]
    Ghz2_4,
    #[serde(rename = "5ghz")]
    Ghz5,
    #[serde(rename = "6ghz")]
    Ghz6,
}

impl WifiBand {
    /// Human-readable band name, e.g. `"2.4 GHz"`.
    pub fn label(&self) -> &'static str {
        match self {
            WifiBand::Ghz2_4 => "2.4 GHz",
            WifiBand::Ghz5 => "5 GHz",
            WifiBand::Ghz6 => "6 GHz",
        }
    }
}

/// The Wi-Fi network the laptop is connected to.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WifiConnection {
    pub ssid: String,
    pub bssid: MacAddr,
    pub rssi_dbm: i32,
    pub channel: u32,
    pub band: WifiBand,
    pub width_mhz: Option<u32>,
    /// Radio standard, e.g. `Wi-Fi 6 (802.11ax)`.
    pub phy: String,
    pub rx_rate_mbps: Option<u32>,
    pub tx_rate_mbps: Option<u32>,
    /// Authentication, e.g. `WPA2-Personal`.
    pub security: String,
}

/// One access point seen in a radio scan.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct NearbyNetwork {
    /// Empty when the network is hidden.
    pub ssid: String,
    pub bssid: MacAddr,
    pub rssi_dbm: i32,
    pub channel: u32,
    pub band: WifiBand,
    pub width_mhz: Option<u32>,
}

/// Map a centre frequency in MHz to its band and channel number.
pub fn channel_from_mhz(mhz: u32) -> Option<(WifiBand, u32)> {
    match mhz {
        2412..=2472 => Some((WifiBand::Ghz2_4, (mhz - 2407) / 5)),
        2484 => Some((WifiBand::Ghz2_4, 14)),
        5160..=5885 => Some((WifiBand::Ghz5, (mhz - 5000) / 5)),
        5955..=7115 => Some((WifiBand::Ghz6, (mhz - 5950) / 5)),
        _ => None,
    }
}

/// Convert a WLAN link quality (0-100) to an approximate RSSI in dBm.
pub fn rssi_from_quality(quality: u32) -> i32 {
    (quality.min(100) / 2) as i32 - 100
}

/// Name a `DOT11_PHY_TYPE` value.
pub fn phy_label(phy_type: u32, band: WifiBand) -> &'static str {
    match phy_type {
        4 => "802.11a",
        5 => "802.11b",
        6 => "802.11g",
        7 => "Wi-Fi 4 (802.11n)",
        8 => "Wi-Fi 5 (802.11ac)",
        10 if band == WifiBand::Ghz6 => "Wi-Fi 6E (802.11ax)",
        10 => "Wi-Fi 6 (802.11ax)",
        11 => "Wi-Fi 7 (802.11be)",
        _ => "Unknown",
    }
}

/// Name a `DOT11_AUTH_ALGORITHM` value.
pub fn security_label(auth_algorithm: u32) -> &'static str {
    match auth_algorithm {
        1 => "Open",
        2 => "Shared key (WEP)",
        3 => "WPA-Enterprise",
        4 => "WPA-Personal",
        6 => "WPA2-Enterprise",
        7 => "WPA2-Personal",
        8 | 11 => "WPA3-Enterprise",
        9 => "WPA3-Personal",
        10 => "Enhanced Open",
        _ => "Unknown",
    }
}

/// Read the channel width from 802.11 information elements (each a 1-byte id,
/// a 1-byte length and the payload). The widest width found wins; `None` when
/// there is no HT, VHT or HE operation element. A truncated element ends the walk.
pub fn channel_width_mhz(ies: &[u8]) -> Option<u32> {
    let mut best: Option<u32> = None;
    let mut widen = |w: u32| best = Some(best.map_or(w, |b| b.max(w)));
    let mut ht = 20;
    let mut rest = ies;
    while let [id, len, tail @ ..] = rest {
        let len = *len as usize;
        if tail.len() < len {
            break;
        }
        let (payload, next) = tail.split_at(len);
        rest = next;
        match (*id, payload) {
            (61, [_, info, ..]) => {
                ht = if matches!(info & 0x03, 1 | 3) { 40 } else { 20 };
                widen(ht);
            }
            (192, [width, _, ccfs1, ..]) => match width {
                0 => widen(ht),
                1 if *ccfs1 != 0 => widen(160),
                1 => widen(80),
                2 | 3 => widen(160),
                _ => {}
            },
            (255, [36, p0, p1, p2, tail @ ..]) => {
                let params = u32::from_le_bytes([*p0, *p1, *p2, 0]);
                if params & (1 << 17) == 0 {
                    continue;
                }
                // BSS colour (1) and basic HE-MCS set (2) come first.
                let mut offset = 3;
                if params & (1 << 14) != 0 {
                    offset += 3;
                }
                if params & (1 << 15) != 0 {
                    offset += 1;
                }
                if let Some(control) = tail.get(offset + 1) {
                    widen(match control & 0x03 {
                        0 => 20,
                        1 => 40,
                        2 => 80,
                        _ => 160,
                    });
                }
            }
            _ => {}
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequencies_map_to_band_and_channel() {
        assert_eq!(channel_from_mhz(2437), Some((WifiBand::Ghz2_4, 6)));
        assert_eq!(channel_from_mhz(2484), Some((WifiBand::Ghz2_4, 14)));
        assert_eq!(channel_from_mhz(5180), Some((WifiBand::Ghz5, 36)));
        assert_eq!(channel_from_mhz(5975), Some((WifiBand::Ghz6, 5)));
        assert_eq!(channel_from_mhz(3000), None);
    }

    #[test]
    fn widths_from_information_elements() {
        let ht40 = [
            61, 22, 6, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(channel_width_mhz(&ht40), Some(40));
        let vht80 = [&ht40[..], &[192, 5, 1, 42, 0, 0, 0][..]].concat();
        assert_eq!(channel_width_mhz(&vht80), Some(80));
        let vht160 = [&ht40[..], &[192, 5, 1, 42, 50, 0, 0][..]].concat();
        assert_eq!(channel_width_mhz(&vht160), Some(160));
        assert_eq!(channel_width_mhz(&[]), None);
        assert_eq!(channel_width_mhz(&[61, 200, 1]), None); // truncated: no panic
    }

    #[test]
    fn vht_width_zero_falls_back_to_the_ht_answer() {
        let ies = [61, 2, 6, 0x03, 192, 3, 0, 0, 0];
        assert_eq!(channel_width_mhz(&ies), Some(40));
    }

    #[test]
    fn he_operation_reads_the_6ghz_control_field() {
        // Parameters: bit 17 set (6 GHz info present), no VHT info or co-hosted.
        // Then colour, 2 MCS bytes, primary channel, control = 2 (80 MHz).
        let ies = [255, 9, 36, 0x00, 0x00, 0x02, 0, 0, 0, 5, 2];
        assert_eq!(channel_width_mhz(&ies), Some(80));
        // Bit 17 clear: ignored.
        let without = [255, 9, 36, 0, 0, 0, 0, 0, 0, 5, 2];
        assert_eq!(channel_width_mhz(&without), None);
    }

    #[test]
    fn labels() {
        assert_eq!(phy_label(10, WifiBand::Ghz6), "Wi-Fi 6E (802.11ax)");
        assert_eq!(phy_label(10, WifiBand::Ghz5), "Wi-Fi 6 (802.11ax)");
        assert_eq!(security_label(7), "WPA2-Personal");
        assert_eq!(rssi_from_quality(100), -50);
        assert_eq!(rssi_from_quality(500), -50);
        assert_eq!(WifiBand::Ghz2_4.label(), "2.4 GHz");
        assert_eq!(
            serde_json::to_string(&WifiBand::Ghz2_4).unwrap(),
            "\"2.4ghz\""
        );
    }
}
