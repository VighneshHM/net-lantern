//! The opt-in check for a newer Net Lantern on GitHub Releases.
//!
//! Nothing is requested until the user has said yes. The one request is a
//! plain GET of [`RELEASES_API`] (no query, no identifiers, no redirects);
//! the HTTP client sends only the User-Agent `netlantern/<version>`. Only a
//! newer stable release whose link stays under [`RELEASES_PAGE_PREFIX`] is
//! ever reported.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use netlantern_platform::NetIo;
use serde::Serialize;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::error::CoreError;
use crate::events::{EventBus, EventKind};
use crate::monitor::Clock;
use crate::store::Store;

pub const RELEASES_API: &str =
    "https://api.github.com/repos/VighneshHM/net-lantern/releases/latest";
pub const RELEASES_PAGE_PREFIX: &str = "https://github.com/VighneshHM/net-lantern/releases/";
/// `"on"` or `"off"`; missing means the user has not answered.
pub const UPDATE_CHECK_KEY: &str = "update_check";

const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UpdateStatus {
    /// `None`: not answered yet.
    pub setting: Option<bool>,
    pub current: String,
    pub latest: Option<String>,
    pub newer: bool,
    /// Only when `newer`, and only under [`RELEASES_PAGE_PREFIX`].
    pub url: Option<String>,
    pub checked_at: Option<i64>,
}

/// What the last good answer said.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Known {
    latest: Option<(String, String)>,
    checked_at: Option<i64>,
}

pub struct Updates {
    store: Store,
    netio: Arc<dyn NetIo>,
    bus: Arc<EventBus>,
    clock: Arc<dyn Clock>,
    current: &'static str,
    known: Mutex<Known>,
    /// Wakes the checker when the setting changes.
    wake: Notify,
    /// Set after a failure was logged, cleared by the next good answer.
    warned: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Updates {
    pub fn new(
        store: Store,
        netio: Arc<dyn NetIo>,
        bus: Arc<EventBus>,
        clock: Arc<dyn Clock>,
        current: &'static str,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            netio,
            bus,
            clock,
            current,
            known: Mutex::new(Known::default()),
            wake: Notify::new(),
            warned: AtomicBool::new(false),
            task: Mutex::new(None),
        })
    }

    /// Checks at start (if on), then every 24 hours, and right after the
    /// setting turns on. A second call does nothing.
    pub fn start(self: &Arc<Self>) {
        let mut task = self.task.lock().unwrap_or_else(|e| e.into_inner());
        if task.is_some() {
            return;
        }
        let this = Arc::clone(self);
        *task = Some(tokio::spawn(async move {
            loop {
                if matches!(this.enabled().await, Ok(Some(true))) {
                    this.check_now().await;
                }
                tokio::select! {
                    () = tokio::time::sleep(CHECK_EVERY) => {}
                    () = this.wake.notified() => {}
                }
            }
        }));
    }

    pub fn stop(&self) {
        let task = self.task.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            task.abort();
        }
    }

    pub async fn status(&self) -> Result<UpdateStatus, CoreError> {
        let setting = self.enabled().await?;
        Ok(self.build(setting))
    }

    /// Saves the answer. Turning on checks right away (in the background);
    /// turning off forgets what was learned.
    pub async fn set_enabled(&self, on: bool) -> Result<UpdateStatus, CoreError> {
        let before = self.status().await?;
        let value = if on { "on" } else { "off" };
        self.store
            .set_global_setting(UPDATE_CHECK_KEY, value)
            .await?;
        if !on {
            *self.known.lock().unwrap_or_else(|e| e.into_inner()) = Known::default();
        }
        let after = self.build(Some(on));
        if after != before {
            self.bus.publish(EventKind::UpdateChanged {});
        }
        self.wake.notify_one();
        Ok(after)
    }

    async fn enabled(&self) -> Result<Option<bool>, CoreError> {
        Ok(self
            .store
            .global_setting(UPDATE_CHECK_KEY)
            .await?
            .map(|v| v == "on"))
    }

    fn build(&self, setting: Option<bool>) -> UpdateStatus {
        let known = self.known.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let (latest, url) = match known.latest {
            Some((version, url)) => {
                let newer = is_newer(&version, self.current);
                (Some(version), newer.then_some(url))
            }
            None => (None, None),
        };
        UpdateStatus {
            setting,
            current: self.current.to_string(),
            newer: url.is_some(),
            latest,
            url,
            checked_at: known.checked_at,
        }
    }

    /// One request. A failure or an unusable answer leaves what was known.
    pub(crate) async fn check_now(&self) {
        let before = self.build(None);
        let outcome = match self.netio.http_get(RELEASES_API, false).await {
            Ok(res) if res.status == 200 => Ok(parse_release(res.body_prefix.as_bytes())),
            // Never the body: it is the server's text, not ours.
            Ok(res) => Err(format!("the server answered {}", res.status)),
            Err(e) => Err(e.to_string()),
        };
        match outcome {
            Err(reason) => {
                if !self.warned.swap(true, Ordering::SeqCst) {
                    tracing::warn!("the update check failed: {reason}");
                }
                return;
            }
            Ok(release) => {
                self.warned.store(false, Ordering::SeqCst);
                *self.known.lock().unwrap_or_else(|e| e.into_inner()) = Known {
                    latest: release,
                    checked_at: Some(self.clock.now_ms()),
                };
            }
        }
        if self.build(None) != before {
            self.bus.publish(EventKind::UpdateChanged {});
        }
    }
}

/// `X.Y.Z` with an optional leading `v`, digits only (no pre-release or
/// build suffix).
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = tag.split('.');
    let mut next = || -> Option<u64> {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// The version (without `v`) and page of the latest release, if it is a
/// stable one that links under [`RELEASES_PAGE_PREFIX`].
pub(crate) fn parse_release(body: &[u8]) -> Option<(String, String)> {
    let json: serde_json::Value = serde_json::from_slice(body).ok()?;
    if json.get("draft")?.as_bool()? || json.get("prerelease")?.as_bool()? {
        return None;
    }
    let tag = json.get("tag_name")?.as_str()?;
    parse_version(tag)?;
    let url = json.get("html_url")?.as_str()?;
    if !url.starts_with(RELEASES_PAGE_PREFIX) {
        return None;
    }
    let version = tag.strip_prefix('v').unwrap_or(tag);
    Some((version.to_string(), url.to_string()))
}

pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;
    use netlantern_platform::{HttpResponse, MdnsService, NetbiosInfo, PlatformError, SsdpReply};

    use crate::store::Db;

    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> i64 {
            1_234
        }
    }

    /// Answers every `http_get` with `reply` and counts the calls.
    struct Fake {
        calls: AtomicUsize,
        urls: Mutex<Vec<(String, bool)>>,
        reply: Mutex<Result<HttpResponse, PlatformError>>,
    }

    impl Fake {
        fn new(reply: Result<HttpResponse, PlatformError>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                urls: Mutex::new(Vec::new()),
                reply: Mutex::new(reply),
            })
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl NetIo for Fake {
        async fn dns_resolve(&self, _: Ipv4Addr, _: &str) -> Result<Vec<Ipv4Addr>, PlatformError> {
            unreachable!()
        }
        async fn http_get(&self, url: &str, follow: bool) -> Result<HttpResponse, PlatformError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.urls.lock().unwrap().push((url.to_string(), follow));
            self.reply.lock().unwrap().clone()
        }
        async fn reverse_dns(
            &self,
            _: Ipv4Addr,
            _: Ipv4Addr,
        ) -> Result<Option<String>, PlatformError> {
            unreachable!()
        }
        async fn mdns_browse(
            &self,
            _: Ipv4Addr,
            _: Duration,
        ) -> Result<Vec<MdnsService>, PlatformError> {
            unreachable!()
        }
        async fn ssdp_search(
            &self,
            _: Ipv4Addr,
            _: Duration,
        ) -> Result<Vec<SsdpReply>, PlatformError> {
            unreachable!()
        }
        async fn netbios_status(
            &self,
            _: Ipv4Addr,
            _: Duration,
        ) -> Result<Option<NetbiosInfo>, PlatformError> {
            unreachable!()
        }
    }

    fn ok(status: u16, body: &str) -> Result<HttpResponse, PlatformError> {
        Ok(HttpResponse {
            status,
            location: None,
            server: None,
            body_prefix: body.to_string(),
        })
    }

    fn release(tag: &str, url: &str, prerelease: bool) -> String {
        serde_json::json!({
            "tag_name": tag, "html_url": url, "draft": false, "prerelease": prerelease
        })
        .to_string()
    }

    fn good_url(tag: &str) -> String {
        format!("{RELEASES_PAGE_PREFIX}tag/{tag}")
    }

    fn make(fake: &Arc<Fake>) -> (Arc<Updates>, Store, Arc<EventBus>) {
        let store = Store::new(Db::open_in_memory().unwrap());
        let bus = Arc::new(EventBus::new(16));
        let updates = Updates::new(
            store.clone(),
            fake.clone(),
            Arc::clone(&bus),
            Arc::new(FixedClock),
            "0.1.0",
        );
        (updates, store, bus)
    }

    /// Lets the checker task and the store thread run.
    async fn settle() {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn update_check_ignores_errors_prereleases_and_foreign_links() {
        let cases: Vec<Result<HttpResponse, PlatformError>> = vec![
            Err(PlatformError::Io {
                message: "offline".into(),
            }),
            ok(403, r#"{"message":"API rate limit exceeded"}"#),
            ok(200, "{ not json"),
            ok(200, &release("v0.9.0", &good_url("v0.9.0"), true)),
            ok(
                200,
                &release("v0.9.0-beta.1", &good_url("v0.9.0-beta.1"), false),
            ),
            ok(
                200,
                &release("v0.9.0", "https://evil.example/releases/tag/v0.9.0", false),
            ),
            ok(200, &release("v0.1.0", &good_url("v0.1.0"), false)),
        ];
        for reply in cases {
            let fake = Fake::new(reply.clone());
            let (u, _, _) = make(&fake);
            u.set_enabled(true).await.unwrap();
            u.check_now().await;
            let s = u.status().await.unwrap();
            assert!(!s.newer && s.url.is_none(), "{reply:?} -> {s:?}");
        }

        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let (u, _, _) = make(&fake);
        u.set_enabled(true).await.unwrap();
        u.check_now().await;
        let s = u.status().await.unwrap();
        assert_eq!(s.latest.as_deref(), Some("0.2.0"));
        assert!(s.newer);
        assert_eq!(s.url, Some(good_url("v0.2.0")));
        assert_eq!(s.checked_at, Some(1_234));
        assert_eq!(s.current, "0.1.0");
        // GET of the one URL, no redirects followed.
        assert_eq!(
            *fake.urls.lock().unwrap(),
            vec![(RELEASES_API.to_string(), false)]
        );

        let fake = Fake::new(ok(200, &release("0.2.0", &good_url("0.2.0"), false)));
        let (u, _, _) = make(&fake);
        u.set_enabled(true).await.unwrap();
        u.check_now().await;
        assert!(u.status().await.unwrap().newer);
    }

    #[test]
    fn a_draft_and_missing_fields_are_ignored() {
        let draft = serde_json::json!({"tag_name":"v1.0.0","html_url":good_url("v1.0.0"),
            "draft":true,"prerelease":false});
        assert_eq!(parse_release(draft.to_string().as_bytes()), None);
        assert_eq!(parse_release(b"{}"), None);
        assert_eq!(parse_release(b""), None);
    }

    #[tokio::test(start_paused = true)]
    async fn not_answered_means_no_request() {
        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let (u, _, _) = make(&fake);
        u.start();
        settle().await;
        tokio::time::sleep(Duration::from_secs(72 * 3600)).await;
        settle().await;
        assert_eq!(fake.calls(), 0);
        let s = u.status().await.unwrap();
        assert_eq!(s.setting, None);
        assert!(!s.newer);
        u.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn checks_at_start_and_every_24_hours_when_on() {
        let fake = Fake::new(ok(200, &release("v0.1.0", &good_url("v0.1.0"), false)));
        let (u, store, _) = make(&fake);
        store
            .set_global_setting(UPDATE_CHECK_KEY, "on")
            .await
            .unwrap();
        u.start();
        settle().await;
        assert_eq!(fake.calls(), 1);
        tokio::time::sleep(Duration::from_secs(23 * 3600)).await;
        settle().await;
        assert_eq!(fake.calls(), 1);
        tokio::time::sleep(Duration::from_secs(2 * 3600)).await;
        settle().await;
        assert_eq!(fake.calls(), 2);
        u.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn turning_on_checks_now() {
        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let (u, _, bus) = make(&fake);
        let mut events = bus.subscribe();
        u.start();
        settle().await;
        assert_eq!(fake.calls(), 0);
        let s = u.set_enabled(true).await.unwrap();
        assert_eq!(s.setting, Some(true));
        settle().await;
        assert_eq!(fake.calls(), 1);
        assert!(u.status().await.unwrap().newer);
        let mut kinds = Vec::new();
        while let Ok(e) = events.try_recv() {
            kinds.push(e.kind);
        }
        assert!(kinds.iter().all(|k| *k == EventKind::UpdateChanged {}));
        assert!(!kinds.is_empty());
        u.stop();
    }

    #[tokio::test(start_paused = true)]
    async fn turning_off_stops_further_checks() {
        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let (u, store, _) = make(&fake);
        store
            .set_global_setting(UPDATE_CHECK_KEY, "on")
            .await
            .unwrap();
        u.start();
        settle().await;
        assert_eq!(fake.calls(), 1);
        let s = u.set_enabled(false).await.unwrap();
        assert_eq!((s.setting, s.newer, s.latest), (Some(false), false, None));
        tokio::time::sleep(Duration::from_secs(72 * 3600)).await;
        settle().await;
        assert_eq!(fake.calls(), 1);
        u.stop();
    }

    #[test]
    fn is_newer_compares_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(!is_newer("0.9.9", "0.10.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("1.0.0-rc.1", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
