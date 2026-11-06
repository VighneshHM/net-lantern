//! One-time launch tokens and the browser sessions they are exchanged for.
//!
//! Both are 32 random bytes from the OS RNG, hex-encoded (64 lowercase chars).
//! Neither value is ever logged, formatted, or put in an error: `Debug` on
//! [`Auth`] prints only counts.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rand::rngs::OsRng;
use rand::RngCore;

/// How much unredeemed state [`Auth`] keeps. Past a bound the oldest entry is
/// dropped, so neither collection can grow without limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthLimits {
    /// How long a launch token can be redeemed after it is issued.
    pub token_ttl: Duration,
    pub max_tokens: usize,
    pub max_sessions: usize,
}

impl Default for AuthLimits {
    fn default() -> Self {
        Self {
            token_ttl: Duration::from_secs(5 * 60),
            max_tokens: 8,
            max_sessions: 32,
        }
    }
}

/// Both collections are oldest-first.
pub struct Auth {
    limits: AuthLimits,
    tokens: Mutex<VecDeque<(String, Instant)>>,
    sessions: Mutex<VecDeque<String>>,
}

impl Default for Auth {
    fn default() -> Self {
        Self::new()
    }
}

impl Auth {
    pub fn new() -> Self {
        Self::with_limits(AuthLimits::default())
    }

    pub fn with_limits(limits: AuthLimits) -> Self {
        Self {
            limits,
            tokens: Mutex::default(),
            sessions: Mutex::default(),
        }
    }

    /// Mints a launch token. It can be redeemed once, for a session id.
    pub fn issue_launch_token(&self) -> String {
        let token = random_hex();
        let mut tokens = lock(&self.tokens);
        tokens.push_back((token.clone(), Instant::now()));
        while tokens.len() > self.limits.max_tokens {
            tokens.pop_front();
        }
        token
    }

    /// Exchanges a launch token for a new session id. The token is consumed
    /// whether or not anything else happens, so of any number of concurrent
    /// callers with the same token at most one gets `Some`. An expired token
    /// is consumed too, but redeems nothing.
    pub fn redeem(&self, token: &str) -> Option<String> {
        // Removal under the lock is the atomic one-time check.
        let issued = {
            let mut tokens = lock(&self.tokens);
            let at = tokens.iter().position(|(t, _)| t == token)?;
            tokens.remove(at)?.1
        };
        if issued.elapsed() > self.limits.token_ttl {
            return None;
        }
        let session = random_hex();
        let mut sessions = lock(&self.sessions);
        sessions.push_back(session.clone());
        while sessions.len() > self.limits.max_sessions {
            sessions.pop_front();
        }
        Some(session)
    }

    pub fn is_session(&self, id: &str) -> bool {
        lock(&self.sessions).iter().any(|s| s == id)
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
