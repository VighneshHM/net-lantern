//! The phone listener: `axum::serve` with a graceful shutdown, on a
//! listener whose connections can all be cut once the grace is over.

use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// How long open connections get to finish when the listener closes: the
/// same grace as quitting (spec §6.12).
const GRACE: Duration = Duration::from_secs(2);

/// A serving phone listener.
pub(super) struct Running {
    /// The interface address it was bound for (the URL's address can differ
    /// with [`LoopbackLanBinder`]).
    pub(super) lan_ip: Ipv4Addr,
    pub(super) addr: SocketAddr,
    shutdown: oneshot::Sender<()>,
    /// `true` cuts every connection the listener accepted.
    kill: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Running {
    pub(super) fn spawn(
        listener: TcpListener,
        addr: SocketAddr,
        lan_ip: Ipv4Addr,
        router: Router,
    ) -> Self {
        let (shutdown, stop) = oneshot::channel::<()>();
        let (kill, killed) = watch::channel(false);
        let listener = PhoneListener {
            inner: listener,
            killed,
        };
        let task = tokio::spawn(async move {
            let serving = axum::serve(listener, router).with_graceful_shutdown(async {
                let _ = stop.await;
            });
            if let Err(error) = serving.await {
                tracing::warn!(%error, "phone listener stopped");
            }
        });
        Self {
            lan_ip,
            addr,
            shutdown,
            kill,
            task,
        }
    }

    /// Stops accepting, gives open requests the grace, then cuts whatever is
    /// left (a WebSocket is not waited for by a graceful shutdown) and drops
    /// the listener.
    pub(super) async fn close(self) {
        let Self {
            shutdown,
            kill,
            mut task,
            ..
        } = self;
        let _ = shutdown.send(());
        let finished = timeout(GRACE, &mut task).await.is_ok();
        kill.send_replace(true);
        if !finished {
            task.abort();
            let _ = task.await;
        }
    }
}

/// The phone `TcpListener`, whose connections can all be cut at once.
struct PhoneListener {
    inner: TcpListener,
    killed: watch::Receiver<bool>,
}

impl axum::serve::Listener for PhoneListener {
    type Io = Cuttable;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let (io, addr) = axum::serve::Listener::accept(&mut self.inner).await;
        let mut killed = self.killed.clone();
        let cut = Box::pin(async move {
            // Also resolves if the sender is gone: no listener, no connections.
            let _ = killed.wait_for(|cut| *cut).await;
        });
        (
            Cuttable {
                io,
                cut,
                dead: false,
            },
            addr,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

/// A connection that fails every read and write once it is cut.
struct Cuttable {
    io: TcpStream,
    cut: Pin<Box<dyn Future<Output = ()> + Send>>,
    dead: bool,
}

impl Cuttable {
    fn is_cut(&mut self, cx: &mut Context<'_>) -> bool {
        if !self.dead && self.cut.as_mut().poll(cx).is_ready() {
            self.dead = true;
        }
        self.dead
    }
}

fn cut_error() -> io::Error {
    io::Error::new(io::ErrorKind::ConnectionAborted, "phone access closed")
}

impl AsyncRead for Cuttable {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.is_cut(cx) {
            return Poll::Ready(Err(cut_error()));
        }
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl AsyncWrite for Cuttable {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.is_cut(cx) {
            return Poll::Ready(Err(cut_error()));
        }
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.is_cut(cx) {
            return Poll::Ready(Err(cut_error()));
        }
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn a_cut_connection_fails_reads_and_writes() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (kill, killed) = watch::channel(false);
        let mut phone = PhoneListener {
            inner: listener,
            killed,
        };
        let mut client = TcpStream::connect(addr).await.unwrap();
        let (mut conn, _) = axum::serve::Listener::accept(&mut phone).await;

        client.write_all(b"hi").await.unwrap();
        let mut buf = [0u8; 2];
        conn.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hi");

        // A read pending when the cut comes is woken and fails.
        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 1];
            let result = conn.read(&mut buf).await;
            (conn, result)
        });
        tokio::task::yield_now().await;
        kill.send_replace(true);
        let (mut conn, result) = reader.await.unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
        assert_eq!(
            conn.write_all(b"x").await.unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
    }
}
