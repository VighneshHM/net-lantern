//! Minimal DNS client: one query over UDP from an ephemeral port.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query, ResponseCode};
use hickory_proto::rr::{Name, RData, RecordType};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::io_error;
use crate::PlatformError;

/// How long each attempt waits for a reply.
pub(crate) const DNS_TIMEOUT: Duration = Duration::from_secs(1);
/// First attempt plus one retry.
const ATTEMPTS: usize = 2;

/// `61.1.168.192.in-addr.arpa.` for 192.168.1.61.
pub fn ptr_name(ip: Ipv4Addr) -> String {
    let [a, b, c, d] = ip.octets();
    format!("{d}.{c}.{b}.{a}.in-addr.arpa.")
}

/// Send `query` to `server` and return the matching reply.
///
/// The message id is replaced with a random one; replies with another id, or
/// from another address, are ignored. Retries once; a silent server is an
/// `Io` error.
pub(crate) async fn dns_query(
    server: SocketAddr,
    mut query: Message,
    timeout: Duration,
) -> Result<Message, PlatformError> {
    let id: u16 = rand::random();
    query.metadata.id = id;
    let bytes = query.to_vec().map_err(io_error)?;

    let bind: SocketAddr = if server.is_ipv4() {
        (Ipv4Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    };
    let socket = UdpSocket::bind(bind).await.map_err(io_error)?;

    let mut buf = [0u8; 4096];
    for _ in 0..ATTEMPTS {
        socket.send_to(&bytes, server).await.map_err(io_error)?;
        let deadline = Instant::now() + timeout;
        loop {
            let received = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await;
            let (n, from) = match received {
                Err(_) => break,
                Ok(r) => r.map_err(io_error)?,
            };
            if from != server {
                continue;
            }
            if let Ok(msg) = Message::from_vec(&buf[..n]) {
                if msg.metadata.id == id && msg.metadata.message_type == MessageType::Response {
                    return Ok(msg);
                }
            }
        }
    }
    Err(io_error(format!("DNS query to {server} timed out")))
}

fn question(name: &str, record_type: RecordType) -> Result<Message, PlatformError> {
    let name = Name::from_ascii(name).map_err(io_error)?;
    let mut msg = Message::new(0, MessageType::Query, OpCode::Query);
    msg.metadata.recursion_desired = true;
    msg.add_query(Query::query(name, record_type));
    Ok(msg)
}

/// A-record lookup. NXDOMAIN gives an empty list.
pub(crate) async fn dns_resolve(
    server: SocketAddr,
    name: &str,
) -> Result<Vec<Ipv4Addr>, PlatformError> {
    let reply = dns_query(server, question(name, RecordType::A)?, DNS_TIMEOUT).await?;
    if reply.metadata.response_code == ResponseCode::NXDomain {
        return Ok(Vec::new());
    }
    Ok(reply
        .answers
        .iter()
        .filter_map(|r| match &r.data {
            RData::A(a) => Some(a.0),
            _ => None,
        })
        .collect())
}

/// PTR lookup. Silence, NXDOMAIN or no answer is `None`.
pub(crate) async fn reverse_dns(server: SocketAddr, ip: Ipv4Addr) -> Option<String> {
    let query = question(&ptr_name(ip), RecordType::PTR).ok()?;
    let reply = dns_query(server, query, DNS_TIMEOUT).await.ok()?;
    reply.answers.iter().find_map(|r| match &r.data {
        RData::PTR(ptr) => {
            let name = ptr.0.to_utf8();
            Some(name.trim_end_matches('.').to_string())
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptr_name_reverses_octets() {
        assert_eq!(
            ptr_name(Ipv4Addr::new(192, 168, 1, 61)),
            "61.1.168.192.in-addr.arpa."
        );
    }

    #[tokio::test]
    async fn replies_with_another_id_are_ignored() {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let server = sock.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let (n, from) = sock.recv_from(&mut buf).await.unwrap();
            let query = Message::from_vec(&buf[..n]).unwrap();
            let mut wrong = Message::response(query.metadata.id.wrapping_add(1), OpCode::Query);
            wrong.queries = query.queries.clone();
            sock.send_to(&wrong.to_vec().unwrap(), from).await.unwrap();
            let mut right = Message::response(query.metadata.id, OpCode::Query);
            right.queries = query.queries.clone();
            right.metadata.authoritative = true;
            sock.send_to(&right.to_vec().unwrap(), from).await.unwrap();
        });
        let q = question("x.test.", RecordType::A).unwrap();
        let reply = dns_query(server, q, Duration::from_secs(1)).await.unwrap();
        assert!(reply.metadata.authoritative);
    }
}
