//! NetBIOS node-status (NBSTAT) over UDP 137.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::io_error;
use crate::netio::NetbiosInfo;
use crate::PlatformError;

/// An NBSTAT query for the wildcard name `*`.
pub fn nbstat_request(txid: u16) -> [u8; 50] {
    let mut p = [0u8; 50];
    p[0..2].copy_from_slice(&txid.to_be_bytes());
    p[4..6].copy_from_slice(&1u16.to_be_bytes()); // qdcount
    p[12] = 0x20;
    p[13..45].copy_from_slice(b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    // p[45] = 0 terminates the name
    p[46..48].copy_from_slice(&0x0021u16.to_be_bytes()); // NBSTAT
    p[48..50].copy_from_slice(&1u16.to_be_bytes()); // IN
    p
}

/// Parse a node-status reply. Wrong txid, truncation or no unique `<00>`
/// name gives `None`.
pub fn parse_nbstat(buf: &[u8], txid: u16) -> Option<NetbiosInfo> {
    if buf.get(0..2)? != txid.to_be_bytes() {
        return None;
    }
    let mut pos = 12usize;
    // Owner name: a plain 34-byte encoded name or a compression pointer.
    match *buf.get(pos)? {
        0x20 => pos += 34,
        b if b & 0xC0 == 0xC0 => pos += 2,
        _ => return None,
    }
    // type, class, ttl, rdlength
    pos = pos.checked_add(10)?;
    let count = usize::from(*buf.get(pos)?);
    pos += 1;

    let mut name = None;
    let mut workgroup = None;
    for i in 0..count {
        let start = pos.checked_add(i.checked_mul(18)?)?;
        let entry = buf.get(start..start.checked_add(18)?)?;
        let suffix = entry[15];
        let flags = u16::from_be_bytes([entry[16], entry[17]]);
        if suffix != 0x00 {
            continue;
        }
        let text = clean_name(&entry[..15]);
        if flags & 0x8000 != 0 {
            workgroup.get_or_insert(text);
        } else {
            name.get_or_insert(text);
        }
    }
    let name = name.filter(|n| !n.is_empty())?;
    Some(NetbiosInfo {
        name,
        workgroup: workgroup.filter(|w| !w.is_empty()),
    })
}

fn clean_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches([' ', '\0'])
        .to_string()
}

pub(crate) async fn netbios_status(
    ip: Ipv4Addr,
    port: u16,
    timeout: Duration,
) -> Result<Option<NetbiosInfo>, PlatformError> {
    let target = SocketAddr::from((ip, port));
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(io_error)?;
    let txid: u16 = rand::random();
    socket
        .send_to(&nbstat_request(txid), target)
        .await
        .map_err(io_error)?;
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1500];
    loop {
        let received = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await;
        let (n, from) = match received {
            Err(_) => return Ok(None),
            // Windows reports an ICMP port-unreachable as a reset on the socket.
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionReset => return Ok(None),
            Ok(r) => r.map_err(io_error)?,
        };
        if from != target {
            continue;
        }
        if let Some(info) = parse_nbstat(&buf[..n], txid) {
            return Ok(Some(info));
        }
    }
}

/// Build a node-status reply in the wire layout (test helper, also used by
/// the integration tests' fake responder through a copy).
#[cfg(test)]
pub(crate) fn nbstat_reply(txid: u16, names: &[(&str, u8, u16)], mac: [u8; 6]) -> Vec<u8> {
    let mut stats = vec![names.len() as u8];
    for (name, suffix, flags) in names {
        let mut n = [b' '; 15];
        n[..name.len()].copy_from_slice(name.as_bytes());
        stats.extend_from_slice(&n);
        stats.push(*suffix);
        stats.extend_from_slice(&flags.to_be_bytes());
    }
    stats.extend_from_slice(&mac);
    stats.extend_from_slice(&[0u8; 40]);

    let mut p = Vec::new();
    p.extend_from_slice(&txid.to_be_bytes());
    p.extend_from_slice(&0x8400u16.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]); // qd 0, an 1
    p.push(0x20);
    p.extend_from_slice(b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    p.push(0);
    p.extend_from_slice(&[0, 0x21, 0, 1, 0, 0, 0, 0]);
    p.extend_from_slice(&(stats.len() as u16).to_be_bytes());
    p.extend_from_slice(&stats);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nbstat_request_layout() {
        let p = nbstat_request(0xBEEF);
        assert_eq!(&p[0..2], &[0xBE, 0xEF]);
        assert_eq!(&p[4..6], &[0, 1]);
        assert_eq!(p[12], 0x20);
        assert_eq!(&p[13..15], b"CK");
        assert_eq!(p[45], 0);
        assert_eq!(&p[46..50], &[0, 0x21, 0, 1]);
    }

    #[test]
    fn parses_nbstat_names() {
        let buf = nbstat_reply(
            0xBEEF,
            &[
                ("DISKSTATION", 0x00, 0x0400),
                ("WORKGROUP", 0x00, 0x8400),
                ("DISKSTATION", 0x20, 0x0400),
            ],
            [0x00, 0x11, 0x32, 0xAB, 0xCD, 0x70],
        );
        assert_eq!(
            parse_nbstat(&buf, 0xBEEF),
            Some(NetbiosInfo {
                name: "DISKSTATION".into(),
                workgroup: Some("WORKGROUP".into())
            })
        );
        assert_eq!(parse_nbstat(&buf, 0x0001), None);
        assert_eq!(parse_nbstat(&buf[..20], 0xBEEF), None);
    }

    #[test]
    fn every_truncation_is_safe_and_no_unique_name_is_none() {
        let buf = nbstat_reply(7, &[("A", 0x00, 0x0400)], [0; 6]);
        for len in 0..buf.len() {
            let _ = parse_nbstat(&buf[..len], 7);
        }
        let groups_only = nbstat_reply(7, &[("WG", 0x00, 0x8400), ("X", 0x20, 0x0400)], [0; 6]);
        assert_eq!(parse_nbstat(&groups_only, 7), None);
    }

    #[test]
    fn name_count_larger_than_data_is_dropped() {
        let mut buf = nbstat_reply(7, &[("A", 0x00, 0x0400)], [0; 6]);
        buf[56] = 200; // num_names
        assert_eq!(parse_nbstat(&buf, 7), None);
    }
}
