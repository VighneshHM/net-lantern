//! One-time launch tokens and the browser sessions they are exchanged for.
//!
//! Both are 32 random bytes from the OS RNG, hex-encoded (64 lowercase chars).
//! Neither value is ever logged, formatted, or put in an error: `Debug` on
//! [`Auth`] prints only counts.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rand::rngs::OsRng;
use rand::RngCore;

#[derive(Default)]
pub struct Auth {
    tokens: Mutex<HashSet<String>>,
    sessions: Mutex<HashSet<String>>,
}

impl Auth {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mints a launch token. It can be redeemed once, for a session id.
    pub fn issue_launch_token(&self) -> String {
        let token = random_hex();
        lock(&self.tokens).insert(token.clone());
        token
    }

    /// Exchanges a launch token for a new session id. The token is consumed
    /// whether or not anything else happens, so of any number of concurrent
    /// callers with the same token at most one gets `Some`.
    pub fn redeem(&self, token: &str) -> Option<String> {
        // `remove` under the lock is the atomic one-time check.
        if !lock(&self.tokens).remove(token) {
            return None;
        }
        let session = random_hex();
        lock(&self.sessions).insert(session.clone());
        Some(session)
    }

    pub fn is_session(&self, id: &str) -> bool {
        lock(&self.sessions).contains(id)
    }
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("pending_tokens", &lock(&self.tokens).len())
            .field("sessions", &lock(&self.sessions).len())
            .finish()
    }
}

fn random_hex() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// The sets stay valid whatever a panicking holder was doing, so a poisoned
/// lock is not a reason to take the server down.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
