//! The portable [`NetIo`] implementation: DNS over UDP, reverse DNS, HTTP and
//! LAN discovery (SSDP, NetBIOS node status, one-shot mDNS).

pub(crate) mod dns;
mod http;
mod mdns;
mod netbios;
mod ssdp;

use std::net::Ipv4Addr;
use std::time::Duration;

use async_trait::async_trait;

use crate::netio::{HttpResponse, MdnsService, NetIo, NetbiosInfo, SsdpReply};
use crate::PlatformError;

pub use dns::ptr_name;

/// Real network I/O. Works on every platform; nothing here is OS specific.
#[derive(Clone, Debug)]
pub struct RealNetIo {
    dns_port: u16,
    netbios_port: u16,
}

impl RealNetIo {
    pub fn new() -> Self {
        Self::with_ports(53, 137)
    }

    /// Use non-standard DNS and NetBIOS ports (tests point these at loopback fakes).
    pub fn with_ports(dns: u16, netbios: u16) -> Self {
        Self {
            dns_port: dns,
            netbios_port: netbios,
        }
    }
}

impl Default for RealNetIo {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn io_error(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Io {
        message: e.to_string(),
    }
}

/// A UDP socket bound to `local:0` that sends multicast out of `local`.
pub(crate) fn multicast_socket(
    local: Ipv4Addr,
    ttl: u32,
) -> Result<tokio::net::UdpSocket, PlatformError> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)).map_err(io_error)?;
    socket
        .bind(&std::net::SocketAddr::from((local, 0)).into())
        .map_err(io_error)?;
    socket.set_multicast_if_v4(&local).map_err(io_error)?;
    socket.set_multicast_ttl_v4(ttl).map_err(io_error)?;
    socket.set_nonblocking(true).map_err(io_error)?;
    tokio::net::UdpSocket::from_std(socket.into()).map_err(io_error)
}

#[async_trait]
impl NetIo for RealNetIo {
    async fn dns_resolve(
        &self,
        server: Ipv4Addr,
        name: &str,
    ) -> Result<Vec<Ipv4Addr>, PlatformError> {
        dns::dns_resolve((server, self.dns_port).into(), name).await
    }

    async fn http_get(
        &self,
        url: &str,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PlatformError> {
        http::http_get(url, follow_redirects).await
    }

    async fn reverse_dns(
        &self,
        server: Ipv4Addr,
        ip: Ipv4Addr,
    ) -> Result<Option<String>, PlatformError> {
        Ok(dns::reverse_dns((server, self.dns_port).into(), ip).await)
    }

    async fn mdns_browse(
        &self,
        local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<MdnsService>, PlatformError> {
        mdns::mdns_browse(local, window).await
    }

    async fn ssdp_search(
        &self,
        local: Ipv4Addr,
        window: Duration,
    ) -> Result<Vec<SsdpReply>, PlatformError> {
        ssdp::ssdp_search(local, window).await
    }

    async fn netbios_status(
        &self,
        ip: Ipv4Addr,
        timeout: Duration,
    ) -> Result<Option<NetbiosInfo>, PlatformError> {
        netbios::netbios_status(ip, self.netbios_port, timeout).await
    }
}
