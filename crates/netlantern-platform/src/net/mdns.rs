//! One-shot ("legacy unicast", RFC 6762 section 6.7) mDNS browsing.
//!
//! Queries go to 224.0.0.251:5353 from an ephemeral port, so responders answer
//! by unicast to that port and Net Lantern never binds 5353.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RData, RecordType};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use super::{io_error, multicast_socket};
use crate::netio::MdnsService;
use crate::PlatformError;

const MDNS_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);
const META_QUERY: &str = "_services._dns-sd._udp.local.";
const QUESTIONS_PER_MESSAGE: usize = 10;
const RESEND_AFTER: Duration = Duration::from_millis(700);

/// Lower-cased dotted form of a name, without the trailing dot.
fn key(name: &Name) -> String {
    labels(name).join(".").to_ascii_lowercase()
}

/// The labels of a name as text. Wire labels are raw bytes, so a name sent as
/// `Officeprinter` already arrives as `Office printer`.
fn labels(name: &Name) -> Vec<String> {
    name.iter()
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

fn txt_pairs(txt: &hickory_proto::rr::rdata::TXT) -> Vec<(String, String)> {
    txt.txt_data
        .iter()
        .filter_map(|s| {
            let s = String::from_utf8_lossy(s);
            let (k, v) = s.split_once('=')?;
            Some((k.to_ascii_lowercase(), v.to_string()))
        })
        .collect()
}

/// Combine records from every received message into service records.
///
/// A service is grouped per (source address, instance name). SRV, TXT and A
/// records are matched by name across all messages; without an A record the
/// source address is the device address. Output is sorted by ip, service type,
/// then instance. The `_services._dns-sd._udp` meta entries are not output.
pub fn services_from_messages(msgs: &[(Ipv4Addr, Message)]) -> Vec<MdnsService> {
    let records = || {
        msgs.iter().flat_map(|(src, m)| {
            m.answers
                .iter()
                .chain(&m.authorities)
                .chain(&m.additionals)
                .map(move |r| (*src, r))
        })
    };

    let mut addr: HashMap<String, Ipv4Addr> = HashMap::new();
    let mut srv: HashMap<String, String> = HashMap::new();
    let mut txt: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for (_, r) in records() {
        match &r.data {
            RData::A(a) => {
                addr.entry(key(&r.name)).or_insert(a.0);
            }
            RData::SRV(s) => {
                let host = labels(&s.target).join(".");
                srv.entry(key(&r.name)).or_insert(host);
            }
            RData::TXT(t) => {
                txt.entry(key(&r.name)).or_default().extend(txt_pairs(t));
            }
            _ => {}
        }
    }

    let mut found: BTreeMap<(Ipv4Addr, String, String), MdnsService> = BTreeMap::new();
    for (src, r) in records() {
        let RData::PTR(ptr) = &r.data else { continue };
        let type_labels = labels(&r.name);
        let service_type = match type_labels.split_last() {
            Some((last, rest)) if last.eq_ignore_ascii_case("local") => rest.join("."),
            _ => type_labels.join("."),
        };
        if service_type.eq_ignore_ascii_case("_services._dns-sd._udp") {
            continue;
        }
        let inst_labels = labels(&ptr.0);
        let Some(instance) = inst_labels.first().filter(|l| !l.is_empty()).cloned() else {
            continue;
        };
        let full = key(&ptr.0);
        let hostname = srv.get(&full).cloned();
        let ip = hostname
            .as_ref()
            .and_then(|h| addr.get(&h.to_ascii_lowercase()))
            .copied()
            .unwrap_or(src);
        let pairs = txt.get(&full).map(Vec::as_slice).unwrap_or_default();
        let find = |k: &str| pairs.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone());
        let model = find("md").or_else(|| find("model")).or_else(|| find("ty"));
        let svc = MdnsService {
            ip,
            service_type: service_type.clone(),
            instance: instance.clone(),
            hostname,
            friendly_name: find("fn"),
            model,
        };
        found.entry((ip, service_type, instance)).or_insert(svc);
    }
    found.into_values().collect()
}

fn query_message(names: &[Name]) -> Result<Vec<u8>, PlatformError> {
    let mut msg = Message::new(rand::random(), MessageType::Query, OpCode::Query);
    for n in names {
        msg.add_query(Query::query(n.clone(), RecordType::PTR));
    }
    msg.to_vec().map_err(io_error)
}

/// Service types named by meta-query answers.
fn service_types(msg: &Message) -> Vec<Name> {
    msg.answers
        .iter()
        .filter(|r| key(&r.name) == "_services._dns-sd._udp.local")
        .filter_map(|r| match &r.data {
            RData::PTR(p) => Some(p.0.clone()),
            _ => None,
        })
        .collect()
}

async fn send(socket: &UdpSocket, dest: SocketAddr, bytes: &[u8]) -> Result<(), PlatformError> {
    socket.send_to(bytes, dest).await.map_err(io_error)?;
    Ok(())
}

pub(crate) async fn mdns_browse(
    local: Ipv4Addr,
    window: Duration,
) -> Result<Vec<MdnsService>, PlatformError> {
    browse(local, window, MDNS_GROUP.into()).await
}

async fn browse(
    local: Ipv4Addr,
    window: Duration,
    dest: SocketAddr,
) -> Result<Vec<MdnsService>, PlatformError> {
    let socket = multicast_socket(local, 255)?;
    let start = Instant::now();
    let deadline = start + window;
    let meta = query_message(&[Name::from_ascii(META_QUERY).map_err(io_error)?])?;
    send(&socket, dest, &meta).await?;
    let mut resend_at = Some(start + RESEND_AFTER).filter(|t| *t < deadline);

    let mut queried: HashSet<String> = HashSet::new();
    let mut msgs: Vec<(Ipv4Addr, Message)> = Vec::new();
    let mut buf = [0u8; 9000];
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => break,
            _ = async {
                match resend_at {
                    Some(t) => tokio::time::sleep_until(t).await,
                    None => std::future::pending().await,
                }
            } => {
                resend_at = None;
                send(&socket, dest, &meta).await?;
            }
            received = socket.recv_from(&mut buf) => {
                let Ok((n, SocketAddr::V4(from))) = received else { continue };
                let Ok(msg) = Message::from_vec(&buf[..n]) else { continue };
                if msg.metadata.message_type != MessageType::Response {
                    continue;
                }
                let fresh: Vec<Name> = service_types(&msg)
                    .into_iter()
                    .filter(|t| queried.insert(key(t)))
                    .collect();
                for batch in fresh.chunks(QUESTIONS_PER_MESSAGE) {
                    send(&socket, dest, &query_message(batch)?).await?;
                }
                msgs.push((*from.ip(), msg));
            }
        }
    }
    Ok(services_from_messages(&msgs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::{A, PTR, SRV, TXT};
    use hickory_proto::rr::Record;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn name(s: &str) -> Name {
        Name::from_ascii(s).unwrap()
    }

    fn response(records: Vec<Record>) -> Message {
        let mut m = Message::new(0, MessageType::Response, OpCode::Query);
        for r in records {
            m.add_answer(r);
        }
        m
    }

    fn rec(owner: &str, data: RData) -> Record {
        Record::from_rdata(name(owner), 120, data)
    }

    #[test]
    fn mdns_records_combine_across_messages() {
        let inst = "Google-Nest-Mini-3c1e._googlecast._tcp.local.";
        let a = response(vec![
            rec("_googlecast._tcp.local.", RData::PTR(PTR(name(inst)))),
            rec(inst, RData::SRV(SRV::new(0, 0, 8009, name("3c1e.local.")))),
            rec(
                inst,
                RData::TXT(TXT::new(vec![
                    "fn=Kitchen speaker".into(),
                    "md=Google Nest Mini".into(),
                ])),
            ),
        ]);
        let b = response(vec![rec("3c1e.local.", RData::A(A(ip("192.168.1.31"))))]);
        let s = services_from_messages(&[(ip("192.168.1.31"), a), (ip("192.168.1.31"), b)]);
        assert_eq!(
            s,
            vec![MdnsService {
                ip: ip("192.168.1.31"),
                service_type: "_googlecast._tcp".into(),
                instance: "Google-Nest-Mini-3c1e".into(),
                hostname: Some("3c1e.local".into()),
                friendly_name: Some("Kitchen speaker".into()),
                model: Some("Google Nest Mini".into()),
            }]
        );
    }

    #[test]
    fn mdns_instance_labels_are_unescaped() {
        // On the wire the label holds a real space (presentation form 032).
        let instance =
            Name::from_labels([&b"Office printer"[..], b"_ipp", b"_tcp", b"local"]).unwrap();
        let m = response(vec![rec("_ipp._tcp.local.", RData::PTR(PTR(instance)))]);
        let m = Message::from_vec(&m.to_vec().unwrap()).unwrap();
        let s = services_from_messages(&[(ip("192.168.1.40"), m)]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].instance, "Office printer");
        assert_eq!(s[0].service_type, "_ipp._tcp");
        // No A or SRV: the source address stands in.
        assert_eq!(s[0].ip, ip("192.168.1.40"));
        assert_eq!(s[0].hostname, None);
    }

    #[test]
    fn meta_entries_are_skipped_and_output_is_sorted() {
        let m = response(vec![
            rec(
                "_services._dns-sd._udp.local.",
                RData::PTR(PTR(name("_ipp._tcp.local."))),
            ),
            rec(
                "_http._tcp.local.",
                RData::PTR(PTR(name("b._http._tcp.local."))),
            ),
            rec(
                "_airplay._tcp.local.",
                RData::PTR(PTR(name("z._airplay._tcp.local."))),
            ),
            rec(
                "_airplay._tcp.local.",
                RData::PTR(PTR(name("a._airplay._tcp.local."))),
            ),
        ]);
        let n = response(vec![rec(
            "_http._tcp.local.",
            RData::PTR(PTR(name("c._http._tcp.local."))),
        )]);
        let s = services_from_messages(&[(ip("10.0.0.9"), m), (ip("10.0.0.2"), n)]);
        let order: Vec<_> = s
            .iter()
            .map(|x| (x.ip.to_string(), x.service_type.clone(), x.instance.clone()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("10.0.0.2".into(), "_http._tcp".into(), "c".into()),
                ("10.0.0.9".into(), "_airplay._tcp".into(), "a".into()),
                ("10.0.0.9".into(), "_airplay._tcp".into(), "z".into()),
                ("10.0.0.9".into(), "_http._tcp".into(), "b".into()),
            ]
        );
    }

    #[test]
    fn model_falls_back_from_md_to_model_to_ty() {
        let inst = "p._ipp._tcp.local.";
        let m = response(vec![
            rec("_ipp._tcp.local.", RData::PTR(PTR(name(inst)))),
            rec(inst, RData::TXT(TXT::new(vec!["ty=Brother HL".into()]))),
        ]);
        let s = services_from_messages(&[(ip("10.0.0.5"), m)]);
        assert_eq!(s[0].model.as_deref(), Some("Brother HL"));
    }

    #[tokio::test]
    async fn browse_follows_service_types_from_a_responder() {
        let responder = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let dest = responder.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = responder.recv_from(&mut buf).await {
                let Ok(q) = Message::from_vec(&buf[..n]) else {
                    continue;
                };
                let Some(first) = q.queries.first() else {
                    continue;
                };
                let reply = if key(first.name()) == "_services._dns-sd._udp.local" {
                    response(vec![rec(
                        META_QUERY,
                        RData::PTR(PTR(name("_ipp._tcp.local."))),
                    )])
                } else {
                    response(vec![rec(
                        "_ipp._tcp.local.",
                        RData::PTR(PTR(name("Printer._ipp._tcp.local."))),
                    )])
                };
                responder
                    .send_to(&reply.to_vec().unwrap(), from)
                    .await
                    .unwrap();
                responder.send_to(b"\x01garbage", from).await.unwrap();
            }
        });
        let got = browse(Ipv4Addr::LOCALHOST, Duration::from_millis(600), dest)
            .await
            .unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].instance, "Printer");
        assert_eq!(got[0].ip, Ipv4Addr::LOCALHOST);
    }
}
