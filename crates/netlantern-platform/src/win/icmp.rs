use std::net::Ipv4Addr;
use std::time::Duration;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho, ICMP_ECHO_REPLY, IP_SUCCESS,
};

use crate::{PingReply, PlatformError};

const PAYLOAD: [u8; 32] = [0x61; 32];

/// Closes an ICMP handle on every exit path.
struct IcmpHandle(HANDLE);

impl Drop for IcmpHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from a successful IcmpCreateFile and is
        // closed exactly once, here.
        let _ = unsafe { IcmpCloseHandle(self.0) };
    }
}

/// One blocking ICMP echo via `IcmpSendEcho`. Every per-address failure
/// (timeout, unreachable, any non-success status, wrong sender) is `Ok(None)`;
/// only a failed `IcmpCreateFile` is an error.
pub fn ping_blocking(ip: Ipv4Addr, timeout: Duration) -> Result<Option<PingReply>, PlatformError> {
    // SAFETY: no arguments; returns a handle or an error.
    let handle = unsafe { IcmpCreateFile() }.map_err(|e| PlatformError::Io {
        message: format!("IcmpCreateFile failed: {e}"),
    })?;
    let handle = IcmpHandle(handle);

    // One reply, the echoed payload, and 8 bytes for an ICMP error. A u64
    // buffer gives the 8-byte alignment ICMP_ECHO_REPLY needs.
    let bytes = std::mem::size_of::<ICMP_ECHO_REPLY>() + PAYLOAD.len() + 8;
    let mut buf = vec![0u64; bytes.div_ceil(8)];
    let target = u32::from_ne_bytes(ip.octets());
    let timeout_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);

    // SAFETY: the handle is valid; the request data is a live 32-byte array;
    // `buf` is writable for `bytes` bytes (its length in u8 is at least that)
    // and outlives the call, which is synchronous.
    let n = unsafe {
        IcmpSendEcho(
            handle.0,
            target,
            PAYLOAD.as_ptr().cast(),
            PAYLOAD.len() as u16,
            None,
            buf.as_mut_ptr().cast(),
            bytes as u32,
            timeout_ms,
        )
    };
    if n == 0 {
        return Ok(None);
    }

    // SAFETY: IcmpSendEcho returned at least one reply, so the start of `buf`
    // holds a valid, 8-byte-aligned ICMP_ECHO_REPLY.
    let reply = unsafe { &*buf.as_ptr().cast::<ICMP_ECHO_REPLY>() };
    if reply.Status != IP_SUCCESS || reply.Address != target {
        return Ok(None);
    }
    Ok(Some(PingReply {
        rtt: Duration::from_millis(u64::from(reply.RoundTripTime)),
        ttl: reply.Options.Ttl,
    }))
}
