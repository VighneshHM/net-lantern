//! Wi-Fi readers over the native WLAN API (`wlanapi.dll`). No admin needed,
//! but Windows hides Wi-Fi details until location access is granted; that
//! surfaces as `PermissionNeeded { what: "location" }`.

use std::ffi::c_void;

use windows::core::GUID;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_DATA, ERROR_INVALID_STATE, ERROR_SERVICE_NOT_ACTIVE, HANDLE,
};
use windows::Win32::NetworkManagement::WiFi::{
    dot11_BSS_type_any, wlan_interface_state_connected, wlan_intf_opcode_channel_number,
    wlan_intf_opcode_current_connection, wlan_intf_opcode_rssi, WlanCloseHandle,
    WlanEnumInterfaces, WlanFreeMemory, WlanGetNetworkBssList, WlanOpenHandle, WlanQueryInterface,
    WlanScan, DOT11_SSID, WLAN_BSS_ENTRY, WLAN_BSS_LIST, WLAN_CONNECTION_ATTRIBUTES,
    WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST, WLAN_INTERFACE_STATE, WLAN_INTF_OPCODE,
};

use crate::{
    channel_from_mhz, channel_width_mhz, phy_label, rssi_from_quality, security_label, MacAddr,
    NearbyNetwork, PlatformError, WifiBand, WifiConnection, LOCATION_PERMISSION,
};

const ACCESS_DENIED: u32 = ERROR_ACCESS_DENIED.0;
const INVALID_STATE: u32 = ERROR_INVALID_STATE.0;
const SERVICE_NOT_ACTIVE: u32 = ERROR_SERVICE_NOT_ACTIVE.0;

/// The WLAN API client version for Windows Vista and later.
const CLIENT_VERSION: u32 = 2;

/// Turn a WLAN API return code into a `PlatformError`.
fn map_wlan_error(code: u32) -> PlatformError {
    match code {
        ACCESS_DENIED => PlatformError::PermissionNeeded {
            what: LOCATION_PERMISSION.into(),
        },
        SERVICE_NOT_ACTIVE => PlatformError::Unavailable {
            reason: "the WLAN AutoConfig service is not running".into(),
        },
        other => PlatformError::Io {
            message: format!("WLAN API error {other}"),
        },
    }
}

/// Closes a WLAN client handle on every exit path.
struct Client(HANDLE);

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful WlanOpenHandle and is closed
        // exactly once, here.
        unsafe { WlanCloseHandle(self.0, None) };
    }
}

/// Frees memory the WLAN API allocated, on every exit path.
struct WlanMemory(*mut c_void);

impl Drop for WlanMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a WLAN API call that allocates its
            // output, and is freed exactly once, here.
            unsafe { WlanFreeMemory(self.0) };
        }
    }
}

fn open() -> Result<Client, PlatformError> {
    let mut negotiated = 0u32;
    let mut handle = HANDLE::default();
    // SAFETY: both out-pointers are valid locals; on success Windows hands back
    // a client handle that `Client` closes.
    let rc = unsafe { WlanOpenHandle(CLIENT_VERSION, None, &mut negotiated, &mut handle) };
    if rc != 0 {
        return Err(map_wlan_error(rc));
    }
    Ok(Client(handle))
}

/// The GUID and state of every wireless interface.
fn interfaces(client: &Client) -> Result<Vec<(GUID, WLAN_INTERFACE_STATE)>, PlatformError> {
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
    // SAFETY: the handle is open and `list` is a valid out-pointer; the list is
    // freed by the guard below.
    let rc = unsafe { WlanEnumInterfaces(client.0, None, &mut list) };
    let _guard = WlanMemory(list.cast());
    if rc != 0 {
        return Err(map_wlan_error(rc));
    }
    if list.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: the call succeeded, so `list` points to a WLAN_INTERFACE_INFO_LIST
    // whose `dwNumberOfItems` entries follow contiguously (a flexible array),
    // alive until `_guard` drops. `addr_of!` avoids a reference to the
    // one-element array so the pointer stays valid for every entry.
    let infos = unsafe {
        let first = std::ptr::addr_of!((*list).InterfaceInfo).cast::<WLAN_INTERFACE_INFO>();
        std::slice::from_raw_parts(first, (*list).dwNumberOfItems as usize)
    };
    Ok(infos.iter().map(|i| (i.InterfaceGuid, i.isState)).collect())
}

/// The first wireless interface in any state, for scans.
fn first_interface(client: &Client) -> Result<GUID, PlatformError> {
    interfaces(client)?
        .first()
        .map(|(guid, _)| *guid)
        .ok_or_else(|| PlatformError::Unavailable {
            reason: "no Wi-Fi adapter".into(),
        })
}

/// `WlanQueryInterface` for one opcode, copied out as `T`. The error is the raw
/// WLAN code so callers can treat some codes specially.
fn query<T: Copy>(client: &Client, guid: &GUID, opcode: WLAN_INTF_OPCODE) -> Result<T, u32> {
    let mut size = 0u32;
    let mut data: *mut c_void = std::ptr::null_mut();
    // SAFETY: the handle is open, `guid` is a live GUID and both out-pointers are
    // valid locals; the returned buffer is freed by the guard below.
    let rc =
        unsafe { WlanQueryInterface(client.0, guid, opcode, None, &mut size, &mut data, None) };
    let _guard = WlanMemory(data);
    if rc != 0 {
        return Err(rc);
    }
    if data.is_null() || (size as usize) < std::mem::size_of::<T>() {
        return Err(ERROR_INVALID_DATA.0);
    }
    // SAFETY: the buffer holds at least `size_of::<T>()` bytes of the struct
    // documented for this opcode; `read_unaligned` makes no alignment assumption.
    Ok(unsafe { data.cast::<T>().read_unaligned() })
}

/// One BSS list entry, copied out of WLAN memory.
struct Bss {
    ssid: String,
    bssid: MacAddr,
    rssi_dbm: i32,
    mhz: u32,
    width_mhz: Option<u32>,
}

fn ssid_text(ssid: &DOT11_SSID) -> String {
    let len = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
    String::from_utf8_lossy(&ssid.ucSSID[..len]).into_owned()
}

/// Every BSS the interface last saw, via `WlanGetNetworkBssList`.
fn bss_list(client: &Client, guid: &GUID) -> Result<Vec<Bss>, PlatformError> {
    let mut list: *mut WLAN_BSS_LIST = std::ptr::null_mut();
    // SAFETY: the handle is open, `guid` is a live GUID and `list` is a valid
    // out-pointer; the list is freed by the guard below.
    let rc = unsafe {
        WlanGetNetworkBssList(
            client.0,
            guid,
            None,
            dot11_BSS_type_any,
            false,
            None,
            &mut list,
        )
    };
    let _guard = WlanMemory(list.cast());
    if rc != 0 {
        return Err(map_wlan_error(rc));
    }
    if list.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: the call succeeded, so `list` points to a WLAN_BSS_LIST of
    // `dwTotalSize` bytes whose `dwNumberOfItems` entries follow contiguously,
    // alive until `_guard` drops. `addr_of!` keeps the pointer valid for every
    // entry, not only the declared one-element array.
    let (total, entries) = unsafe {
        let first = std::ptr::addr_of!((*list).wlanBssEntries).cast::<WLAN_BSS_ENTRY>();
        (
            (*list).dwTotalSize as usize,
            std::slice::from_raw_parts(first, (*list).dwNumberOfItems as usize),
        )
    };
    let base = list as usize;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        // The IE blob's offset is relative to the entry; read it only when it
        // lies inside the list's reported size.
        let start = entry as *const WLAN_BSS_ENTRY as usize - base + entry.ulIeOffset as usize;
        let len = entry.ulIeSize as usize;
        let width_mhz = if len > 0 && start + len <= total {
            // SAFETY: `start..start + len` lies within the `dwTotalSize` bytes of
            // the list allocation, which outlives this slice.
            let ies = unsafe { std::slice::from_raw_parts((base + start) as *const u8, len) };
            channel_width_mhz(ies)
        } else {
            None
        };
        out.push(Bss {
            ssid: ssid_text(&entry.dot11Ssid),
            bssid: MacAddr(entry.dot11Bssid),
            rssi_dbm: entry.lRssi,
            mhz: entry.ulChCenterFrequency / 1000,
            width_mhz,
        });
    }
    Ok(out)
}

/// The connection of the first connected wireless interface, or `None` when
/// no interface is connected (or there is no adapter at all).
pub fn wifi_current_blocking() -> Result<Option<WifiConnection>, PlatformError> {
    let client = open()?;
    let Some(guid) = interfaces(&client)?
        .into_iter()
        .find(|(_, state)| *state == wlan_interface_state_connected)
        .map(|(guid, _)| guid)
    else {
        return Ok(None);
    };

    let attrs: WLAN_CONNECTION_ATTRIBUTES =
        match query(&client, &guid, wlan_intf_opcode_current_connection) {
            Ok(a) => a,
            Err(INVALID_STATE) => return Ok(None),
            Err(code) => return Err(map_wlan_error(code)),
        };
    let assoc = attrs.wlanAssociationAttributes;
    let rssi_dbm = query::<i32>(&client, &guid, wlan_intf_opcode_rssi)
        .unwrap_or_else(|_| rssi_from_quality(assoc.wlanSignalQuality));
    let channel: u32 =
        query(&client, &guid, wlan_intf_opcode_channel_number).map_err(map_wlan_error)?;

    // Width and frequency come from the matching BSS entry; a denied or missing
    // BSS list only costs the width.
    let bssid = MacAddr(assoc.dot11Bssid);
    let bss = bss_list(&client, &guid)
        .ok()
        .and_then(|list| list.into_iter().find(|b| b.bssid == bssid));
    let band = match bss.as_ref().and_then(|b| channel_from_mhz(b.mhz)) {
        Some((band, _)) => band,
        None if (1..=14).contains(&channel) => WifiBand::Ghz2_4,
        None => WifiBand::Ghz5,
    };
    let rate = |kbps: u32| (kbps > 0).then_some(kbps / 1000);

    Ok(Some(WifiConnection {
        ssid: ssid_text(&assoc.dot11Ssid),
        bssid,
        rssi_dbm,
        channel,
        band,
        width_mhz: bss.and_then(|b| b.width_mhz),
        phy: phy_label(assoc.dot11PhyType.0 as u32, band).into(),
        rx_rate_mbps: rate(assoc.ulRxRate),
        tx_rate_mbps: rate(assoc.ulTxRate),
        security: security_label(attrs.wlanSecurityAttributes.dot11AuthAlgorithm.0 as u32).into(),
    }))
}

/// Ask the first wireless interface to scan. Returns at once; results land in
/// the BSS list a few seconds later.
pub fn wifi_scan_start_blocking() -> Result<(), PlatformError> {
    let client = open()?;
    let guid = first_interface(&client)?;
    // SAFETY: the handle is open and `guid` is a live GUID; no SSID or IE data.
    let rc = unsafe { WlanScan(client.0, &guid, None, None, None) };
    if rc != 0 {
        return Err(map_wlan_error(rc));
    }
    Ok(())
}

/// The nearby networks the first wireless interface last saw. Entries whose
/// frequency maps to no channel are skipped.
pub fn wifi_bss_list_blocking() -> Result<Vec<NearbyNetwork>, PlatformError> {
    let client = open()?;
    let guid = first_interface(&client)?;
    Ok(bss_list(&client, &guid)?
        .into_iter()
        .filter_map(|b| {
            let (band, channel) = channel_from_mhz(b.mhz)?;
            Some(NearbyNetwork {
                ssid: b.ssid,
                bssid: b.bssid,
                rssi_dbm: b.rssi_dbm,
                channel,
                band,
                width_mhz: b.width_mhz,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wlan_error_codes_map_to_platform_errors() {
        assert_eq!(
            map_wlan_error(5),
            PlatformError::PermissionNeeded {
                what: "location".into()
            }
        );
        assert_eq!(
            map_wlan_error(1062),
            PlatformError::Unavailable {
                reason: "the WLAN AutoConfig service is not running".into()
            }
        );
        assert_eq!(
            map_wlan_error(87),
            PlatformError::Io {
                message: "WLAN API error 87".into()
            }
        );
    }
}
