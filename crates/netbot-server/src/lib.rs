//! NetBot HTTP/WebSocket server: the security layer (launch tokens, sessions,
//! Host/Origin guard), the structured error type, the REST routes and the
//! WebSocket event stream.

mod auth;
mod error;
mod guard;
mod routes;
mod state;
mod ws;

pub use auth::Auth;
pub use error::ApiError;
pub use guard::{guard, AccessPolicy, GuardState, SESSION_COOKIE};
pub use routes::router;
pub use state::AppState;
