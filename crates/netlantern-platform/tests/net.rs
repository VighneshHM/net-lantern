use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, ResponseCode};
use hickory_proto::rr::rdata::{A, PTR};
use hickory_proto::rr::{Name, RData, Record};
use netlantern_platform::{NetIo, RealNetIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};

/// A fake DNS server on loopback answering every query via `answer`.
async fn fake_dns(answer: fn(&Message) -> Message) -> u16 {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = sock.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            let Ok((n, from)) = sock.recv_from(&mut buf).await else {
                return;
            };
            let Ok(query) = Message::from_vec(&buf[..n]) else {
                continue;
            };
            let reply = answer(&query);
            sock.send_to(&reply.to_vec().unwrap(), from).await.unwrap();
        }
    });
    port
}

fn response_for(query: &Message) -> Message {
    let mut reply = Message::response(query.metadata.id, OpCode::Query);
    reply.metadata.message_type = MessageType::Response;
    reply.queries = query.queries.clone();
    reply
}

fn answer_a(query: &Message) -> Message {
    let mut reply = response_for(query);
    let name = query.queries[0].name().clone();
    reply.add_answer(Record::from_rdata(
        name,
        60,
        RData::A(A(Ipv4Addr::new(10, 0, 0, 9))),
    ));
    reply
}

fn answer_ptr(query: &Message) -> Message {
    let mut reply = response_for(query);
    let name = query.queries[0].name().clone();
    let target = Name::from_ascii("echo-dot.lan.").unwrap();
    reply.add_answer(Record::from_rdata(name, 60, RData::PTR(PTR(target))));
    reply
}

fn answer_nxdomain(query: &Message) -> Message {
    let mut reply = response_for(query);
    reply.metadata.response_code = ResponseCode::NXDomain;
    reply
}

const LOOPBACK: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);

#[tokio::test]
async fn resolves_a_records_from_the_given_server() {
    let port = fake_dns(answer_a).await;
    let io = RealNetIo::with_ports(port, 137);
    let ips = io.dns_resolve(LOOPBACK, "example.test").await.unwrap();
    assert_eq!(ips, vec![Ipv4Addr::new(10, 0, 0, 9)]);
}

#[tokio::test]
async fn nxdomain_resolves_to_nothing() {
    let port = fake_dns(answer_nxdomain).await;
    let io = RealNetIo::with_ports(port, 137);
    let ips = io.dns_resolve(LOOPBACK, "nope.test").await.unwrap();
    assert!(ips.is_empty());
}

#[tokio::test]
async fn resolve_timeout_is_an_io_error() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = silent.local_addr().unwrap().port();
    let io = RealNetIo::with_ports(port, 137);
    let err = io.dns_resolve(LOOPBACK, "example.test").await.unwrap_err();
    assert!(matches!(err, netlantern_platform::PlatformError::Io { .. }));
}

#[tokio::test]
async fn reverse_dns_returns_name_without_trailing_dot() {
    let port = fake_dns(answer_ptr).await;
    let io = RealNetIo::with_ports(port, 137);
    let name = io
        .reverse_dns(LOOPBACK, Ipv4Addr::new(192, 168, 1, 61))
        .await
        .unwrap();
    assert_eq!(name.as_deref(), Some("echo-dot.lan"));
}

#[tokio::test]
async fn reverse_dns_nxdomain_is_none() {
    let port = fake_dns(answer_nxdomain).await;
    let io = RealNetIo::with_ports(port, 137);
    let name = io
        .reverse_dns(LOOPBACK, Ipv4Addr::new(192, 168, 1, 61))
        .await
        .unwrap();
    assert_eq!(name, None);
}

#[tokio::test]
async fn reverse_dns_silence_is_none() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = silent.local_addr().unwrap().port();
    let io = RealNetIo::with_ports(port, 137);
    let started = std::time::Instant::now();
    let name = io
        .reverse_dns(LOOPBACK, Ipv4Addr::new(192, 168, 1, 61))
        .await
        .unwrap();
    assert_eq!(name, None);
    assert!(started.elapsed() < Duration::from_secs(3));
}

/// A one-connection-at-a-time HTTP server that answers every request with `response`.
async fn fake_http(response: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

const REDIRECT: &str =
    "HTTP/1.1 302 Found\r\nLocation: /x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

#[tokio::test]
async fn http_get_does_not_follow_redirects_when_asked() {
    let addr = fake_http(REDIRECT).await;
    let io = RealNetIo::new();
    let resp = io
        .http_get(&format!("http://{addr}/"), false)
        .await
        .unwrap();
    assert_eq!(resp.status, 302);
}

#[tokio::test]
async fn http_get_returns_a_body_prefix() {
    let addr =
        fake_http("HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").await;
    let io = RealNetIo::new();
    let resp = io.http_get(&format!("http://{addr}/"), true).await.unwrap();
    assert_eq!((resp.status, resp.body_prefix.as_str()), (200, "hello"));
}

/// Answers `/final` with a 200 "final" and everything else with `other`.
async fn fake_http_routes(other: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let response = if request.starts_with("GET /final") {
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nfinal"
            } else {
                other
            };
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    addr
}

#[tokio::test]
async fn location_header_is_returned_without_following() {
    let addr = fake_http(REDIRECT).await;
    let resp = RealNetIo::new()
        .http_get(&format!("http://{addr}/"), false)
        .await
        .unwrap();
    assert_eq!(resp.status, 302);
    assert_eq!(resp.location.as_deref(), Some("/x"));
}

#[tokio::test]
async fn redirects_to_another_host_are_not_followed() {
    let addr = fake_http(
        "HTTP/1.1 302 Found\r\nLocation: http://example.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    let resp = RealNetIo::new()
        .http_get(&format!("http://{addr}/"), true)
        .await
        .unwrap();
    assert_eq!(resp.status, 302);
    assert_eq!(resp.location.as_deref(), Some("http://example.invalid/"));
}

#[tokio::test]
async fn same_host_redirects_are_followed() {
    let addr = fake_http_routes(
        "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    let resp = RealNetIo::new()
        .http_get(&format!("http://{addr}/"), true)
        .await
        .unwrap();
    assert_eq!((resp.status, resp.body_prefix.as_str()), (200, "final"));
}

/// Redirects every request to `/next` on the same listener and counts requests.
#[tokio::test]
async fn same_host_redirects_stop_after_five() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = stream
                .write_all(
                    b"HTTP/1.1 302 Found
Location: /next
Content-Length: 0
Connection: close

",
                )
                .await;
            let _ = stream.shutdown().await;
        }
    });
    let resp = RealNetIo::new()
        .http_get(&format!("http://{addr}/"), true)
        .await
        .unwrap();
    assert_eq!(resp.status, 302);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 6);
}

#[tokio::test]
#[ignore = "needs the internet"]
async fn self_signed_site_is_a_certificate_error() {
    let err = RealNetIo::new()
        .http_get("https://self-signed.badssl.com/", false)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            netlantern_platform::PlatformError::TlsCertificate { .. }
        ),
        "{err:?}"
    );
}

#[tokio::test]
#[ignore = "needs the internet"]
async fn microsoft_homepage_loads() {
    let response = RealNetIo::new()
        .http_get("https://www.microsoft.com/", false)
        .await
        .unwrap();
    assert!((200..400).contains(&response.status), "{}", response.status);
}

#[tokio::test]
async fn http_get_connection_refused_is_an_io_error() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };
    let io = RealNetIo::new();
    let err = io
        .http_get(&format!("http://127.0.0.1:{port}/"), false)
        .await
        .unwrap_err();
    assert!(matches!(err, netlantern_platform::PlatformError::Io { .. }));
}

/// A fake NetBIOS node-status responder on loopback: echoes the query's
/// transaction id and answers with one unique and one group `<00>` name.
async fn fake_netbios() -> u16 {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let port = sock.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = sock.recv_from(&mut buf).await {
            if n < 50 {
                continue;
            }
            let mut stats = vec![2u8];
            for (name, flags) in [("NAS-BOX", 0x0400u16), ("HOME", 0x8400)] {
                let mut padded = [b' '; 15];
                padded[..name.len()].copy_from_slice(name.as_bytes());
                stats.extend_from_slice(&padded);
                stats.push(0x00);
                stats.extend_from_slice(&flags.to_be_bytes());
            }
            stats.extend_from_slice(&[0u8; 46]);
            let mut p = vec![buf[0], buf[1], 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0x20];
            p.extend_from_slice(b"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
            p.push(0);
            p.extend_from_slice(&[0, 0x21, 0, 1, 0, 0, 0, 0]);
            p.extend_from_slice(&(stats.len() as u16).to_be_bytes());
            p.extend_from_slice(&stats);
            sock.send_to(&p, from).await.unwrap();
        }
    });
    port
}

#[tokio::test]
async fn netbios_status_reads_the_name_from_a_responder() {
    let port = fake_netbios().await;
    let io = RealNetIo::with_ports(53, port);
    let info = io
        .netbios_status(Ipv4Addr::LOCALHOST, Duration::from_secs(2))
        .await
        .unwrap()
        .expect("a name");
    assert_eq!(info.name, "NAS-BOX");
    assert_eq!(info.workgroup.as_deref(), Some("HOME"));
}

#[tokio::test]
async fn netbios_status_on_a_closed_port_is_none_within_the_timeout() {
    // Bind then drop to find a port nothing listens on.
    let port = {
        let s = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        s.local_addr().unwrap().port()
    };
    let io = RealNetIo::with_ports(53, port);
    let started = std::time::Instant::now();
    let got = io
        .netbios_status(Ipv4Addr::LOCALHOST, Duration::from_millis(500))
        .await
        .unwrap();
    assert_eq!(got, None);
    assert!(started.elapsed() < Duration::from_millis(1500));
}

/// Manual smoke test against the real LAN: `cargo test -p netlantern-platform
/// --test net -- --ignored --nocapture`. Prints counts only.
#[cfg(windows)]
#[tokio::test]
#[ignore]
async fn smoke_real_discovery() {
    use netlantern_platform::{Platform, WindowsPlatform};
    let local = WindowsPlatform::new()
        .active_interface()
        .await
        .expect("an active interface")
        .ip;
    let io = RealNetIo::new();
    let ssdp = io.ssdp_search(local, Duration::from_secs(3)).await.unwrap();
    let mdns = io.mdns_browse(local, Duration::from_secs(3)).await.unwrap();
    println!(
        "SMOKE ssdp_replies={} mdns_services={}",
        ssdp.len(),
        mdns.len()
    );
}
