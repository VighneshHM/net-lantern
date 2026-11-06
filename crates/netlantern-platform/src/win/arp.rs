use std::net::Ipv4Addr;

use windows::Win32::Foundation::NO_ERROR;
use windows::Win32::NetworkManagement::IpHelper::SendARP;

use crate::MacAddr;

/// One blocking `SendARP` resolve. Any failure code is `None`.
pub fn arp_resolve_blocking(ip: Ipv4Addr) -> Option<MacAddr> {
    let dest = u32::from_ne_bytes(ip.octets());
    let mut mac = [0u32; 2];
    let mut len: u32 = 6;
    // SAFETY: `mac` is 8 writable bytes and `len` says 6 are expected; both are
    // valid for the duration of this synchronous call.
    let rc = unsafe { SendARP(dest, 0, mac.as_mut_ptr().cast(), &mut len) };
    if rc != NO_ERROR.0 || len != 6 {
        return None;
    }
    let raw = mac[0].to_ne_bytes();
    let raw2 = mac[1].to_ne_bytes();
    let m = [raw[0], raw[1], raw[2], raw[3], raw2[0], raw2[1]];
    if m == [0u8; 6] {
        return None;
    }
    Some(MacAddr(m))
}
