# Phase 7a: Desktop App and In-App Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the Phase 3 Tauri spike into the real Net Lantern desktop app, which hosts the server in-process, keeps running in the tray, can start at login and sends desktop notifications. Add the in-app operations Phase 6 deferred: rotating log files, "Copy debug info", an opt-in update check and "Delete all data".

**Architecture:**
- **Shared host:** the startup code in `crates/netlantern/src/main.rs` moves into a reusable `host` module in the `netlantern` library. It handles the single-instance pipe, data folder, store, port, `AppState`, phone access, monitor and clean shutdown. Both `netlantern.exe` and the desktop app call it, so there is one startup path and the monitor is always started (the spike's surprise).
- **Server:** gains four small features, each a route plus a core or store piece:
  - redacted debug info;
  - the update check;
  - the data reset;
  - a "host" capability for start-at-login, injected by the desktop app as a trait object.
- **Desktop app:** `apps/desktop/src-tauri`, its own Cargo workspace as today. It adds:
  - a tray icon with Open and Quit;
  - closing the window hides it;
  - the Tauri notification and autostart plugins;
  - a second launch focuses the window.
- **UI:** the Settings cards and the Dashboard update question and banner.

**Tech Stack:**
- Rust: tokio, axum, rusqlite, `tracing-appender` (new), `windows` 0.62.
- Tauri v2 with `tauri-plugin-notification` and `tauri-plugin-autostart` (new); tray via Tauri's `tray-icon` feature.
- UI: React 19 + TypeScript, TanStack Query, Tailwind 4, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`. The relevant sections are:
- §2.1: Startup and footprint targets.
- §4: run model, single instance, tray, start at login.
- §6.9: desktop notifications via the Tauri plugin.
- §6.11: tokens on localhost and in the webview.
- §9: Settings.
- §10.1: update check, uninstall/data reset, logs, crashes, copy debug info.
- §12: the phase 7 row.
- §13: the checkpoint and learning goals.
- §16: the Tauri and location-rule risks.

**Branch:** `phase-7a-desktop-app`, created from `phase-6-phone-settings`. Phase 6 is pushed but not merged yet, so this branch is stacked on it.

**Out of this plan:**
- **Phase 7b:** the per-user installer with the data-removal choice, the GitHub Actions release, the unsigned build with SmartScreen guidance, and the README and screenshots.
- **Phase 7c:** the landing site, which still needs the author's palette and background picks.

## Decisions this plan makes (for the author's review)

1. **One data folder.** The desktop app and `netlantern.exe` share `%LOCALAPPDATA%\NetLantern`: one database and one `logs` folder. The single-instance pipe already makes sure only one runs, so they never write the same database at once.
2. **Start at login** is off by default, as in spec §4. When the app is launched at login it starts **hidden in the tray**. Monitoring runs, and no window pops up.
3. **Closing the window** hides it to the tray. The first time, a notification says "Net Lantern is still running in the tray. Quit it from the tray menu." It is shown once per install, saved as a global setting.
4. **Update check:**
   - It is unanswered until the user chooses; the first answer is asked on the Dashboard and can be changed in Settings.
   - When on, it checks once at start and then every 24 hours.
   - It makes a single GET to the GitHub Releases API for this repository and sends nothing else.
   - The banner links only to a `https://github.com/<repo>/releases/...` address. The repository name lives in one constant.
5. **Delete all data:**
   - It clears every table, including settings and per-network consent, so it is a true fresh start. It turns phone access off first and keeps the log files.
   - It needs a confirm dialog with the exact button "Delete all data".
   - The monitor is stopped and restarted around it, and a running scan is cancelled first.
6. **Debug info redaction:**
   - Every IPv4 address becomes `x.x.x.x`, except `1.1.1.1`, `8.8.8.8` and `127.0.0.1`.
   - Every MAC address becomes `xx:xx:xx:xx:xx:xx`.
   - Every SSID and device name or hostname the store knows becomes `[name]`.
   - The text is built and redacted on the server, so the UI never sees unredacted log lines.
7. **The exe's toasts stay as they are** (PowerShell app id, Phase 3 route B). The desktop app uses the Tauri notification plugin.

## Global Constraints

- Spec §4:
  - "The Tauri app keeps running in the **system tray** when its window is closed, so monitoring, history and alerts continue. 'Quit' in the tray menu stops it."
  - "**Start at login** is offered in Settings and is **off by default**."
  - "**Single instance.** Only one Net Lantern (exe or Tauri app) runs per user… A second launch does not start a server or open the database. It connects to the pipe and sends 'open', and the **running instance** issues a fresh one-time token and opens its dashboard (or focuses its window)."
- Spec §6.11: "the Tauri webview loads the UI from the same server with a token. The token is exchanged for a session cookie (`HttpOnly`, `SameSite=Strict`)."
- Spec §10.1:
  - "**Update check:** the first-run screen asks whether Net Lantern may check GitHub Releases for new versions (no default pre-selected; changeable in Settings). The check is one request for the latest version number; it sends no scan data. A newer version shows a banner linking to the release page. No automatic download or install in v1."
  - "**Logs:** structured logs (`tracing`) to rotating files in `%LOCALAPPDATA%\NetLantern\logs`, kept 7 days. Session tokens are never logged."
  - "**Crashes:** a panic hook writes the panic to the log; the monitoring session ends with reason `crash`." The `crash` end reason is already written for sessions left open, at the next open (`store/sessions.rs`). Keep it.
  - "**Copy debug info:** Settings button that copies Net Lantern version, Windows build, interface type, permission/firewall states, recent errors and the last log lines to the clipboard, with IPs, MACs, SSIDs and device names redacted. Nothing is sent anywhere."
  - "Settings → data reset clears data without uninstalling."
- Spec §2.1 targets checked in this phase:
  - Startup: "Dashboard reachable < 2 s after launch".
  - Footprint: "Tauri app with window open: < 200 MB RAM", and "Server process: < 1% average CPU, < 50 MB RAM".
- Never change a Windows setting without the user's explicit action. Start at login is written only when the user turns it on and removed when they turn it off. Nothing else touches the registry.
- Tokens, session ids and the login URL are never logged, written to log files or included in debug info.
- `netlantern.exe` stays a supported way to run Net Lantern (spec §4). Its behaviour and flags are unchanged except where this plan says.
- Build with `CARGO_TARGET_DIR=C:/Users/vighn/AppData/Local/Temp/nl-target` for the root workspace. Never kill a `netlantern.exe` or Net Lantern app you did not start. Use `NETLANTERN_PIPE_NAME=<unique>` for any instance you run. Cargo may need `export PATH="$HOME/.cargo/bin:$PATH"`.
- UI: design tokens only, touch targets at least 44 px, status as text, plain-English copy exactly as written here, and real "…", "—", "·", "→" characters (no U+FFFD).
- Commits are one plain sentence, with no prefix, no body, and no Co-Authored-By or Claude attribution (the author's rule).
- Per-task verification:
  - Rust: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
  - Desktop crate: the same three commands run inside `apps/desktop/src-tauri`.
  - UI: `npm run lint`, `npx tsc -b` and `npm test`.

## Review Focus

1. **Quitting from the tray while a scan, a diagnosis or phone access is running, or while the window is hidden.** The app must stop the phone listener, the monitor (session ended `quit`) and the server, and exit within about 3 s, with no orphan process holding the pipe or port. Test: Task 1, `shutdown_ends_the_session_as_quit_and_frees_the_port_and_pipe`. Checked by hand in checkpoint item 4.
2. **A second launch while the app is hidden in the tray, or while `netlantern.exe` runs.** Expected:
   - the desktop app shows and focuses its window, with no second server and no database open;
   - when the exe is the running one, it opens the browser, as today.

   Test: Task 1, `second_start_reports_already_running_and_calls_on_open`.
3. **The update check with no internet, a GitHub error, rate limiting, a malformed reply, a pre-release, a `v` prefix, or a link to another host.** Expected:
   - none of these crash or show a banner;
   - only a strictly newer stable `vX.Y.Z` gives a banner;
   - only a github.com release URL is ever linked.

   Test: Task 4, `update check ignores errors, prereleases and foreign links`.
4. **Debug info leaking a personal value through a path the redactor doesn't know.** Examples:
   - a device name inside a log line;
   - an IPv6 address;
   - an SSID with regex characters;
   - the Windows username in a file path.

   The text must contain none of them. Test: Task 3, `redaction removes names, addresses, ssids and the user folder`.
5. **Delete all data while monitoring, a scan or phone access is active, or clicked twice.** Expected:
   - everything is cleared;
   - the monitor restarts on the empty database (waiting for a scan);
   - phone access is off;
   - no half-reset state;
   - the second click is harmless.

   Test: Task 5, `reset during a running scan and monitor leaves a clean empty store`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/netlantern/src/host.rs` (new) | Shared startup and shutdown: instance pipe, data folder, store, port, `AppState`, phone access, monitor, pipe "open" handling. |
| `crates/netlantern/src/logging.rs` (new) | stderr and rotating-file subscribers, 7-day pruning, recent-lines tap, panic hook. |
| `crates/netlantern/src/main.rs` (modify) | The exe becomes a thin caller of `host` and `logging`. |
| `crates/netlantern-server/src/debug_info.rs` (new) | Builds and redacts the debug text; route `GET /api/debug-info`. |
| `crates/netlantern-core/src/updates.rs` (new) | The update setting, the GitHub check, version comparison, the 24-hour loop. |
| `crates/netlantern-core/src/store/reset.rs` (new) | `Store::reset_all`. |
| `crates/netlantern-server/src/{routes,state,error,lib}.rs` (modify) | Routes for update, reset and host; `HostInfo` and the `Autostart` trait in state. |
| `crates/netlantern-platform/src/{platform,win,sim}…` (modify) | `os_description()` for the Windows build. |
| `apps/desktop/src-tauri/*` (rewrite) | The real desktop app: tray, window, plugins, notifier, autostart, single instance through `host`. |
| `ui/src/components/{AppSettings,UpdateCard}.tsx` (new), `ui/src/pages/{Settings,Dashboard}.tsx` (modify), `ui/src/api/ops.ts` (new) | UI for debug info, reset, start at login, the update question, banner and toggle. |
| `docs/checkpoints/phase-7a.md` (new), spec (modify) | Checkpoint and notes. |

---

### Task 1: Shared host module, used by the exe

**Files:**
- Create: `crates/netlantern/src/host.rs`
- Modify: `crates/netlantern/src/lib.rs` (export `host`), `crates/netlantern/src/main.rs` (use it; keep the one-shot modes and CLI as they are), `crates/netlantern-server/src/{state,lib}.rs` (`HostKind`, `Autostart`, `HostInfo`)
- Test: `crates/netlantern/tests/host.rs` (new)

**Interfaces:**
- Produces:

```rust
pub enum Backend { Real { data_dir: Option<PathBuf> }, Demo { scenario: Option<PathBuf>, time_scale: Option<u32> }, Memory }
pub struct HostOptions {
    pub backend: Backend,
    pub port: Option<u16>,                  // None = first free in DEFAULT_PORTS
    pub dev_origins: Vec<String>,
    pub notifier: Arc<dyn Notifier>,
    pub autostart: Option<Arc<dyn Autostart>>, // None for the exe
    pub host_kind: HostKind,                 // Exe | Desktop
    /// Called (on the pipe thread) when a second launch asks this instance to open.
    pub on_open: Arc<dyn Fn(String) + Send + Sync>, // receives a fresh login URL
    pub pipe_name: String,                   // instance::pipe_name() in production
}
pub enum Started { Running(Host), AlreadyRunning }
pub struct Host { /* … */ }
impl Host {
    pub async fn start(opts: HostOptions) -> Result<Started, HostError>; // AlreadyRunning after sending "open"
    pub fn port(&self) -> u16;
    pub fn login_url(&self) -> String;       // fresh one-time token each call; never logged
    pub fn state(&self) -> &AppState;
    /// Serve until `signal` completes, then stop phone access, the monitor (EndReason::Quit) and the server.
    pub async fn serve_until(self, signal: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()>;
}
#[derive(Debug, thiserror::Error)] pub enum HostError { /* NoFreePort, PortInUse(u16), DataDir(String), Store(String), Demo(String) */ }

// Defined in this task in crates/netlantern-server/src/state.rs (Task 6 adds the routes that expose them):
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "snake_case")] pub enum HostKind { Exe, Desktop }
pub trait Autostart: Send + Sync {
    fn is_enabled(&self) -> Result<bool, String>;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
}
#[derive(Clone)] pub struct HostInfo { pub kind: HostKind, pub version: &'static str, pub autostart: Option<Arc<dyn Autostart>> }
// AppState gains `pub host: HostInfo` (version = env!("CARGO_PKG_VERSION"); defaults to Exe/None in tests)
```

**Decisions:**
- **What moves out of `main.rs`:**
  - `build_backend`, `real_backend`, `open_data_store`, `data_dir`, `load_demo`, `open_memory_store`;
  - the bind with the port-error wording;
  - the `AppState` wiring, phone access, `attach_router` and `restore()`;
  - the pipe acquire and `serve_open_requests`;
  - `monitor.start()`;
  - the shutdown order: phone, then monitor, then server.

  Error message text stays identical, because tests already pin some of it.
- **What `main.rs` keeps:** CLI parsing, logging (Task 2), the one-shot modes, `announce()` (print the URL and open the browser), and `shutdown_signal()`.
- **On a second launch:** `on_open` prints and opens the browser with the URL, which is today's behaviour.
- **Pipe on non-Windows:** as today, no pipe; `start` never returns `AlreadyRunning`.

- [ ] **Step 1: Write the failing tests** in `tests/host.rs`. Use `Backend::Memory` or a demo scenario, a unique `pipe_name` per test, and `port: None`.
  - `start_serves_the_dashboard_and_login_url_signs_in_once`: GET `/` is 200. POST `/api/session` with the token from `login_url()` sets a cookie, and the same token a second time is 401.
  - `monitor_is_running_after_start`: `GET /api/monitor` with a session is not `not_connected` on the home demo after a quick scan.
  - `second_start_reports_already_running_and_calls_on_open` (Windows only, `#[cfg(windows)]`): a second `Host::start` with the same pipe returns `AlreadyRunning`, and the first host's `on_open` receives one URL that signs in.
  - `shutdown_ends_the_session_as_quit_and_frees_the_port_and_pipe`: after `serve_until` resolves, the port can be bound again, a new `Host::start` on the same pipe is `Running`, and the monitor session in the store ended with `quit`. Use a temp data folder (`Backend::Real { data_dir: Some(tmp) }`) so the store survives.
  - `port_in_use_error_keeps_the_existing_wording`.
- [ ] **Step 2:** Run `cargo test -p netlantern --test host` and see the tests fail.
- [ ] **Step 3:** Implement `host.rs`. Switch `main.rs` to it, and run the existing exe tests (`crates/netlantern/tests/*`) to prove the behaviour is unchanged.
- [ ] **Step 4:** Run the full workspace verification. Then run `--demo` (unique pipe and port) and check the printed URL signs in; report status codes only.
- [ ] **Step 5:** Commit: "Move startup and shutdown into a shared host module that both the exe and the desktop app can use."

---

### Task 2: Rotating log files, a recent-lines tap, and the panic hook

**Files:**
- Create: `crates/netlantern/src/logging.rs`
- Modify: `crates/netlantern/src/main.rs` (call it), `crates/netlantern/Cargo.toml`, root `Cargo.toml` (`tracing-appender`)
- Test: unit tests in `logging.rs`

**Interfaces:**
- Produces:

```rust
pub struct LogOptions { pub dir: Option<PathBuf>, pub stderr: bool }  // dir = <data folder>/logs; None = no files (demo/memory)
pub struct LogGuard { /* holds the appender worker guard */ }
/// Installs the global subscriber once. Files roll daily as `netlantern.log.YYYY-MM-DD`; files older than 7 days are deleted at start.
pub fn init(opts: LogOptions) -> LogGuard;
/// The last 200 formatted lines at INFO and above, and the last 50 at WARN and above, newest last.
pub fn recent_lines() -> Vec<String>;
pub fn recent_problems() -> Vec<String>;
pub fn install_panic_hook();
pub(crate) fn files_to_prune(names: &[String], today: chrono::NaiveDate /* or (y,m,d) */, keep_days: u32) -> Vec<String>;
pub const KEEP_DAYS: u32 = 7;
```

**Decisions:**
- **Writers:** use `tracing_appender::rolling::daily` with a non-blocking writer. Pruning is our own `files_to_prune` over the folder listing, run at start, so it does not depend on the appender's retention options.
- **Taps:** the recent-lines taps are a `tracing_subscriber::Layer` writing into two bounded `Mutex<VecDeque<String>>` rings.
- **Levels:** INFO, as now. `log` records stay unrouted, as the existing comment explains: the `webbrowser` crate logs the URL.
- **Real run:** the exe logs to stderr and files.
- **Demo or memory backends:** stderr only; no `logs` folder is created.
- **Panic hook:** keeps today's behaviour (logs the panic at ERROR) and also flushes the file writer. The `crash` end reason is already recorded at the next open; leave it.
- **Phone URL:** add a test-level check that no `?t=` appears in a log line produced by `announce()` or phone access. Grep the code: the phone URL must never be logged.

- [ ] **Step 1: Write the failing tests:**
  - `prunes_files_older_than_seven_days_only`: names for today, 6, 7 and 8 days ago, plus `other.txt`. Only the 8-day file is pruned, and unrelated files are untouched.
  - `recent_rings_keep_the_newest_lines_up_to_their_limits`.
  - `problems_ring_only_takes_warn_and_error`.
  - `files_are_written_under_the_logs_folder`: init with a temp dir, log a line, drop the guard, and a `netlantern.log.*` file contains it.
  - `no_launch_or_phone_url_reaches_the_logs`: run `announce()` and an `on_open` with a token URL through the tap, and no captured line contains `?t=`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement, and wire it into `main.rs` (real run: `dir = data_dir/logs`).
- [ ] **Step 4:** Run the full workspace verification.
- [ ] **Step 5:** Commit: "Write logs to daily files kept for seven days, keep the latest lines in memory, and flush them on a panic."

---

### Task 3: Copy debug info (server and redaction)

**Files:**
- Create: `crates/netlantern-server/src/debug_info.rs`
- Modify: `crates/netlantern-server/src/{routes,state,lib}.rs`, `crates/netlantern-platform/src/platform.rs` + `win/platform.rs` + `sim/mod.rs` (`os_description`), `crates/netlantern/src/host.rs` (pass the log taps into `AppState`)
- Test: unit tests in `debug_info.rs`; `crates/netlantern-server/tests/api.rs`

**Interfaces:**
- Consumes: `logging::recent_lines` and `recent_problems` (Task 2), passed into the server as a closure so the server crate does not depend on the exe crate.
- Produces:

```rust
// platform
async fn os_description(&self) -> String; // Platform trait, required. Windows: "Windows 11 (build 26200)" via RtlGetVersion; sim: "Simulated network"
// server state
pub struct LogTaps { pub lines: Arc<dyn Fn() -> Vec<String> + Send + Sync>, pub problems: Arc<dyn Fn() -> Vec<String> + Send + Sync> }
// AppState gains `pub logs: Option<LogTaps>`; the version and host kind come from `state.host` (Task 1)
pub struct Redactor { /* names, ssids, user folder */ }
impl Redactor {
    pub fn new(names: Vec<String>, user_dir: Option<String>) -> Self; // names = every device name, hostname, announced name and SSID label in the store
    pub fn apply(&self, text: &str) -> String;
}
// GET /api/debug-info -> text/plain; charset=utf-8 (redacted)
```

**The debug text layout** (one item per line):
```
Net Lantern <version> · <host kind>
OS: <os_description>
Interface: wifi|ethernet|other|not connected
Wi-Fi details: shown|hidden (location permission)|no Wi-Fi
Network category: Private|Public|Domain|unknown
Phone access: off|on|paused (<reason>) · firewall <verdict or unknown>
Monitoring: <monitor state>

Recent problems:
<up to 50 lines>

Last log lines:
<up to 200 lines>
```

**Redaction rules:**
- IPv4 addresses (word-bounded) become `x.x.x.x`, except 1.1.1.1, 8.8.8.8 and 127.0.0.1.
- IPv6 addresses (anything matching a hex:hex: run with at least 2 colons) become `x::x`.
- MAC addresses in `:` or `-` form, any case, become `xx:xx:xx:xx:xx:xx`.
- Each known name of 2 or more characters is replaced case-insensitively, as literal text (escape regex characters), longest first, with `[name]`.
- The user's home folder (from `USERPROFILE`, or the parent of LOCALAPPDATA's `AppData`) becomes `%USERPROFILE%`.
- Redaction runs on the whole assembled text, headers included.

- [ ] **Step 1: Write the failing tests:**
  - `redaction removes names, addresses, ssids and the user folder`. The input holds:
    - `Lounge TV (192.168.1.52) aa-bb-cc-dd-ee-ff`;
    - `fe80::1c2:3ff:fe4a:5b6c`;
    - SSID `Home (5G)+`;
    - `C:\Users\alice\AppData\Local\NetLantern\logs`;
    - `pinged 1.1.1.1`.

    The output has none of the private values, keeps `1.1.1.1`, and shows `[name]`, `x.x.x.x`, `xx:xx:xx:xx:xx:xx`, `x::x` and `%USERPROFILE%`.
  - `longest_name_wins`: names `TV` and `Lounge TV`.
  - `short_names_are_not_redacted`: a one-letter name does not blank every "a".
  - API: `debug_info_requires_a_session_and_is_redacted`:
    - 401 without a cookie;
    - with a session on the home demo after a scan, the text starts with `Net Lantern ` and contains no IP from the scenario except 1.1.1.1 and 8.8.8.8;
    - it contains no device name from the scenario and no `?t=`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement. Add `Win32_System_SystemInformation`, or the needed Wdk feature for `RtlGetVersion`, read-only. Name the feature in the report.
- [ ] **Step 4:** Run the full workspace verification.
- [ ] **Step 5:** Commit: "Add redacted debug info with the version, Windows build, network state, recent problems and the latest log lines."

---

### Task 4: Opt-in update check (core and API)

**Files:**
- Create: `crates/netlantern-core/src/updates.rs`
- Modify: `crates/netlantern-core/src/{lib,events}.rs`, `crates/netlantern-server/src/{routes,state}.rs`, `crates/netlantern/src/host.rs` (start the checker), `scenarios/home.json` (an `http` entry for the API URL returning a release equal to the current version)
- Test: unit tests in `updates.rs`; `crates/netlantern-server/tests/api.rs`

**Interfaces:**
- Produces:

```rust
pub const RELEASES_API: &str = "https://api.github.com/repos/VighneshHM/net-lantern/releases/latest";
pub const RELEASES_PAGE_PREFIX: &str = "https://github.com/VighneshHM/net-lantern/releases/";
pub const UPDATE_CHECK_KEY: &str = "update_check"; // "on" | "off"; missing = not answered
#[derive(Clone, Debug, PartialEq, Eq, Serialize)] pub struct UpdateStatus {
    pub setting: Option<bool>,          // None = not answered yet
    pub current: String,                // CARGO_PKG_VERSION
    pub latest: Option<String>,         // "0.2.0" when known
    pub newer: bool,
    pub url: Option<String>,            // only when newer, only under RELEASES_PAGE_PREFIX
    pub checked_at: Option<i64>,        // ms
}
pub struct Updates { /* store, netio, bus, clock */ }
impl Updates {
    pub fn new(store: Store, netio: Arc<dyn NetIo>, bus: Arc<EventBus>, clock: Arc<dyn Clock>, current: &'static str) -> Arc<Self>;
    pub fn start(self: &Arc<Self>);            // checks at start (if on) then every 24 h; also right after the setting turns on
    pub async fn status(&self) -> Result<UpdateStatus, CoreError>;
    pub async fn set_enabled(&self, on: bool) -> Result<UpdateStatus, CoreError>;
    pub fn stop(&self);
}
pub(crate) fn parse_release(body: &[u8]) -> Option<(String /*version*/, String /*html_url*/)>; // stable vX.Y.Z or X.Y.Z only; prerelease/draft -> None
pub(crate) fn is_newer(latest: &str, current: &str) -> bool; // numeric semver compare
```

**Decisions:**
- **The request:** `netio.http_get(RELEASES_API, false)`. The existing client already sends a User-Agent. Any error or non-200 means "unknown" and is logged once at WARN without the body.
- **The answer:** `prerelease == true`, `draft == true`, a non-stable tag (with a `-` suffix), or a URL outside the prefix all mean no banner.
- **Events:** publish `update.changed` (no data) when the status changes.
- **Routes:**
  - `GET /api/update` → `UpdateStatus`.
  - `PUT /api/update {enabled: bool}` → `UpdateStatus`.
  - Both need a session.
- **Demo:** the home scenario's API entry returns `tag_name` equal to the workspace version, so there is no banner and no real network call.

- [ ] **Step 1: Write the failing tests** (`updates.rs`), with a sim or fake NetIo and a paused clock: `update check ignores errors, prereleases and foreign links`.
  - No banner for each of:
    - an HTTP error;
    - status 403 (rate limited);
    - malformed JSON;
    - `prerelease: true`;
    - `v0.9.0-beta.1`;
    - `html_url` on `evil.example`;
    - an equal version.
  - `v0.2.0` against current `0.1.0` gives `newer` with the github URL. A `0.2.0` tag without `v` also works.

  Also:
  - `not_answered_means_no_request`: the NetIo call count is 0.
  - `checks_at_start_and_every_24_hours_when_on` (paused clock).
  - `turning_on_checks_now`.
  - `turning_off_stops_further_checks`.
  - `is_newer_compares_numerically` (`0.10.0 > 0.9.9`).

  API: `update_routes_need_a_session_and_round_trip_the_setting`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement, and start and stop `Updates` from `host.rs` (stopped in `serve_until`).
- [ ] **Step 4:** Run the full workspace verification.
- [ ] **Step 5:** Commit: "Add an opt-in check for new versions on GitHub Releases that only reports a newer stable release."

---

### Task 5: Delete all data (store and API)

**Files:**
- Create: `crates/netlantern-core/src/store/reset.rs`
- Modify: `crates/netlantern-core/src/store/mod.rs`, `crates/netlantern-core/src/events.rs` (`data.reset`), `crates/netlantern-server/src/{routes,error}.rs`, `crates/netlantern-server/src/phone.rs` (a public `disable` is already there; use it)
- Test: `crates/netlantern-core/tests/store.rs`, `crates/netlantern-server/tests/api.rs`

**Interfaces:**
- Produces:

```rust
impl Store { pub async fn reset_all(&self) -> Result<(), StoreError>; } // one transaction: DELETE FROM every data table (children first); schema and migration version kept
// POST /api/data/reset {"confirm": "delete all data"} -> 204; wrong/missing confirm -> 400 bad_request
```

**Decisions:**
- **Route order:**
  1. Take a server-wide reset mutex, so concurrent resets queue and the second one finds an empty store and is harmless.
  2. Disable phone access, if present.
  3. Cancel a running scan and wait for it to end, up to 5 s.
  4. `monitor.stop(EndReason::Quit)`.
  5. `store.reset_all()`.
  6. Clear the scan manager's remembered network (add `ScanManager::forget_network()` if none exists).
  7. `monitor.start()`.
  8. Publish `data.reset`.
- **The update setting** goes too, so the question is asked again.
- **Tables:** list every table from `migrations.rs` in `reset_all`. A test fails if a table is added later and not cleared: it queries `sqlite_master` and checks every table except `schema_migrations`, or whatever the migration bookkeeping table is called (read `migrations.rs`), is empty after reset.

- [ ] **Step 1: Write the failing tests:**
  - Store:
    - `reset_all_empties_every_table_but_keeps_the_schema` (via `sqlite_master`);
    - `store_still_works_after_reset`: a new network and device can be written.
  - API:
    - `reset_needs_a_session_and_the_exact_confirm_text`;
    - `reset during a running scan and monitor leaves a clean empty store`:
      1. start a deep scan with consent, and turn phone access on with the loopback binder;
      2. POST reset;
      3. expect 204, `GET /api/devices` empty, `GET /api/monitor` state `waiting_for_scan`, `GET /api/phone-access` state `off`, and a `data.reset` event received;
      4. a second reset also returns 204.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the full workspace verification.
- [ ] **Step 5:** Commit: "Add Delete all data, which stops monitoring, phone access and any scan, clears every table, and starts monitoring again."

---

### Task 6: Host info and start-at-login capability (API)

**Files:**
- Modify: `crates/netlantern-server/src/{state,routes,lib}.rs`, `crates/netlantern/src/host.rs`
- Test: `crates/netlantern-server/tests/api.rs`

**Interfaces:**
- Consumes: `HostKind`, `Autostart` and `HostInfo` from Task 1 (`state.host`).
- Produces:

```rust
// GET /api/host -> { kind: "exe"|"desktop", version: "0.1.0", autostart: { available: bool, enabled: bool | null } }
// PUT /api/host/autostart {enabled: bool} -> same body; 409 autostart_unavailable when None; 500 autostart_failed with the message (no paths) on error
```

- [ ] **Step 1: Write the failing tests** with a fake `Autostart` (an `AtomicBool`) and one that errors:
  - `host_reports_exe_without_autostart`;
  - `desktop_autostart_round_trips`;
  - `autostart_unavailable_is_409`;
  - `autostart_error_is_500_autostart_failed`;
  - `host_routes_need_a_session`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement the two routes over `state.host`.
- [ ] **Step 4:** Run the full workspace verification.
- [ ] **Step 5:** Commit: "Report which host is running and let the desktop app offer start at login through the API."

---

### Task 7: The real desktop app

**Files:**
- Rewrite: `apps/desktop/src-tauri/{Cargo.toml,tauri.conf.json,build.rs,src/main.rs}`
- Create: `apps/desktop/src-tauri/src/{tray,notify,autostart}.rs`, `apps/desktop/src-tauri/capabilities/default.json` if the plugins need it (they need no JS IPC: the UI talks HTTP only)
- Modify: `.gitignore` if needed

**Interfaces:**
- Consumes: `netlantern::host::{Host, HostOptions, Started, Backend}` (Task 1), `netlantern::logging` (Task 2), `HostKind` and `Autostart` (Task 6), and `Notifier` from core.

**Decisions:**
- **Crate:** `name = "netlantern-desktop"`, `productName "Net Lantern"`, `identifier "dev.netlantern.app"`. It stays its own workspace with the `[workspace]` table, so the root CI is unchanged until 7b. It depends on the `netlantern` lib crate by path.
- **Startup in `main`:**
  1. `logging::init` with files in `%LOCALAPPDATA%\NetLantern\logs` and no stderr in release.
  2. `install_panic_hook`.
  3. Build a tokio runtime on a dedicated thread, as in the spike.
  4. `Host::start` with `Backend::Real { data_dir: None }`, `HostKind::Desktop`, the plugin notifier, the plugin autostart, and an `on_open` that asks the Tauri main thread (through an `AppHandle` stored in a `OnceLock` and `run_on_main_thread`) to show, unminimize and focus the window.
  5. On `AlreadyRunning`, exit 0 immediately. The running instance handles it.
  6. The `main` window loads `host.login_url()`.
- **Arguments:** launched with `--hidden` (the autostart plugin passes this arg) means no window is shown until Open or a second launch. When the window is first created hidden, give it the login URL anyway, so the first show is already signed in.
- **Window close:** `CloseRequested` prevents the close and hides the window. The first time, it sends the plugin notification "Net Lantern is still running in the tray. Quit it from the tray menu." It saves the global setting `tray_hint_shown = "on"` through the store, via a tiny helper on `Host`/`AppState`.
- **Tray:** the app icon, tooltip "Net Lantern", and a menu with "Open Net Lantern" (show and focus) and "Quit". A left click shows the window.
- **Quit:** signals `serve_until` to finish, waits for it (phone, monitor, server stop), then `app.exit(0)`. `RunEvent::ExitRequested` without our quit flag, for example when the last window closes, is prevented, so the app keeps running in the tray.
- **Notifier:** `TauriNotifier { handle: AppHandle }`. `notify(title, body)` calls `handle.notification().builder().title(format!("Net Lantern: {title}")).body(body).show()`. Errors are logged once per distinct error, never with the title or body, as in `notify.rs`.
- **Autostart:** wraps `tauri_plugin_autostart::ManagerExt::autolaunch()`, initialised with `MacosLauncher::LaunchAgent` (ignored on Windows) and `Some(vec!["--hidden"])`. `is_enabled` and `set_enabled` map errors to strings.
- **Release profile:** keep the spike's profile, `windows_subsystem = "windows"` in release, and `bundle.active = false` (bundling is 7b).
- **Tests** (pure helpers only; the Tauri runtime is checked by hand):
  - `args_hidden_detects_flag`;
  - `notifier_formats_title`;
  - `login_url_is_never_logged` (reuse Task 2's tap check on the desktop's logging init path, if feasible);
  - `cargo build --release` of the app succeeds.

- [ ] **Step 1:** Write the failing unit tests above.
- [ ] **Step 2:** Run `cargo test` inside `apps/desktop/src-tauri` and see them fail.
- [ ] **Step 3:** Implement. Check the exact plugin APIs against the versions you install: run `cargo add tauri-plugin-notification@2 tauri-plugin-autostart@2` and enable the `tray-icon` feature on `tauri`. Report the versions used.
- [ ] **Step 4:** Run the desktop crate's fmt, clippy and test, then the root workspace verification.
  - Build `cargo build --release` in `apps/desktop/src-tauri`, after `npm run build` in `ui/`.
  - Smoke test it yourself: launch the release binary with `NETLANTERN_PIPE_NAME=<unique>`, confirm `GET /` on the printed port (find it in the log file) answers 200, launch it a second time and confirm the second process exits within 2 s, then quit by killing **only the PID you started**.
  - Report what you could and could not verify by hand. The tray and the window need the author.
- [ ] **Step 5:** Commit: "Turn the Tauri spike into the desktop app that hosts the server, lives in the tray, can start at login and sends notifications."

---

### Task 8: Settings additions: debug info, delete all data, start at login, update checks

**Files:**
- Create: `ui/src/api/ops.ts`, `ui/src/components/AppSettings.tsx` (+ test)
- Modify: `ui/src/pages/Settings.tsx` (+ test), `ui/src/api/events.ts` (`update.changed` → `["update"]`; `data.reset` → invalidate every query and reset the live store)

**Interfaces:**
- Consumes: `GET /api/debug-info`, `GET/PUT /api/update`, `POST /api/data/reset`, and `GET /api/host` with `PUT /api/host/autostart`.
- Produces:

```ts
export interface HostInfo { kind: "exe" | "desktop"; version: string; autostart: { available: boolean; enabled: boolean | null } }
export interface UpdateStatus { setting: boolean | null; current: string; latest: string | null; newer: boolean; url: string | null; checked_at: number | null }
export function useHost(): UseQueryResult<HostInfo>;
export function useUpdate(): UseQueryResult<UpdateStatus>;
export function useSetUpdateCheck(): (on: boolean) => Promise<UpdateStatus>;
export function useSetAutostart(): (on: boolean) => Promise<HostInfo>;
export async function fetchDebugInfo(): Promise<string>;
export async function resetAllData(): Promise<void>;
```

**The new cards on Settings** go after the existing four, in this order. Each is a `<section aria-label=…>`, and the copy is exact.
1. **"Start with Windows"** (only when `autostart.available`): a checkbox "Start Net Lantern when you sign in to Windows" bound to `enabled`. The note: "It starts in the tray, without opening a window." An error is shown as `role="alert"` with the server message.
2. **"Updates":**
   - A checkbox "Check GitHub for new versions" (unchecked when `setting` is false or null).
   - The note: "Net Lantern asks GitHub for the latest version number once a day. Nothing about your network is sent."
   - The line: "You have version {current}." When `newer`, it adds "Version {latest} is available." and a link "See what's new" to `url` (with `target="_blank"` and `rel="noreferrer"`).
3. **"Help with a problem":**
   - A button "Copy debug info".
   - The note: "Copies your Net Lantern version, Windows version, network state and recent log lines, with addresses and names removed. Nothing is sent anywhere."
   - After a copy: "Copied — paste it into your bug report."
   - If the clipboard fails, show a read-only `<textarea aria-label="Debug info">` with the text, plus "Select the text and copy it."
4. **"Delete all data":**
   - The note: "Removes every network, device, name, history and setting Net Lantern has stored on this computer. Log files are kept."
   - A button "Delete all data…" opens a confirm dialog (`role="alertdialog"`). It says "Delete all Net Lantern data? This can't be undone." and has the buttons "Delete all data" and "Cancel". Focus follows the `ConsentPrompt` pattern.
   - On success: "All data deleted." The app reloads its queries through the `data.reset` event.
   - A failure is shown as `role="alert"` inside the dialog.

- [ ] **Step 1: Write the failing tests** with `mockFetch`:
  - `start_with_windows_card_only_on_desktop_and_toggles`;
  - `update_checkbox_reflects_and_sets_the_setting`;
  - `update_card_shows_newer_version_link_only_for_github`: with a `url` on another host, no link is rendered, as defence in depth;
  - `copy_debug_info_copies_the_server_text`;
  - `copy_falls_back_to_a_textarea_when_the_clipboard_fails`;
  - `delete_all_data_needs_the_confirm_dialog_and_posts_the_exact_confirm_text`;
  - `delete_failure_shows_an_alert_inside_the_dialog`;
  - `data_reset_event_invalidates_queries` (in `events` tests).
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm run lint`, `npx tsc -b`, `npm test` and `npm run build`.
- [ ] **Step 5:** Commit: "Add Settings for start at login, update checks, copying debug info and deleting all data."

---

### Task 9: Dashboard update question and new-version banner

**Files:**
- Create: `ui/src/components/UpdateCard.tsx` (+ test)
- Modify: `ui/src/pages/Dashboard.tsx` (+ test)

**Decisions:**
- **The question:** when `setting === null`, a card at the top of the Dashboard, below the router prompt and first-run card if shown, with exact copy:
  - Heading: "Check for new versions?"
  - Text: "Net Lantern can ask GitHub once a day whether a newer version is out. It sends nothing about your network."
  - Buttons: "Yes, check for updates" and "No thanks". Neither is pre-selected or styled as the default: both use the secondary button style.
  - Answering hides the card. It can be changed in Settings.
- **The banner:** when `newer`, a slim banner at the top of the Dashboard: "Version {latest} is available." with a link "Download from GitHub" to `url` (only if it starts with `https://github.com/`). It can be dismissed for this version in this browser via `localStorage` key `nl_dismissed_update`, wrapped in try/catch.
- **Combined state:** nothing shows while the update query is pending or failed.

- [ ] **Step 1: Write the failing tests:**
  - `question_shows_when_unanswered_and_sets_yes_or_no`;
  - `neither_answer_is_styled_as_the_default`: both buttons have the same class list;
  - `banner_shows_for_a_newer_version_and_links_to_github_only`;
  - `banner_dismissal_is_remembered_for_that_version_only`;
  - `nothing_shows_while_loading_or_on_error`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm run lint`, `npx tsc -b`, `npm test` and `npm run build`.
- [ ] **Step 5:** Commit: "Ask on the Dashboard whether to check for new versions, and show a banner when a newer one is out."

---

### Task 10: End-to-end tests, checkpoint and spec notes

**Files:**
- Modify: `ui/e2e/demo.spec.ts` (at the end of the serial describe), `docs/superpowers/specs/2026-10-31-netlantern-design.md`
- Create: `docs/checkpoints/phase-7a.md`

- [ ] **Step 1: Add e2e tests**, reusing the signed-in page:
  - `update question can be answered and changed in Settings`:
    - the Dashboard shows "Check for new versions?";
    - "No thanks" hides it;
    - in Settings the checkbox is unchecked; checking it shows "You have version 0.1.0." and no "is available" text, because the demo scenario returns the same version.
  - `copy debug info gives redacted text`: in Settings click "Copy debug info". The test grants clipboard permission in its context. Read the clipboard or the fallback textarea: it starts with "Net Lantern", and contains no `192.168.1.` address and no "Living Room".
  - `delete all data clears the demo network`: run this LAST. Open the dialog, confirm, "All data deleted." appears, Devices shows the empty-state text, and the Dashboard shows the first-run card again.
- [ ] **Step 2:** Run all e2e tests (rebuild UI then exe, with `NETLANTERN_EXE` in the same shell call) and make sure they pass.
- [ ] **Step 3: Write the checkpoint** `docs/checkpoints/phase-7a.md`. Use the same shape as `phase-6.md`, with a build block for both the exe and the desktop app (`npm run build`, then `cargo build --release` in `apps/desktop/src-tauri`). Items:
  1. Launch the desktop app: the window opens signed in, and the Dashboard is reachable in under 2 s (spec §2.1). Note the time.
  2. Close the window: the one-time tray notification shows, the app keeps running (monitor ticks continue), and closing again shows no second hint.
  3. Tray: "Open Net Lantern" shows the window. Launching the app again while hidden focuses it, and no second process stays.
  4. Quit from the tray during a quick scan, and again with phone access on: the app exits within about 3 s, and no `netlantern-desktop.exe` remains in Task Manager.
  5. Turn "Start Net Lantern when you sign in to Windows" on, sign out and in: the app starts in the tray with no window. Turn it off again.
  6. Desktop notification: star a device and switch it off. The alert appears as a Windows notification titled "Net Lantern: …".
  7. Footprint: with the window open after 10 minutes of monitoring, Task Manager memory for the app and its WebView2 processes should be under 200 MB in total (spec §2.1). Write the numbers.
  8. Logs: `%LOCALAPPDATA%\NetLantern\logs` has today's `netlantern.log.*`, and it contains no `?t=`.
  9. "Copy debug info": paste it into Notepad and check that no IP (other than 1.1.1.1 and 8.8.8.8), MAC, Wi-Fi name, device name or your Windows username appears.
  10. Update check: answer the Dashboard question. In Settings, turn it on and see "You have version 0.1.0." There is no banner until a newer release exists in 7b.
  11. Location retest (spec §16 risk): does the desktop app show Wi-Fi details where the unpackaged exe needed both location switches? The answer is expected to be the same until the 7b installer gives a package identity. Record it.
  12. "Delete all data" on a throwaway copy: back up `%LOCALAPPDATA%\NetLantern\netlantern.db` first, or run with `--data-dir`. Confirm the app starts fresh, then restore the backup.
  13. The learning check (spec §13, phase 7): explain how Tauri hosts the server in-process (one process, the webview talks HTTP to 127.0.0.1); the tray lifecycle (close hides, Quit stops); and why the release in 7b is unsigned and what SmartScreen will say.

  End with a "What to look at" note: the shared data folder between exe and app, and that the exe still uses PowerShell toasts.
- [ ] **Step 4: Update the spec.**
  - §4: a phase-7a note: the shared host module, start at login launching hidden in the tray, the one-time tray hint, and the shared data folder.
  - §6.9: the desktop app uses the Tauri notification plugin, and the exe keeps route B.
  - §6.12: add rows, each marked *(changed in phase 7)*:
    - update check: at start and every 24 h when on;
    - log files: daily, kept 7 days;
    - debug info: last 200 log lines, last 50 problems;
    - reset scan-cancel wait: 5 s.
  - §8: `GET /api/debug-info`, `GET/PUT /api/update`, `POST /api/data/reset`, `GET /api/host`, `PUT /api/host/autostart`; events `update.changed` and `data.reset`; errors `autostart_unavailable` (409) and `autostart_failed` (500).
  - §10.1: the update check details from decision 4, the debug info redaction rules from decision 6, and that the reset keeps log files.
  - §12: a note that Phase 7 is split into 7a (desktop app and in-app operations), 7b (installer, release, README) and 7c (landing site), each with its own plan.
- [ ] **Step 5: Final verification.** Root workspace fmt, clippy and test; the desktop crate's fmt, clippy, test and release build; UI lint, tsc, test, build and e2e. All must be clean.
- [ ] **Step 6:** Commit: "Add end-to-end tests for updates, debug info and data reset, the Phase 7a checkpoint, and the desktop app notes in the spec."
