use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use tokio::net::TcpStream;

use crate::TcpProbe;

/// Portable TCP connect probe: accepted is `Open`, refused is `Closed`, and
/// a timeout or any other error is `Filtered`.
pub async fn tcp_connect_probe(ip: Ipv4Addr, port: u16, timeout: Duration) -> TcpProbe {
    let addr = SocketAddrV4::new(ip, port);
    match tokio::time::timeout(timeout, TcpStream::connect(addr)).await {
        Ok(Ok(_)) => TcpProbe::Open,
        Ok(Err(e)) if e.kind() == ErrorKind::ConnectionRefused => TcpProbe::Closed,
        _ => TcpProbe::Filtered,
    }
}
