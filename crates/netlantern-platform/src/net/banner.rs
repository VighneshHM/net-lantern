//! Reading the greeting line a TCP service sends on connect.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::PlatformError;

const MAX_BYTES: usize = 256;

/// Connect within `timeout`, then read until the first `\n`, 256 bytes or
/// `timeout`. Text after the first newline is dropped; the rest is returned
/// untrimmed. A failed connect or an empty read is `None`.
pub(crate) async fn tcp_banner(
    ip: Ipv4Addr,
    port: u16,
    timeout: Duration,
) -> Result<Option<String>, PlatformError> {
    let deadline = Instant::now() + timeout;
    let Ok(Ok(mut stream)) =
        tokio::time::timeout_at(deadline, TcpStream::connect((ip, port))).await
    else {
        return Ok(None);
    };
    let mut buf = Vec::with_capacity(MAX_BYTES);
    let mut chunk = [0u8; MAX_BYTES];
    while buf.len() < MAX_BYTES && !buf.contains(&b'\n') {
        let room = MAX_BYTES - buf.len();
        match tokio::time::timeout_at(deadline, stream.read(&mut chunk[..room])).await {
            Ok(Ok(n)) if n > 0 => buf.extend_from_slice(&chunk[..n]),
            _ => break,
        }
    }
    if let Some(end) = buf.iter().position(|b| *b == b'\n') {
        buf.truncate(end + 1);
    }
    if buf.is_empty() {
        return Ok(None);
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}
