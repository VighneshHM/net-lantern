//! Phone access: a second listener, on the laptop's LAN address, that serves
//! the same router (and so the same guard) to phones on the network.
//!
//! - It is off by default. Turning it on mints a phone token
//!   ([`Auth::rotate_phone_token`]); the phone opens
//!   `http://{lan_ip}:{port}/?t={token}`, which the UI exchanges for a
//!   session as it does a launch token.
//! - It never runs on a network Windows calls Public, and a watcher follows
//!   the network every 5 s while the setting is on: a new address rebinds
//!   (same token, phone sessions end), Public or no connection pauses (the
//!   listener closes, the token is cleared, phone sessions end), and a Private
//!   network again resumes with a new token.
//! - The guard accepts the phone listener's Host and Origin only while it is
//!   On ([`PhoneAccess::allowed_host`]).
//!
//! Every state change (enable, disable, the watcher's steps, restore, stop)
//! runs under one async lock, so a turn-off can never interleave with a
//! rotate that would re-arm phone access. Turning off is always
//! `clear_phone_token` then `end_phone_sessions`, then closing the listener
//! within the quit grace (2 s); connections still open after it, upgraded
//! WebSockets included, are cut.
//!
//! The token appears only in [`PhoneStatus::url`], which only signed-in users
//! see. It is never logged and never put in an error.

mod binder;
mod listener;

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use axum::Router;
use netlantern_core::{EventBus, EventKind, Store, PHONE_ACCESS_KEY};
use netlantern_platform::{FirewallVerdict, NetworkCategory, Platform};
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::{interval_at, Instant, MissedTickBehavior};

use crate::auth::Auth;
use crate::error::ApiError;
use listener::Running;

pub use binder::{LanBinder, LoopbackLanBinder, RealLanBinder};

/// How often the watcher reads the network while the setting is on (spec
/// §6.12).
const WATCH_EVERY: Duration = Duration::from_secs(5);

const PUBLIC_NETWORK: &str = "public_network";
const NOT_CONNECTED: &str = "not_connected";
const BIND_FAILED: &str = "bind_failed";

const NEEDS_NETWORK: &str = "Phone access needs a network connection.";
const CANNOT_LISTEN: &str = "Phone access couldn't listen on this computer's network address.";

// ------------------------------------------------------------------- status

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneState {
    Off,
    On,
    Paused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PhoneStatus {
    pub state: PhoneState,
    /// Why it is paused or could not start: "public_network" | "not_connected" | "bind_failed"; None when Off/On.
    pub reason: Option<String>,
    /// Full URL with token, only when On. Shown only to signed-in users.
    pub url: Option<String>,
    /// None when Off or when it could not be read.
    pub firewall: Option<FirewallVerdict>,
    pub category: Option<NetworkCategory>,
    /// True with [`LoopbackLanBinder`]: "this address works on this laptop only".
    pub demo: bool,
}

// ------------------------------------------------------------ phone access

pub struct PhoneAccess {
    platform: Arc<dyn Platform>,
    store: Store,
    auth: Arc<Auth>,
    bus: Arc<EventBus>,
    binder: Arc<dyn LanBinder>,
    exe: PathBuf,
    main_port: u16,
    demo: bool,
    /// What the phone listener serves; `None` until attached and after stop.
    router: Mutex<Option<Router>>,
    /// Every state change holds this for its whole length.
    inner: tokio::sync::Mutex<Inner>,
    /// The phone listener's address while On, read by the guard on every
    /// request. Written only under `inner`.
    serving: RwLock<Option<SocketAddr>>,
}

struct Inner {
    mode: Mode,
    /// Runs while the setting is on.
    watcher: Option<JoinHandle<()>>,
    /// Set by [`PhoneAccess::stop`]: from then on nothing starts again (no
    /// listener, no watcher, no event), even a `restore` that arrives late.
    stopped: bool,
}

enum Mode {
    Off,
    On(Running),
    /// The setting is on, but phone access cannot run now, for this reason.
    Paused(&'static str),
}

/// What one watcher step does, decided from the mode and a network reading.
enum Step {
    Stop,
    Stay,
    Rebind(Ipv4Addr),
    Pause(&'static str),
    Resume(Ipv4Addr),
}

impl PhoneAccess {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        platform: Arc<dyn Platform>,
        store: Store,
        auth: Arc<Auth>,
        bus: Arc<EventBus>,
        binder: Arc<dyn LanBinder>,
        exe: PathBuf,
        main_port: u16,
        demo: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            platform,
            store,
            auth,
            bus,
            binder,
            exe,
            main_port,
            demo,
            router: Mutex::new(None),
            inner: tokio::sync::Mutex::new(Inner {
                mode: Mode::Off,
                watcher: None,
                stopped: false,
            }),
            serving: RwLock::new(None),
        })
    }

    /// The router the phone listener serves. AppState holds the PhoneAccess, and the router needs the
    /// AppState, so the host calls this once after building both; enable() before it is bind_failed.
    pub fn attach_router(&self, router: Router) {
        *self.router.lock().unwrap_or_else(PoisonError::into_inner) = Some(router);
    }

    /// Restore the saved setting at start (On if saved "on" and allowed).
    /// Saved "on" but not allowed now (Public, no connection, no address to
    /// listen on) is Paused, and the watcher resumes it when it can.
    pub async fn restore(self: &Arc<Self>) {
        // Read the setting only once the lock is held, so a disable that lands
        // first is never overridden by a stale "on".
        let mut inner = self.inner.lock().await;
        match self.store.global_setting(PHONE_ACCESS_KEY).await {
            Ok(Some(value)) if value == "on" => {}
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(%error, "could not read the phone access setting");
                return;
            }
        }
        if inner.stopped || !matches!(inner.mode, Mode::Off) {
            return;
        }
        match self.read_network().await {
            Ok(ip) => {
                if self.start(&mut inner, ip, true).await.is_err() {
                    inner.mode = Mode::Paused(BIND_FAILED);
                }
            }
            Err(reason) => inner.mode = Mode::Paused(reason),
        }
        self.ensure_watcher(&mut inner);
        self.changed();
    }

    /// 409 `public_network` (the setting is still saved "on"), 503
    /// `phone_access_unavailable` without a connection or an address to
    /// listen on. Already On: the current status, same token.
    ///
    /// The watcher is armed as soon as the mode changes, before any further
    /// await, so a caller dropping this future part-way (a client that
    /// disconnects) can never leave phone access On without the watcher that
    /// pauses it on a Public network. The route also runs it in its own task.
    pub async fn enable(self: &Arc<Self>) -> Result<PhoneStatus, ApiError> {
        let mut inner = self.inner.lock().await;
        if inner.stopped {
            return Err(ApiError::phone_access_unavailable(CANNOT_LISTEN));
        }
        if matches!(inner.mode, Mode::On(_)) {
            return Ok(self.status_of(&inner).await);
        }
        match self.read_network().await {
            Err(NOT_CONNECTED) => Err(ApiError::phone_access_unavailable(NEEDS_NETWORK)),
            Err(reason) => {
                inner.mode = Mode::Paused(reason);
                self.ensure_watcher(&mut inner);
                self.save("on").await;
                self.changed();
                Err(ApiError::public_network())
            }
            Ok(ip) => {
                self.start(&mut inner, ip, true).await?;
                self.ensure_watcher(&mut inner);
                self.save("on").await;
                self.changed();
                Ok(self.status_of(&inner).await)
            }
        }
    }

    /// Saves "off", ends phone access (token cleared, phone sessions ended,
    /// listener closed within the grace) and stops the watcher.
    ///
    /// Asked for from a phone, this takes the full 2 s grace: the graceful
    /// shutdown waits for in-flight requests, and the disabling request is one
    /// of them. That is unavoidable (its connection is on the listener being
    /// closed), so the phone may not see the response; the route runs this in
    /// its own task, so the turn-off and its `phone.changed` still complete.
    pub async fn disable(&self) -> PhoneStatus {
        let mut inner = self.inner.lock().await;
        self.save("off").await;
        self.turn_off(&mut inner).await;
        self.changed();
        self.status_of(&inner).await
    }

    pub async fn status(&self) -> PhoneStatus {
        let inner = self.inner.lock().await;
        self.status_of(&inner).await
    }

    /// Host and Origin values the guard must accept now (empty when not On).
    pub fn allowed_host(&self) -> Option<String> {
        self.serving_addr().map(|addr| addr.to_string())
    }

    pub fn allowed_origin(&self) -> Option<String> {
        self.serving_addr().map(|addr| format!("http://{addr}"))
    }

    /// On quit: like [`PhoneAccess::disable`] but the setting is kept, so the
    /// next start restores it. Also lets go of the router, which holds the
    /// state that holds this.
    pub async fn stop(&self) {
        let mut inner = self.inner.lock().await;
        inner.stopped = true;
        self.turn_off(&mut inner).await;
        self.router
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }

    // ------------------------------------------------------------ internals

    fn serving_addr(&self) -> Option<SocketAddr> {
        *self.serving.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_serving(&self, addr: Option<SocketAddr>) {
        *self.serving.write().unwrap_or_else(PoisonError::into_inner) = addr;
    }

    fn changed(&self) {
        self.bus.publish(EventKind::PhoneChanged {});
    }

    async fn save(&self, value: &str) {
        if let Err(error) = self.store.set_global_setting(PHONE_ACCESS_KEY, value).await {
            tracing::warn!(%error, "could not save the phone access setting");
        }
    }

    /// The address to listen on, if this network allows phone access. A
    /// category that cannot be read counts as Public: phone access only runs
    /// where Windows says the network is Private or Domain.
    async fn read_network(&self) -> Result<Ipv4Addr, &'static str> {
        let ip = self
            .platform
            .active_interface()
            .await
            .map_err(|_| NOT_CONNECTED)?
            .ip;
        match self.platform.network_category().await {
            Ok(NetworkCategory::Private | NetworkCategory::Domain) => Ok(ip),
            Ok(NetworkCategory::Public) => Err(PUBLIC_NETWORK),
            Err(netlantern_platform::PlatformError::NotConnected) => Err(NOT_CONNECTED),
            Err(error) => {
                tracing::warn!(%error, "could not read the network category; treating it as Public");
                Err(PUBLIC_NETWORK)
            }
        }
    }

    /// Binds on `ip` and serves. A new token unless `rotate` is false (a
    /// rebind keeps it). On failure nothing changes.
    async fn start(&self, inner: &mut Inner, ip: Ipv4Addr, rotate: bool) -> Result<(), ApiError> {
        let router = self
            .router
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(router) = router else {
            tracing::warn!("phone access asked for before the server was ready");
            return Err(ApiError::phone_access_unavailable(CANNOT_LISTEN));
        };
        let (listener, addr) = self
            .binder
            .bind(ip, self.main_port)
            .await
            .map_err(|error| {
                tracing::warn!(%error, "phone access could not listen");
                ApiError::phone_access_unavailable(CANNOT_LISTEN)
            })?;
        if rotate {
            self.auth.rotate_phone_token();
        }
        let running = Running::spawn(listener, addr, ip, router);
        self.set_serving(Some(addr));
        tracing::info!(%addr, "phone access listening");
        inner.mode = Mode::On(running);
        Ok(())
    }

    /// Ends phone access for now: the guard stops accepting the phone host,
    /// phone sessions end (the token is cleared first unless `keep_token`),
    /// and the listener closes. Leaves the mode Off.
    async fn shut(&self, inner: &mut Inner, keep_token: bool) {
        self.set_serving(None);
        if !keep_token {
            self.auth.clear_phone_token();
        }
        self.auth.end_phone_sessions();
        if let Mode::On(running) = std::mem::replace(&mut inner.mode, Mode::Off) {
            running.close().await;
            tracing::info!("phone access closed");
        }
    }

    async fn turn_off(&self, inner: &mut Inner) {
        if let Some(watcher) = inner.watcher.take() {
            watcher.abort();
        }
        self.shut(inner, false).await;
    }

    fn ensure_watcher(self: &Arc<Self>, inner: &mut Inner) {
        if inner.stopped {
            return;
        }
        if inner.watcher.as_ref().is_some_and(|w| !w.is_finished()) {
            return;
        }
        let this = Arc::downgrade(self);
        // The first check is due 5 s after arming, not after the task first
        // runs, which can be later.
        let first = Instant::now() + WATCH_EVERY;
        inner.watcher = Some(tokio::spawn(async move {
            let mut ticks = interval_at(first, WATCH_EVERY);
            ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                let Some(this) = this.upgrade() else { return };
                if !this.follow_network().await {
                    return;
                }
            }
        }));
    }

    /// One watcher step. False once phone access is off.
    async fn follow_network(&self) -> bool {
        let mut inner = self.inner.lock().await;
        let reading = self.read_network().await;
        let step = match (&inner.mode, reading) {
            (Mode::Off, _) => Step::Stop,
            (Mode::On(running), Ok(ip)) if running.lan_ip == ip => Step::Stay,
            (Mode::On(_), Ok(ip)) => Step::Rebind(ip),
            (Mode::On(_), Err(reason)) => Step::Pause(reason),
            (Mode::Paused(_), Ok(ip)) => Step::Resume(ip),
            (Mode::Paused(old), Err(reason)) if *old == reason => Step::Stay,
            (Mode::Paused(_), Err(reason)) => Step::Pause(reason),
        };
        match step {
            Step::Stop => return false,
            Step::Stay => return true,
            Step::Rebind(ip) => {
                // Phone cookies are bound to the old host, so their sessions
                // end; the token stays, so the same URL works on the new one.
                self.shut(&mut inner, true).await;
                if self.start(&mut inner, ip, false).await.is_err() {
                    self.auth.clear_phone_token();
                    self.auth.end_phone_sessions();
                    inner.mode = Mode::Paused(BIND_FAILED);
                }
            }
            Step::Pause(reason) => {
                self.shut(&mut inner, false).await;
                inner.mode = Mode::Paused(reason);
            }
            Step::Resume(ip) => {
                if self.start(&mut inner, ip, true).await.is_err() {
                    if matches!(inner.mode, Mode::Paused(BIND_FAILED)) {
                        return true;
                    }
                    inner.mode = Mode::Paused(BIND_FAILED);
                }
            }
        }
        self.changed();
        true
    }

    async fn status_of(&self, inner: &Inner) -> PhoneStatus {
        let off = PhoneStatus {
            state: PhoneState::Off,
            reason: None,
            url: None,
            firewall: None,
            category: None,
            demo: self.demo,
        };
        match &inner.mode {
            Mode::Off => off,
            Mode::Paused(reason) => PhoneStatus {
                state: PhoneState::Paused,
                reason: Some((*reason).to_string()),
                category: self.platform.network_category().await.ok(),
                ..off
            },
            Mode::On(running) => {
                let category = self.platform.network_category().await.ok();
                let firewall = match category {
                    Some(category) => self
                        .platform
                        .firewall_status(&self.exe, running.addr.port())
                        .await
                        .ok()
                        .map(|status| status.verdict_for(category)),
                    None => None,
                };
                PhoneStatus {
                    state: PhoneState::On,
                    url: self
                        .auth
                        .phone_token()
                        .map(|token| format!("http://{}/?t={token}", running.addr)),
                    firewall,
                    category,
                    ..off
                }
            }
        }
    }
}
