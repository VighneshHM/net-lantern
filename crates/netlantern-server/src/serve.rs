//! Binding the listener and running the server on it.

use std::future::Future;
use std::net::Ipv4Addr;
use std::ops::RangeInclusive;

use tokio::net::TcpListener;

use crate::routes::router;
use crate::state::AppState;

/// The ports Net Lantern tries by default, in order.
pub const DEFAULT_PORTS: RangeInclusive<u16> = 47800..=47809;

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(
        "Ports {first}–{last} are all in use. \
         Close the program using them or start Net Lantern with --port <number>."
    )]
    NoFreePort { first: u16, last: u16 },
}

/// Binds the first port in `ports` that is free, on `127.0.0.1` only, and
/// returns the listener with its port.
///
/// Any bind error moves on to the next port, not just "address in use":
/// Windows also refuses ports inside the ranges Hyper-V reserves, with a
/// permission error.
pub async fn bind(ports: RangeInclusive<u16>) -> Result<(TcpListener, u16), ServeError> {
    let (first, last) = (*ports.start(), *ports.end());
    for port in ports {
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await {
            return Ok((listener, port));
        }
    }
    Err(ServeError::NoFreePort { first, last })
}

/// Serves the app on `listener` until `shutdown` completes, then lets
/// in-flight requests finish.
pub async fn run(
    listener: TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}
