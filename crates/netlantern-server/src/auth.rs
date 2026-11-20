//! Launch tokens, the phone token, and the browser sessions they are
//! exchanged for.
//!
//! - A **launch token** is one-time and short-lived; it gives a
//!   [`SessionKind::Local`] session.
//! - The **phone token** (at most one at a time) is reusable until rotated or
//!   cleared; each redeem gives a new [`SessionKind::Phone`] session.
//!
//! All are 32 random bytes from the OS RNG, hex-encoded (64 lowercase chars).
//! None is ever logged, formatted, or put in an error: `Debug` on [`Auth`]
//! prints only counts and whether a phone token exists. Presented tokens are
//! compared in constant time (`subtle`).

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rand::rngs::OsRng;
use rand::RngCore;
use subtle::ConstantTimeEq;
use tokio::sync::watch;

/// Which kind of token a session was made from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKind {
    /// From a launch token: the browser on this PC.
    Local,
    /// From the phone token.
    Phone,
}

/// How much unredeemed state [`Auth`] keeps. Past a bound the oldest entry is
/// dropped, so no collection can grow without limit. Local and phone sessions
/// are capped separately, so phone sign-ins can never push out a local one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthLimits {
    /// How long a launch token can be redeemed after it is issued.
    pub token_ttl: Duration,
    pub max_tokens: usize,
    /// Cap on local sessions.
    pub max_sessions: usize,
    pub max_phone_sessions: usize,
}

impl Default for AuthLimits {
    fn default() -> Self {
        Self {
            token_ttl: Duration::from_secs(5 * 60),
            max_tokens: 8,
            max_sessions: 32,
            max_phone_sessions: 32,
        }
    }
}

/// Every collection is oldest-first.
pub struct Auth {
    limits: AuthLimits,
    tokens: Mutex<VecDeque<(String, Instant)>>,
    phone_token: Mutex<Option<String>>,
    sessions: Mutex<VecDeque<String>>,
    phone_sessions: Mutex<VecDeque<String>>,
    /// The revocation generation; bumped whenever sessions are revoked.
    revoked: watch::Sender<u64>,
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
            phone_token: Mutex::default(),
            sessions: Mutex::default(),
            phone_sessions: Mutex::default(),
            revoked: watch::channel(0).0,
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

    /// Exchanges a token for a new session id and its kind.
    ///
    /// A launch token (one-time, TTL) gives a Local session. It is consumed
    /// whether or not anything else happens, so of any number of concurrent
    /// callers with the same launch token at most one gets `Some`. An expired
    /// launch token is consumed too, but redeems nothing.
    ///
    /// The current phone token (reusable) gives a Phone session.
    pub fn redeem(&self, token: &str) -> Option<(String, SessionKind)> {
        if let Some(issued) = self.take_launch_token(token) {
            if issued.elapsed() > self.limits.token_ttl {
                return None;
            }
            let session = random_hex();
            push_capped(&self.sessions, session.clone(), self.limits.max_sessions);
            return Some((session, SessionKind::Local));
        }
        if self.is_phone_token(token) {
            let session = random_hex();
            push_capped(
                &self.phone_sessions,
                session.clone(),
                self.limits.max_phone_sessions,
            );
            return Some((session, SessionKind::Phone));
        }
        None
    }

    /// Removes `token` from the pending launch tokens and returns when it
    /// was issued. Removal under the lock is the atomic one-time check. Every
    /// pending token is compared, in constant time.
    fn take_launch_token(&self, token: &str) -> Option<Instant> {
        let mut tokens = lock(&self.tokens);
        let mut found = None;
        for (at, (t, _)) in tokens.iter().enumerate() {
            if bool::from(t.as_bytes().ct_eq(token.as_bytes())) {
                found = Some(at);
            }
        }
        Some(tokens.remove(found?)?.1)
    }

    fn is_phone_token(&self, token: &str) -> bool {
        lock(&self.phone_token)
            .as_ref()
            .is_some_and(|t| bool::from(t.as_bytes().ct_eq(token.as_bytes())))
    }

    /// New phone token, replacing any old one. Never logged.
    pub fn rotate_phone_token(&self) -> String {
        let token = random_hex();
        *lock(&self.phone_token) = Some(token.clone());
        token
    }

    /// No phone token: nothing redeems as Phone.
    pub fn clear_phone_token(&self) {
        *lock(&self.phone_token) = None;
    }

    pub fn phone_token(&self) -> Option<String> {
        lock(&self.phone_token).clone()
    }

    /// Drops every phone session and bumps the revocation generation, so
    /// open event streams of those sessions close. Local sessions stay.
    pub fn end_phone_sessions(&self) {
        lock(&self.phone_sessions).clear();
        self.revoked.send_modify(|generation| *generation += 1);
    }

    pub fn session_kind(&self, id: &str) -> Option<SessionKind> {
        if lock(&self.sessions).iter().any(|s| s == id) {
            Some(SessionKind::Local)
        } else if lock(&self.phone_sessions).iter().any(|s| s == id) {
            Some(SessionKind::Phone)
        } else {
            None
        }
    }

    /// Whether `id` is a live session of either kind.
    pub fn is_session(&self, id: &str) -> bool {
        self.session_kind(id).is_some()
    }

    /// Fires (value changes) whenever sessions are revoked. A holder then
    /// re-checks its own session with [`Auth::is_session`].
    pub fn revocations(&self) -> watch::Receiver<u64> {
        self.revoked.subscribe()
    }
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("pending_tokens", &lock(&self.tokens).len())
            .field("sessions", &lock(&self.sessions).len())
            .field("phone_sessions", &lock(&self.phone_sessions).len())
            .field("phone_token", &lock(&self.phone_token).is_some())
            .finish()
    }
}

/// Appends `session`, dropping the oldest past `cap`.
fn push_capped(sessions: &Mutex<VecDeque<String>>, session: String, cap: usize) {
    let mut sessions = lock(sessions);
    sessions.push_back(session);
    while sessions.len() > cap {
        sessions.pop_front();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn local(auth: &Auth) -> String {
        let (id, kind) = auth.redeem(&auth.issue_launch_token()).unwrap();
        assert_eq!(kind, SessionKind::Local);
        id
    }

    #[test]
    fn phone_token_redeems_many_times_as_phone() {
        let auth = Auth::new();
        let token = auth.rotate_phone_token();
        assert_eq!(auth.phone_token().as_deref(), Some(token.as_str()));
        let (a, kind_a) = auth.redeem(&token).unwrap();
        let (b, kind_b) = auth.redeem(&token).unwrap();
        assert_eq!((kind_a, kind_b), (SessionKind::Phone, SessionKind::Phone));
        assert_ne!(a, b, "each redeem is its own session");
        assert_ne!(a, token, "a session id is not the phone token");
        assert_eq!(auth.session_kind(&a), Some(SessionKind::Phone));
        assert_eq!(auth.session_kind(&b), Some(SessionKind::Phone));
        assert!(auth.is_session(&a) && auth.is_session(&b));
        // A session id is not a phone token either.
        assert!(auth.redeem(&a).is_none());
        assert!(auth.redeem("").is_none());
    }

    #[test]
    fn launch_token_still_redeems_once_as_local() {
        let auth = Auth::new();
        auth.rotate_phone_token();
        let token = auth.issue_launch_token();
        let (id, kind) = auth.redeem(&token).unwrap();
        assert_eq!(kind, SessionKind::Local);
        assert_eq!(auth.session_kind(&id), Some(SessionKind::Local));
        assert!(auth.is_session(&id));
        assert!(auth.redeem(&token).is_none(), "launch tokens stay one-time");
    }

    #[test]
    fn launch_tokens_still_expire() {
        let auth = Auth::with_limits(AuthLimits {
            token_ttl: Duration::ZERO,
            ..AuthLimits::default()
        });
        let token = auth.issue_launch_token();
        std::thread::sleep(Duration::from_millis(5));
        assert!(auth.redeem(&token).is_none());
    }

    #[test]
    fn rotated_phone_token_refuses_the_old_one() {
        let auth = Auth::new();
        let old = auth.rotate_phone_token();
        let new = auth.rotate_phone_token();
        assert_ne!(old, new);
        assert!(auth.redeem(&old).is_none());
        assert_eq!(auth.redeem(&new).unwrap().1, SessionKind::Phone);
        assert_eq!(auth.phone_token().as_deref(), Some(new.as_str()));
    }

    #[test]
    fn cleared_phone_token_redeems_nothing() {
        let auth = Auth::new();
        assert_eq!(auth.phone_token(), None, "no phone token until one is made");
        assert!(auth.redeem("").is_none());
        let token = auth.rotate_phone_token();
        auth.clear_phone_token();
        assert_eq!(auth.phone_token(), None);
        assert!(auth.redeem(&token).is_none());
        assert!(auth.redeem("").is_none());
    }

    #[test]
    fn end_phone_sessions_keeps_local_sessions() {
        let auth = Auth::new();
        let token = auth.rotate_phone_token();
        let before = local(&auth);
        let (phone, _) = auth.redeem(&token).unwrap();
        let after = local(&auth);

        auth.end_phone_sessions();
        assert!(!auth.is_session(&phone));
        assert_eq!(auth.session_kind(&phone), None);
        assert_eq!(auth.session_kind(&before), Some(SessionKind::Local));
        assert_eq!(auth.session_kind(&after), Some(SessionKind::Local));
        // The phone token itself is untouched: the phone can sign in again.
        assert_eq!(auth.redeem(&token).unwrap().1, SessionKind::Phone);
    }

    #[test]
    fn phone_sessions_never_evict_local_sessions() {
        let auth = Auth::new();
        assert_eq!(AuthLimits::default().max_phone_sessions, 32);
        let first_local = local(&auth);
        let token = auth.rotate_phone_token();
        let phones: Vec<String> = (0..40).map(|_| auth.redeem(&token).unwrap().0).collect();

        assert_eq!(auth.session_kind(&first_local), Some(SessionKind::Local));
        // Phone sessions are capped on their own, oldest first.
        assert!(!auth.is_session(&phones[7]));
        assert!(auth.is_session(&phones[8]));
        assert!(auth.is_session(&phones[39]));
    }

    #[test]
    fn revocations_receiver_sees_a_change_on_end_phone_sessions() {
        let auth = Auth::new();
        let mut rx = auth.revocations();
        assert!(!rx.has_changed().unwrap());
        auth.end_phone_sessions();
        assert!(rx.has_changed().unwrap());
        let first = *rx.borrow_and_update();
        auth.end_phone_sessions();
        assert!(rx.has_changed().unwrap());
        assert_ne!(*rx.borrow_and_update(), first);
    }

    #[test]
    fn debug_never_contains_tokens_or_session_ids() {
        let auth = Auth::new();
        let pending = auth.issue_launch_token();
        let local_id = local(&auth);
        let phone_token = auth.rotate_phone_token();
        let (phone_id, _) = auth.redeem(&phone_token).unwrap();

        let printed = format!("{auth:?} {auth:#?}");
        for secret in [&pending, &local_id, &phone_token, &phone_id] {
            assert!(
                !printed.contains(secret.as_str()),
                "secret leaked: {printed}"
            );
        }
        assert!(printed.contains("phone_token: true"), "{printed}");
        auth.clear_phone_token();
        assert!(format!("{auth:?}").contains("phone_token: false"));
    }
}
