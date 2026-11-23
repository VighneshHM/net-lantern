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
            // A permit stored by a turn-on before `start` must not cause a
            // second check right after the first.
            let _ = futures::FutureExt::now_or_never(this.wake.notified());
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

    /// Forgets what the last check learned, for "Delete all data" (which
    /// also deletes the saved answer, so the question is asked again). The
    /// checker keeps running: with no answer saved it sends nothing, and a
    /// later turn-on wakes it as usual.
    pub fn forget(&self) {
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = Known::default();
        self.warned.store(false, Ordering::SeqCst);
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
        if on && before.setting != Some(true) {
            self.wake.notify_one();
        }
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
        let (latest, url) = match known.latest.filter(|_| setting == Some(true)) {
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
            checked_at: known.checked_at.filter(|_| setting == Some(true)),
        }
    }

    /// One request. A failure or an answer with no usable release leaves
    /// what was known.
    pub(crate) async fn check_now(&self) {
        let before = self.build(Some(true));
        let outcome = match self.netio.http_get(RELEASES_API, false).await {
            Ok(res) if res.status == 200 => parse_release(res.body_prefix.as_bytes())
                .ok_or_else(|| "the answer has no usable release".to_string()),
            // Never the body: it is the server's text, not ours.
            Ok(res) => Err(format!("the server answered {}", res.status)),
            Err(e) => Err(e.to_string()),
        };
        // The user may have said no while the request was out.
        if !matches!(self.enabled().await, Ok(Some(true))) {
            return;
        }
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
                    latest: Some(release),
                    checked_at: Some(self.clock.now_ms()),
                };
            }
        }
        if self.build(Some(true)) != before {
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

/// The four top-level keys of the release that matter.
#[derive(Default)]
struct Fields {
    tag: Option<String>,
    url: Option<String>,
    draft: Option<bool>,
    prerelease: Option<bool>,
}

impl Fields {
    fn complete(&self) -> bool {
        self.tag.is_some()
            && self.url.is_some()
            && self.draft.is_some()
            && self.prerelease.is_some()
    }
}

struct FieldsVisitor<'a>(&'a mut Fields);

impl<'de> serde::de::Visitor<'de> for FieldsVisitor<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a release object")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let fields = self.0;
        // Stops at the first error and as soon as all four are known, so the
        // rest (which may be cut off) is never read. Nested values are
        // skipped, never searched.
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "tag_name" => fields.tag = Some(map.next_value()?),
                "html_url" => fields.url = Some(map.next_value()?),
                "draft" => fields.draft = Some(map.next_value()?),
                "prerelease" => fields.prerelease = Some(map.next_value()?),
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            if fields.complete() {
                break;
            }
        }
        Ok(())
    }
}

/// The version (without `v`) and page of the latest release, if it is a
/// stable one that links under [`RELEASES_PAGE_PREFIX`]. `body` may be cut
/// off (the HTTP client keeps only a prefix), as long as all four keys came
/// first.
pub(crate) fn parse_release(body: &[u8]) -> Option<(String, String)> {
    use serde::Deserializer;
    let mut fields = Fields::default();
    // The result is ignored on purpose: a cut-off or trailing document is an
    // error after the keys were read, and `complete` is the real test.
    let _ = serde_json::Deserializer::from_slice(body).deserialize_map(FieldsVisitor(&mut fields));
    if !fields.complete() || fields.draft? || fields.prerelease? {
        return None;
    }
    let (tag, url) = (fields.tag?, fields.url?);
    parse_version(&tag)?;
    if !url.starts_with(RELEASES_PAGE_PREFIX) {
        return None;
    }
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    Some((version, url))
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
        /// When set, `http_get` waits for a permit before answering.
        gate: Mutex<Option<Arc<Notify>>>,
    }

    impl Fake {
        fn new(reply: Result<HttpResponse, PlatformError>) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                urls: Mutex::new(Vec::new()),
                reply: Mutex::new(reply),
                gate: Mutex::new(None),
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
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.notified().await;
            }
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

    /// GitHub's key order, with `author` and `assets` (and their own
    /// `tag_name`/`html_url`) around the keys that matter, then a long body.
    fn github_release(tag: &str) -> String {
        let user = |n: u32| {
            format!(
                r#"{{"login":"u{n}","id":{n},"node_id":"MDQ6VXNlcjE=","avatar_url":"https://avatars.githubusercontent.com/u/{n}?v=4","url":"https://api.github.com/users/u{n}","html_url":"https://github.com/u{n}","followers_url":"https://api.github.com/users/u{n}/followers","type":"User","site_admin":false}}"#
            )
        };
        let assets: Vec<String> = (0..8)
            .map(|n| {
                format!(
                    r#"{{"url":"https://api.github.com/repos/o/r/releases/assets/{n}","id":{n},"name":"netlantern-{n}.zip","tag_name":"v9.9.9","uploader":{},"content_type":"application/zip","size":{},"browser_download_url":"https://github.com/o/r/releases/download/{tag}/netlantern-{n}.zip"}}"#,
                    user(n + 10),
                    1000 + n
                )
            })
            .collect();
        format!(
            r#"{{"url":"https://api.github.com/repos/o/r/releases/1","assets_url":"https://api.github.com/repos/o/r/releases/1/assets","html_url":"{page}","id":1,"author":{author},"node_id":"RE_1","tag_name":"{tag}","target_commitish":"main","name":"{tag}","draft":false,"prerelease":false,"created_at":"2026-01-01T00:00:00Z","assets":[{assets}],"tarball_url":"https://api.github.com/repos/o/r/tarball/{tag}","body":"{body}"}}"#,
            page = good_url(tag),
            author = user(1),
            assets = assets.join(","),
            body = "release notes ".repeat(3000),
        )
    }

    #[test]
    fn a_release_cut_at_16_kib_is_still_read() {
        let full = github_release("v0.2.0");
        assert!(full.len() > 2 * 16 * 1024, "{}", full.len());
        let cut = &full.as_bytes()[..16 * 1024];
        assert!(serde_json::from_slice::<serde_json::Value>(cut).is_err());
        assert_eq!(
            parse_release(cut),
            Some(("0.2.0".to_string(), good_url("v0.2.0")))
        );
        // Cut in the middle of a string of the body, too.
        let mid_string = &full.as_bytes()[..full.len() - 5000];
        assert_eq!(
            parse_release(mid_string),
            Some(("0.2.0".to_string(), good_url("v0.2.0")))
        );
    }

    #[tokio::test]
    async fn a_long_github_answer_is_reported_through_the_check() {
        let full = github_release("v0.2.0");
        let cut = String::from_utf8(full.as_bytes()[..16 * 1024].to_vec()).unwrap();
        let fake = Fake::new(ok(200, &cut));
        let (u, _, _) = make(&fake);
        u.set_enabled(true).await.unwrap();
        u.check_now().await;
        let s = u.status().await.unwrap();
        assert!(s.newer, "{s:?}");
        assert_eq!(s.latest.as_deref(), Some("0.2.0"));
        assert_eq!(s.url, Some(good_url("v0.2.0")));
    }

    #[test]
    fn a_prefix_that_ends_before_prerelease_gives_no_release() {
        let body = format!(
            r#"{{"html_url":"{}","tag_name":"v0.2.0","draft":false,"#,
            good_url("v0.2.0")
        );
        assert_eq!(parse_release(body.as_bytes()), None);
        let no_draft = format!(
            r#"{{"html_url":"{}","tag_name":"v0.2.0","prerelease":false}}"#,
            good_url("v0.2.0")
        );
        assert_eq!(parse_release(no_draft.as_bytes()), None);
    }

    #[test]
    fn nested_tag_names_and_links_are_not_the_release() {
        // No top-level tag_name: only nested ones.
        let nested_only = format!(
            r#"{{"html_url":"{}","author":{{"tag_name":"v9.9.9"}},"assets":[{{"tag_name":"v9.9.9"}}],"draft":false,"prerelease":false}}"#,
            good_url("v9.9.9")
        );
        assert_eq!(parse_release(nested_only.as_bytes()), None);
        // A nested one before the real one does not win.
        let both = format!(
            r#"{{"author":{{"tag_name":"v9.9.9","html_url":"https://evil.example/"}},"html_url":"{}","tag_name":"v0.2.0","draft":false,"prerelease":false}}"#,
            good_url("v0.2.0")
        );
        assert_eq!(
            parse_release(both.as_bytes()),
            Some(("0.2.0".to_string(), good_url("v0.2.0")))
        );
    }

    #[tokio::test]
    async fn an_unusable_answer_keeps_the_previous_release() {
        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let (u, _, _) = make(&fake);
        u.set_enabled(true).await.unwrap();
        u.check_now().await;
        let before = u.status().await.unwrap();
        assert!(before.newer);
        for bad in ["{ not json", "{}"] {
            *fake.reply.lock().unwrap() = ok(200, bad);
            u.check_now().await;
            assert_eq!(u.status().await.unwrap(), before, "{bad}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_in_flight_when_turned_off_is_discarded() {
        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let gate = Arc::new(Notify::new());
        *fake.gate.lock().unwrap() = Some(Arc::clone(&gate));
        let (u, store, _) = make(&fake);
        store
            .set_global_setting(UPDATE_CHECK_KEY, "on")
            .await
            .unwrap();
        u.start();
        settle().await;
        assert_eq!(fake.calls(), 1, "the request is in flight");
        u.set_enabled(false).await.unwrap();
        gate.notify_one();
        settle().await;
        assert_eq!(*u.known.lock().unwrap(), Known::default());
        let s = u.status().await.unwrap();
        assert_eq!(
            (s.setting, s.newer, s.latest, s.url),
            (Some(false), false, None, None)
        );
        u.stop();
    }

    #[tokio::test]
    async fn after_delete_all_data_the_question_is_asked_again_and_nothing_is_known() {
        let fake = Fake::new(ok(200, &release("v0.2.0", &good_url("v0.2.0"), false)));
        let (u, store, _) = make(&fake);
        u.set_enabled(true).await.unwrap();
        u.check_now().await;
        assert!(u.status().await.unwrap().newer);

        store.reset_all().await.unwrap();
        u.forget();
        let s = u.status().await.unwrap();
        assert_eq!((s.setting, s.newer, s.latest), (None, false, None));
        // Said yes again: nothing from before shows until a new check.
        let s = u.set_enabled(true).await.unwrap();
        assert_eq!((s.latest, s.checked_at, s.url), (None, None, None));
    }

    #[tokio::test(start_paused = true)]
    async fn turning_on_twice_or_before_start_checks_once() {
        let fake = Fake::new(ok(200, &release("v0.1.0", &good_url("v0.1.0"), false)));
        let (u, _, _) = make(&fake);
        u.set_enabled(true).await.unwrap();
        u.start();
        settle().await;
        assert_eq!(fake.calls(), 1, "no stored permit from before start");
        u.set_enabled(true).await.unwrap();
        settle().await;
        assert_eq!(fake.calls(), 1, "already on: no extra request");
        u.stop();
    }
}
