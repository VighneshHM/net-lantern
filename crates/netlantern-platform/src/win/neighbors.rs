use std::net::Ipv4Addr;

use windows::Win32::Foundation::{ERROR_NOT_FOUND, NO_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetIpNetTable2, MIB_IPNET_ROW2, MIB_IPNET_TABLE2,
};
use windows::Win32::Networking::WinSock::AF_INET;

use crate::iface_select::{neighbor_mac, neighbor_state};
use crate::{ArpEntry, PlatformError};

/// Frees a table returned by `GetIpNetTable2` on every exit path.
struct TableGuard(*mut MIB_IPNET_TABLE2);

impl Drop for TableGuard {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a successful GetIpNetTable2 call and is
        // freed exactly once, here.
        unsafe { FreeMibTable(self.0.cast()) };
    }
}

/// Read the IPv4 neighbour (ARP) table for one interface via `GetIpNetTable2`.
///
/// Incomplete and Unreachable rows are kept with an all-zero MAC: the scan
/// treats them as "already tried", not as devices. Rows in any other state with
/// a MAC that is not 6 bytes or all zeros, and rows with an unknown state, are
/// skipped. No admin needed.
pub fn read_neighbors(if_index: u32) -> Result<Vec<ArpEntry>, PlatformError> {
    let mut table: *mut MIB_IPNET_TABLE2 = std::ptr::null_mut();
    // SAFETY: `table` is a valid out-pointer; on success Windows allocates a table
    // that is freed with `FreeMibTable` below.
    let rc = unsafe { GetIpNetTable2(AF_INET, &mut table) };
    if rc == ERROR_NOT_FOUND {
        return Ok(Vec::new());
    }
    if rc != NO_ERROR {
        return Err(PlatformError::Io {
            message: format!("GetIpNetTable2 failed with code {}", rc.0),
        });
    }
    if table.is_null() {
        return Ok(Vec::new());
    }

    let _guard = TableGuard(table);
    let mut out = Vec::new();
    // SAFETY: the call succeeded, so `table` points to a valid MIB_IPNET_TABLE2
    // whose `NumEntries` rows follow contiguously (a flexible array); it stays
    // alive until `_guard` drops. The row pointer is derived with `addr_of!` so no
    // reference to the one-element `Table` array is created, which keeps the
    // pointer valid for all `NumEntries` rows.
    let rows = unsafe {
        let first = std::ptr::addr_of!((*table).Table).cast::<MIB_IPNET_ROW2>();
        std::slice::from_raw_parts(first, (*table).NumEntries as usize)
    };
    for row in rows {
        if row.InterfaceIndex != if_index {
            continue;
        }
        let Some(state) = neighbor_state(row.State.0) else {
            continue;
        };
        let len = row.PhysicalAddressLength;
        let Some(mac) = neighbor_mac(len, &row.PhysicalAddress[..(len as usize).min(32)], state)
        else {
            continue;
        };
        // SAFETY: we requested AF_INET, so the `Ipv4` arm of the address union is valid.
        let raw = unsafe { row.Address.Ipv4.sin_addr.S_un.S_addr };
        out.push(ArpEntry {
            ip: Ipv4Addr::from(u32::from_be(raw)),
            mac,
            state,
        });
    }
    Ok(out)
}
