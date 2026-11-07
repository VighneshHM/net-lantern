//! Net Lantern HTTP/WebSocket server: the security layer (launch tokens, sessions,
//! Host/Origin guard), the structured error type, the REST routes, the
//! WebSocket event stream, the embedded UI, and binding a port to serve on.

mod assets;
mod auth;
mod error;
mod guard;
mod refusals;
mod routes;
mod serve;
mod state;
mod ws;

pub use auth::{Auth, AuthLimits};
pub use error::ApiError;
pub use guard::{guard, AccessPolicy, GuardState, SESSION_COOKIE};
pub use refusals::RefusalLog;
pub use routes::router;
pub use serve::{bind, run, ServeError, DEFAULT_PORTS};
pub use state::AppState;
