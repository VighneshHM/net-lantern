//! Where the phone listener binds: the LAN address for real, loopback for
//! `--demo` and tests.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};

use async_trait::async_trait;
use tokio::net::TcpListener;

use crate::serve::DEFAULT_PORTS;

#[async_trait]
pub trait LanBinder: Send + Sync {
    /// Bind a listener for phones. Returns the listener and the address to put in the URL.
    async fn bind(
        &self,
        lan_ip: Ipv4Addr,
        preferred_port: u16,
    ) -> io::Result<(TcpListener, SocketAddr)>;
}

/// Binds `(lan_ip, preferred_port)`, else the first free port in
/// [`DEFAULT_PORTS`]. Only ever that one address: never the unspecified
/// address (every interface) or loopback.
pub struct RealLanBinder;

#[async_trait]
impl LanBinder for RealLanBinder {
    async fn bind(
        &self,
        lan_ip: Ipv4Addr,
        preferred_port: u16,
    ) -> io::Result<(TcpListener, SocketAddr)> {
        if lan_ip.is_unspecified() || lan_ip.is_loopback() || lan_ip.is_broadcast() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a LAN address to listen on",
            ));
        }
        let mut last = None;
        for port in std::iter::once(preferred_port).chain(DEFAULT_PORTS) {
            match TcpListener::bind((lan_ip, port)).await {
                Ok(listener) => {
                    let addr = listener.local_addr()?;
                    return Ok((listener, addr));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::AddrInUse, "no free port")))
    }
}

/// For `--demo` and tests: binds `127.0.0.1:0`, whatever the "LAN" address,
/// so the phone URL works on this laptop only.
pub struct LoopbackLanBinder;

#[async_trait]
impl LanBinder for LoopbackLanBinder {
    async fn bind(&self, _: Ipv4Addr, _: u16) -> io::Result<(TcpListener, SocketAddr)> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        Ok((listener, addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loopback_binder_binds_loopback_whatever_the_lan_address() {
        let (listener, addr) = LoopbackLanBinder
            .bind(Ipv4Addr::new(192, 168, 1, 23), 47800)
            .await
            .unwrap();
        assert!(addr.ip().is_loopback());
        assert_eq!(listener.local_addr().unwrap(), addr);
    }

    #[tokio::test]
    async fn real_binder_refuses_non_lan_addresses() {
        for ip in [
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::BROADCAST,
        ] {
            let err = RealLanBinder.bind(ip, 0).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{ip}");
        }
    }

    #[tokio::test]
    async fn real_binder_reports_an_address_this_machine_does_not_have() {
        // A documentation address (TEST-NET-3) is on no interface: every
        // port fails, and the last bind error comes back.
        let err = RealLanBinder
            .bind(Ipv4Addr::new(203, 0, 113, 7), 47800)
            .await
            .unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
