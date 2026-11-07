//! Single-instance control over a per-user Windows named pipe.
//!
//! The first Net Lantern to start owns the pipe and serves it. A later launch finds
//! the pipe taken, sends `open`, and exits; the owner then opens its dashboard
//! with a fresh launch token instead of a second server starting.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use tokio::time::{sleep, timeout, Instant};

/// `ERROR_ACCESS_DENIED`: creating a pipe with `first_pipe_instance(true)`
/// fails like this when the name already exists.
const ERROR_ACCESS_DENIED: i32 = 5;
/// `ERROR_PIPE_BUSY`: every server instance is serving a client right now.
const ERROR_PIPE_BUSY: i32 = 231;

/// The only request there is.
const OPEN_REQUEST: &[u8] = b"open";
/// A request is at most this many bytes; anything longer is not `open`.
const MAX_REQUEST: u64 = 64;
/// How long the owner waits for a connected client to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a client keeps retrying while the pipe is busy.
const BUSY_RETRY_FOR: Duration = Duration::from_secs(2);
const BUSY_RETRY_EVERY: Duration = Duration::from_millis(25);
/// Pause before retrying a failed attempt to create a server instance.
const RECREATE_EVERY: Duration = Duration::from_millis(250);

/// Whether this process owns the pipe.
pub enum Instance {
    /// First to start. Hand the server to [`serve_open_requests`].
    Primary(NamedPipeServer),
    /// Another Net Lantern already owns the pipe.
    Secondary,
}

/// The pipe name for the current user: `\\.\pipe\netlantern-<USERNAME>`.
///
/// `NETLANTERN_PIPE_NAME` overrides it. That is for tests, which must not
/// depend on a Net Lantern running on the same machine.
pub fn pipe_name() -> String {
    if let Some(name) = std::env::var("NETLANTERN_PIPE_NAME")
        .ok()
        .filter(|n| !n.is_empty())
    {
        return name;
    }
    pipe_name_for(std::env::var("USERNAME").ok().as_deref())
}

fn pipe_name_for(user: Option<&str>) -> String {
    let user = user.filter(|u| !u.is_empty()).unwrap_or("user");
    format!(r"\\.\pipe\netlantern-{user}")
}

/// Tries to become the primary instance by creating the first instance of
/// `pipe`. Must be called inside a Tokio runtime.
pub fn acquire(pipe: &str) -> io::Result<Instance> {
    match ServerOptions::new().first_pipe_instance(true).create(pipe) {
        Ok(server) => Ok(Instance::Primary(server)),
        Err(e) => match e.raw_os_error() {
            Some(ERROR_ACCESS_DENIED | ERROR_PIPE_BUSY) => Ok(Instance::Secondary),
            _ => Err(e),
        },
    }
}

/// Asks the primary instance to open its dashboard.
pub async fn notify_open(pipe: &str) -> io::Result<()> {
    let give_up = Instant::now() + BUSY_RETRY_FOR;
    let mut client = loop {
        match ClientOptions::new().open(pipe) {
            Ok(client) => break client,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && Instant::now() < give_up => {
                sleep(BUSY_RETRY_EVERY).await;
            }
            Err(e) => return Err(e),
        }
    };
    client.write_all(b"open\n").await
}

/// Serves `open` requests on `pipe` forever, calling `on_open` for each.
///
/// Nothing a client sends can end the loop: a bad, slow or vanishing client
/// is dropped and the next one is served.
pub async fn serve_open_requests(
    first: NamedPipeServer,
    pipe: String,
    on_open: impl Fn() + Send + 'static,
) {
    let mut server = first;
    loop {
        if let Err(e) = server.connect().await {
            tracing::warn!("a pipe client failed to connect: {e}");
            server = new_instance(&pipe).await;
            continue;
        }
        // Start listening again before handling this client, so the next
        // launch never finds the pipe without a free instance.
        let next = new_instance(&pipe).await;
        let mut client = std::mem::replace(&mut server, next);

        match timeout(REQUEST_TIMEOUT, read_request(&mut client)).await {
            Ok(Ok(request)) if request == OPEN_REQUEST => on_open(),
            Ok(Ok(_)) => tracing::debug!("ignored a pipe request that was not `open`"),
            Ok(Err(e)) => tracing::debug!("could not read a pipe request: {e}"),
            Err(_) => tracing::debug!("a pipe client sent no request in time"),
        }
    }
}

/// Creates another server instance of `pipe`, retrying until it works.
async fn new_instance(pipe: &str) -> NamedPipeServer {
    loop {
        match ServerOptions::new().create(pipe) {
            Ok(server) => return server,
            Err(e) => {
                tracing::warn!("could not create a pipe instance: {e}");
                sleep(RECREATE_EVERY).await;
            }
        }
    }
}

/// Reads one request line (without its newline), at most [`MAX_REQUEST`] bytes.
async fn read_request(client: &mut NamedPipeServer) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    BufReader::new(client.take(MAX_REQUEST))
        .read_until(b'\n', &mut line)
        .await?;
    if line.last() == Some(&b'\n') {
        line.pop();
    }
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::pipe_name_for;

    #[test]
    fn pipe_name_is_per_user() {
        assert_eq!(pipe_name_for(Some("vighn")), r"\\.\pipe\netlantern-vighn");
    }

    #[test]
    fn pipe_name_falls_back_without_a_user() {
        assert_eq!(pipe_name_for(None), r"\\.\pipe\netlantern-user");
        assert_eq!(pipe_name_for(Some("")), r"\\.\pipe\netlantern-user");
    }
}
