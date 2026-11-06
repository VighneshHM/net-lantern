use std::net::Ipv4Addr;

use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, ERROR_SUCCESS};
use windows::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR, SOCKADDR_IN};

use crate::iface_select::AdapterInfo;
use crate::{MacAddr, PlatformError};

/// How many times to grow the buffer when the adapter list changes under us.
const MAX_ATTEMPTS: usize = 5;

/// Read every IPv4-enabled adapter via `GetAdaptersAddresses`. No admin needed.
pub fn read_adapters() -> Result<Vec<AdapterInfo>, PlatformError> {
    let flags = GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    let mut size: u32 = 16 * 1024;
    for _ in 0..MAX_ATTEMPTS {
        // u64 elements keep the buffer 8-byte aligned for IP_ADAPTER_ADDRESSES_LH.
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let head = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `head` points to a zeroed, suitably aligned buffer of at least
        // `size` bytes, and `size` is a valid in/out length for the call.
        let rc = unsafe {
            GetAdaptersAddresses(u32::from(AF_INET.0), flags, None, Some(head), &mut size)
        };
        match rc {
            r if r == ERROR_SUCCESS.0 => {
                // SAFETY: the call succeeded, so `head` starts a valid linked list
                // whose nodes and pointees live inside `buf`, which outlives this.
                return Ok(unsafe { collect(head) });
            }
            r if r == ERROR_NO_DATA.0 => return Ok(Vec::new()),
            // `size` now holds the length needed; go round again.
            r if r == ERROR_BUFFER_OVERFLOW.0 => continue,
            r => {
                return Err(PlatformError::Io {
                    message: format!("GetAdaptersAddresses failed with code {r}"),
                })
            }
        }
    }
    Err(PlatformError::Unavailable {
        reason: "the adapter list kept changing while it was being read".into(),
    })
}

/// Walk the adapter list into owned [`AdapterInfo`]s.
///
/// # Safety
/// `head` must be the start of a list returned by a successful
/// `GetAdaptersAddresses` call whose buffer is still alive.
unsafe fn collect(head: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<AdapterInfo> {
    let mut out = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` is non-null and points to a node of the live list.
        let a = unsafe { &*cur };

        let mut ipv4 = Vec::new();
        let mut uni = a.FirstUnicastAddress;
        while !uni.is_null() {
            // SAFETY: non-null node of the unicast list inside the live buffer.
            let u = unsafe { &*uni };
            // SAFETY: the node's sockaddr pointer is null or valid per the API.
            if let Some(ip) = unsafe { sockaddr_v4(u.Address.lpSockaddr) } {
                ipv4.push((ip, u.OnLinkPrefixLength));
            }
            uni = u.Next;
        }

        let mut gateways = Vec::new();
        let mut gw = a.FirstGatewayAddress;
        while !gw.is_null() {
            // SAFETY: non-null node of the gateway list inside the live buffer.
            let g = unsafe { &*gw };
            // SAFETY: the node's sockaddr pointer is null or valid per the API.
            if let Some(ip) = unsafe { sockaddr_v4(g.Address.lpSockaddr) } {
                gateways.push(ip);
            }
            gw = g.Next;
        }

        let mut dns = Vec::new();
        let mut ds = a.FirstDnsServerAddress;
        while !ds.is_null() {
            // SAFETY: non-null node of the DNS list inside the live buffer.
            let d = unsafe { &*ds };
            // SAFETY: the node's sockaddr pointer is null or valid per the API.
            if let Some(ip) = unsafe { sockaddr_v4(d.Address.lpSockaddr) } {
                dns.push(ip);
            }
            ds = d.Next;
        }

        let mac = (a.PhysicalAddressLength == 6).then(|| {
            let mut m = [0u8; 6];
            m.copy_from_slice(&a.PhysicalAddress[..6]);
            MacAddr(m)
        });

        // SAFETY: `Anonymous1.Anonymous` is the struct arm of the header union and
        // is always valid for entries returned by this API.
        let if_index = unsafe { a.Anonymous1.Anonymous.IfIndex };

        out.push(AdapterInfo {
            if_index,
            if_type: a.IfType,
            up: a.OperStatus == IfOperStatusUp,
            ipv4,
            gateways,
            dns,
            mac,
            metric: a.Ipv4Metric,
        });
        cur = a.Next;
    }
    out
}

/// Read an IPv4 address out of a `SOCKADDR`, or `None` for null / non-IPv4.
///
/// # Safety
/// `sa` must be null or point to a valid `SOCKADDR` of the size its family implies.
unsafe fn sockaddr_v4(sa: *const SOCKADDR) -> Option<Ipv4Addr> {
    if sa.is_null() {
        return None;
    }
    // SAFETY: non-null and valid per the function contract.
    if unsafe { (*sa).sa_family } != AF_INET {
        return None;
    }
    // SAFETY: the family is AF_INET, so the pointee is a SOCKADDR_IN.
    let sin = unsafe { &*sa.cast::<SOCKADDR_IN>() };
    // SAFETY: reading the `S_addr` arm of the address union; all arms are plain integers.
    let raw = unsafe { sin.sin_addr.S_un.S_addr };
    Some(Ipv4Addr::from(u32::from_be(raw)))
}
