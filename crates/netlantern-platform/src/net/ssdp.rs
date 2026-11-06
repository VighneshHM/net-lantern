//! SSDP (UPnP) discovery: one M-SEARCH from an ephemeral port, replies by unicast.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::{io_error, multicast_socket};
use crate::netio::SsdpReply;
use crate::PlatformError;

const SSDP_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
const RESEND_AFTER: Duration = Duration::from_millis(500);

/// The M-SEARCH request that asks every UPnP device and service to answer.
pub fn msearch_request() -> &'static str {
    "M-SEARCH * HTTP/1.1\r\n\
     HOST: 239.255.255.250:1900\r\n\
     MAN: \"ssdp:discover\"\r\n\
     MX: 2\r\n\
     ST: ssdp:all\r\n\
     \r\n"
}

/// Parse an SSDP response into `(st, server, location)`. Anything that is not
/// an HTTP 200 carrying an `ST` header is dropped.
pub fn parse_ssdp_response(text: &str) -> Option<(String, Option<String>, Option<String>)> {
    let mut lines = text.split("\r\n");
    let status = lines.next()?;
    let mut parts = status.split_whitespace();
    if !parts.next()?.to_ascii_uppercase().starts_with("HTTP/1.") || parts.next()? != "200" {
        return None;
    }
    let (mut st, mut server, mut location) = (None, None, None);
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match name.trim().to_ascii_lowercase().as_str() {
            "st" => st = Some(value),
            "server" => server = Some(value),
            "location" => location = Some(value),
            _ => {}
        }
    }
    Some((st?, server, location))
}

pub(crate) async fn ssdp_search(
    local: Ipv4Addr,
    window: Duration,
) -> Result<Vec<SsdpReply>, PlatformError> {
    search(local, window, SSDP_GROUP.into()).await
}

async fn search(
    local: Ipv4Addr,
    window: Duration,
    dest: SocketAddr,
) -> Result<Vec<SsdpReply>, PlatformError> {
    let socket = multicast_socket(local, 2)?;
    let start = Instant::now();
    let deadline = start + window;
    send(&socket, dest).await?;
    let mut resend_at = Some(start + RESEND_AFTER).filter(|t| *t < deadline);

    let mut replies = Vec::new();
    let mut seen: HashSet<(Ipv4Addr, String)> = HashSet::new();
    let mut buf = [0u8; 4096];
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            _ = sleep_opt(resend_at) => {
                resend_at = None;
                send(&socket, dest).await?;
            }
            received = socket.recv_from(&mut buf) => {
                let Ok((n, SocketAddr::V4(from))) = received else { continue };
                let text = String::from_utf8_lossy(&buf[..n]);
                let Some((st, server, location)) = parse_ssdp_response(&text) else { continue };
                if seen.insert((*from.ip(), st.clone())) {
                    replies.push(SsdpReply { ip: *from.ip(), st, server, location });
                }
            }
        }
    }
    Ok(replies)
}

async fn send(socket: &UdpSocket, dest: SocketAddr) -> Result<(), PlatformError> {
    socket
        .send_to(msearch_request().as_bytes(), dest)
        .await
        .map_err(io_error)?;
    Ok(())
}

async fn sleep_opt(at: Option<Instant>) {
    match at {
        Some(t) => tokio::time::sleep_until(t).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_has_required_headers_and_crlf() {
        let r = msearch_request();
        assert!(r.starts_with("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\n"));
        assert!(r.contains("MAN: \"ssdp:discover\"\r\n"));
        assert!(r.contains("MX: 2\r\n"));
        assert!(r.contains("ST: ssdp:all\r\n"));
        assert!(r.ends_with("\r\n\r\n"));
        assert!(!r.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn parses_ssdp_reply_case_insensitively() {
        let r = "HTTP/1.1 200 OK\r\nst: upnp:rootdevice\r\nSERVER: Hue/1.0 UPnP/1.0 IpBridge\r\nLocation: http://192.168.1.130/description.xml\r\n\r\n";
        assert_eq!(
            parse_ssdp_response(r),
            Some((
                "upnp:rootdevice".into(),
                Some("Hue/1.0 UPnP/1.0 IpBridge".into()),
                Some("http://192.168.1.130/description.xml".into())
            ))
        );
        assert_eq!(
            parse_ssdp_response("NOTIFY * HTTP/1.1\r\nNT: x\r\n\r\n"),
            None
        );
    }

    #[test]
    fn reply_without_st_or_garbage_is_dropped() {
        assert_eq!(
            parse_ssdp_response("HTTP/1.1 200 OK\r\nSERVER: x\r\n\r\n"),
            None
        );
        assert_eq!(parse_ssdp_response(""), None);
        assert_eq!(parse_ssdp_response("\u{fffd}\u{fffd}"), None);
        assert_eq!(
            parse_ssdp_response("HTTP/1.1 404 Not Found\r\nST: a\r\n\r\n"),
            None
        );
    }

    #[tokio::test]
    async fn collects_and_deduplicates_replies_from_a_responder() {
        let responder = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let dest = responder.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            let Ok((_, from)) = responder.recv_from(&mut buf).await else {
                return;
            };
            let reply = "HTTP/1.1 200 OK\r\nST: upnp:rootdevice\r\nSERVER: x\r\n\r\n";
            for _ in 0..2 {
                responder.send_to(reply.as_bytes(), from).await.unwrap();
            }
            responder.send_to(b"junk", from).await.unwrap();
        });
        let got = search(Ipv4Addr::LOCALHOST, Duration::from_millis(700), dest)
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].ip, Ipv4Addr::LOCALHOST);
        assert_eq!(got[0].st, "upnp:rootdevice");
    }
}
