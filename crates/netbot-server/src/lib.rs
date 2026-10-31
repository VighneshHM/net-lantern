//! NetBot HTTP/WebSocket server: the security layer (launch tokens, sessions,
//! Host/Origin guard) and the structured error type. Routes arrive in later
//! tasks.

mod auth;
mod error;
mod guard;

pub use auth::Auth;
pub use error::ApiError;
pub use guard::{guard, AccessPolicy, GuardState, SESSION_COOKIE};
