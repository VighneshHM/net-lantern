//! Net Lantern's desktop app. It hosts the server in-process, shows the
//! dashboard in its own window, lives in the tray, can start at login and
//! sends notifications. The window talks to the server over plain HTTP on
//! 127.0.0.1, so there is no JavaScript bridge.
//!
//! The login URL carries a one-time token: it goes to the webview only and is
//! never logged.
//!
//! Arguments:
//! - `--hidden` (what a start at login passes): no window is shown until it is
//!   opened from the tray or by a second launch.
//! - `--data-dir <folder>` (or the `NETLANTERN_DATA_DIR` setting): keep the
//!   data and the logs in `<folder>` instead of `%LOCALAPPDATA%\NetLantern`.
//!   For tests; a start at login does not carry it.
//!
//! - `--demo` / `--demo=<scenario file>`: run against a simulated network
//!   (the built-in home one, or the file) with the data in memory, no log
//!   files and `--data-dir` ignored. A bare `--demo` never takes the next
//!   argument as the file. For trying the app without a real network.
//!
//! `NETLANTERN_PIPE_NAME` overrides the single-instance pipe, also for tests.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod notify;
mod tray;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use netlantern::host::{self, Backend, Host, HostOptions, Started};
use netlantern::logging::{self, LogOptions};
use netlantern_core::{Store, RELEASES_PAGE_PREFIX};
use netlantern_server::{Auth, HostKind, LogTaps, SESSION_COOKIE};
use tauri::webview::NewWindowResponse;
use tauri::{
    App, AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent,
};
use tokio::sync::oneshot;

use crate::autostart::DesktopAutostart;
use crate::notify::TauriNotifier;

/// The dashboard window's label.
const MAIN: &str = "main";
/// The global setting that records the tray hint was shown once.
const TRAY_HINT_KEY: &str = "tray_hint_shown";
const TRAY_HINT: &str = "Net Lantern is still running in the tray. Quit it from the tray menu.";

/// The one link that leaves the app: the dashboard's location-permission
/// link. Only this constant is ever handed to Windows.
const LOCATION_SETTINGS: &str = "ms-settings:privacy-location";

const STOPPED_TITLE: &str = "Net Lantern stopped";
const STOPPED_BODY: &str = "Net Lantern stopped unexpectedly. Start it again to keep monitoring.";

/// How long the way out (Quit, the app exiting, a failed start) waits for the
/// server to stop before the process ends anyway. Everything runs in this
/// process, so its end releases the pipe, the port and the database.
const STOP_LIMIT: Duration = Duration::from_secs(3);
/// After asking Tauri to exit, how long before the process ends regardless.
const EXIT_BACKSTOP: Duration = Duration::from_secs(2);

/// Set once the app is on its way out (Quit, a stopped server, or a failed
/// start): from then on an exit request is let through.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// How a bounded wait ended.
#[derive(Debug, PartialEq, Eq)]
enum Wait {
    /// Done, or the other side is gone (which also means it finished).
    Done,
    TimedOut,
}

/// Waits at most `limit` for `done`.
fn wait_bounded(done: &Receiver<()>, limit: Duration) -> Wait {
    match done.recv_timeout(limit) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => Wait::Done,
        Err(RecvTimeoutError::Timeout) => Wait::TimedOut,
    }
}

/// What the webview may do with a URL it is about to load.
#[derive(Debug, PartialEq, Eq)]
enum Nav {
    /// The dashboard on this run's port.
    Allow,
    /// Exactly [`LOCATION_SETTINGS`]: Windows opens its settings page,
    /// outside the app.
    Outside,
    /// A Net Lantern release page: opened in the default browser.
    Browser,
    Deny,
}

/// A release page of Net Lantern on GitHub, judged on the parsed URL: https,
/// no user info, no explicit port, under [`RELEASES_PAGE_PREFIX`] (trailing
/// slash included).
fn release_page(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("github.com")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.as_str().starts_with(RELEASES_PAGE_PREFIX)
}

/// Only the dashboard (`http://127.0.0.1:<port>` or `http://localhost:<port>`)
/// loads in the window, and only the exact location-settings link leaves it.
/// Net Lantern's release pages open in the browser. Every other URL, any
/// other `ms-settings:` or GitHub one included, is refused and never handed
/// to Windows.
fn navigation(url: &Url, port: u16) -> Nav {
    if url.as_str() == LOCATION_SETTINGS {
        return Nav::Outside;
    }
    if release_page(url) {
        return Nav::Browser;
    }
    match url.scheme() {
        "http"
            if matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
                && url.port() == Some(port) =>
        {
            Nav::Allow
        }
        _ => Nav::Deny,
    }
}

/// What the command line asks for.
struct Args {
    hidden: bool,
    data_dir: Option<PathBuf>,
    /// `Some(None)`: the built-in demo network; `Some(Some(file))`: a scenario.
    demo: Option<Option<PathBuf>>,
}

/// Reads `--hidden`, `--data-dir <folder>` / `--data-dir=<folder>` and
/// `--demo` / `--demo=<file>` (an empty file name means the built-in demo);
/// anything else is ignored.
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Args {
    let mut parsed = Args {
        hidden: false,
        data_dir: None,
        demo: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg.as_os_str() == "--hidden" {
            parsed.hidden = true;
        } else if arg.as_os_str() == "--demo" {
            parsed.demo = Some(None);
        } else if let Some(file) = arg.to_str().and_then(|a| a.strip_prefix("--demo=")) {
            parsed.demo = Some(Some(file).filter(|f| !f.is_empty()).map(PathBuf::from));
        } else if arg.as_os_str() == "--data-dir" {
            parsed.data_dir = args.next().filter(|v| !v.is_empty()).map(PathBuf::from);
        } else if let Some(dir) = arg.to_str().and_then(|a| a.strip_prefix("--data-dir=")) {
            parsed.data_dir = Some(dir).filter(|d| !d.is_empty()).map(PathBuf::from);
        }
    }
    parsed
}

/// Shown when a launch with options finds Net Lantern already running.
const OPTIONS_IGNORED: &str = "Net Lantern is already running, so your options were ignored. Quit it from the tray first, then start it again with those options.";

/// True when the command line carries `--data-dir` or `--demo` (any form), or
/// `NETLANTERN_DATA_DIR` is set and not empty: options a running instance
/// cannot take, so a second launch would silently drop them.
fn ignored_options(args: &[OsString], env_data_dir: Option<OsString>) -> bool {
    let flag = |a: &OsString| {
        a.to_str().is_some_and(|a| {
            a == "--data-dir"
                || a == "--demo"
                || a.starts_with("--data-dir=")
                || a.starts_with("--demo=")
        })
    };
    args.iter().any(flag) || env_data_dir.is_some_and(|v| !v.is_empty())
}

/// The data folder: the `--data-dir` argument as given, else
/// `NETLANTERN_DATA_DIR` when it is an absolute folder `usable` accepts.
/// `None` means the default `%LOCALAPPDATA%\NetLantern`. The flag is true
/// when the setting was there but ignored (empty, relative, or unusable).
fn data_dir_choice(
    arg: Option<PathBuf>,
    env: Option<OsString>,
    usable: impl Fn(&Path) -> bool,
) -> (Option<PathBuf>, bool) {
    if arg.is_some() {
        return (arg, false);
    }
    match env.map(PathBuf::from) {
        None => (None, false),
        Some(dir) if dir.is_absolute() && usable(&dir) => (Some(dir), false),
        Some(_) => (None, true),
    }
}

/// Where the window should go when it is opened: `None` to stay on its page
/// (it is still signed in), else the login URL. Never logs the URL.
fn sign_in_target(signed_in: bool, url: &str) -> Option<Url> {
    if signed_in {
        return None;
    }
    match url.parse() {
        Ok(url) => Some(url),
        Err(_) => {
            tracing::warn!("the login address is not valid; the window stays on its page");
            None
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The single-instance pipe for this user (`NETLANTERN_PIPE_NAME` overrides it).
#[cfg(windows)]
fn pipe_name() -> String {
    netlantern::instance::pipe_name()
}

#[cfg(not(windows))]
fn pipe_name() -> String {
    String::new()
}

/// The server's thread and the way to stop it. Dropping it stops the server
/// and waits until `serve_until` is done: phone access, the monitor and the
/// server have stopped and the pipe is let go.
struct Server {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        // A dropped sender ends the wait just as a sent one does.
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::error!("the server thread panicked");
            }
        }
    }
}

/// What a started server hands back to the app.
struct Ready {
    login_url: String,
    port: u16,
    store: Store,
    auth: Arc<Auth>,
    runtime: tokio::runtime::Handle,
}

enum Outcome {
    Running(Ready),
    /// Another Net Lantern runs and was asked to open.
    AlreadyRunning,
    Failed(String),
}

/// What the window, tray and exit handling share, managed by Tauri.
struct Shared {
    /// Taken (and so stopped) on the way out.
    server: Mutex<Option<Server>>,
    runtime: tokio::runtime::Handle,
    store: Store,
    auth: Arc<Auth>,
    port: u16,
    notifier: Arc<TauriNotifier>,
    /// Without a tray icon, closing the window quits.
    has_tray: bool,
    hint_sent: AtomicBool,
}

fn main() {
    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let options_given = ignored_options(&raw_args, std::env::var_os("NETLANTERN_DATA_DIR"));
    let args = parse_args(raw_args);
    let (data_dir, env_ignored) = data_dir_choice(
        args.data_dir,
        std::env::var_os("NETLANTERN_DATA_DIR"),
        |dir| std::fs::create_dir_all(dir).is_ok(),
    );
    // Demo mode keeps no files, logs included.
    let log_dir = if args.demo.is_some() {
        None
    } else {
        host::data_dir(
            data_dir.as_deref(),
            std::env::var("LOCALAPPDATA").ok().as_deref(),
        )
        .ok()
        .map(|dir| dir.join("logs"))
    };
    // Held for the life of the process.
    let _log_guard = logging::init(LogOptions {
        dir: log_dir,
        stderr: cfg!(debug_assertions),
    });
    logging::install_panic_hook();
    if env_ignored {
        // The value is not logged: it is the user's own setting.
        tracing::warn!(
            "NETLANTERN_DATA_DIR is ignored: it must be an absolute folder that can be created"
        );
    }

    if args.demo.is_some() && data_dir.is_some() {
        tracing::warn!("--data-dir is ignored in demo mode; demo data stays in memory");
    }
    let hidden = args.hidden;
    let demo = args.demo;
    let built = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            start(app, data_dir, demo, hidden, options_given);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == MAIN {
                    api.prevent_close();
                    on_close(window);
                }
            }
        })
        .build(tauri::generate_context!());
    let app = match built {
        Ok(app) => app,
        Err(e) => {
            tracing::error!("Net Lantern could not start its window system: {e}");
            std::process::exit(1);
        }
    };

    app.run(|handle, event| match event {
        // Closing the last window, for one: keep running in the tray.
        RunEvent::ExitRequested { api, .. } if !QUITTING.load(Ordering::SeqCst) => {
            api.prevent_exit();
        }
        RunEvent::Exit => {
            QUITTING.store(true, Ordering::SeqCst);
            // Quit took it already; any other way out stops it here.
            if let Some(shared) = handle.try_state::<Shared>() {
                stop_server(lock(&shared.server).take());
            }
        }
        _ => {}
    });
}

/// The start-at-login switch, with an existing entry pointed at this exe
/// first. Failures are logged without the path, which holds the user name.
fn start_at_login() -> Option<DesktopAutostart> {
    let autostart = match DesktopAutostart::for_current_exe() {
        Ok(autostart) => autostart,
        Err(e) => {
            tracing::warn!(kind = ?e.kind(), "cannot find this exe, so start at login is unavailable");
            return None;
        }
    };
    if let Err(e) = autostart.repair() {
        tracing::warn!(kind = ?e.kind(), "cannot update the start-at-login entry: {e}");
    }
    Some(autostart)
}

/// Starts the server, then adds the tray icon and the window. Runs once, on
/// the main thread, when the event loop is ready.
fn start(
    app: &mut App,
    data_dir: Option<PathBuf>,
    demo: Option<Option<PathBuf>>,
    hidden: bool,
    options_given: bool,
) {
    let handle = app.handle().clone();
    let notifier = Arc::new(TauriNotifier::new(handle.clone()));
    let on_open = {
        let handle = handle.clone();
        // Runs on the pipe task: hand off and return at once.
        Arc::new(move |url: String| open_window(handle.clone(), Some(url)))
    };
    let autostart = start_at_login();
    let opts = HostOptions {
        backend: match demo {
            Some(scenario) => Backend::Demo {
                scenario,
                time_scale: None,
            },
            None => Backend::Real { data_dir },
        },
        port: None,
        dev_origins: Vec::new(),
        notifier: Arc::clone(&notifier) as _,
        autostart: autostart.map(|a| Arc::new(a) as _),
        host_kind: HostKind::Desktop,
        logs: Some(LogTaps {
            lines: Arc::new(logging::recent_lines),
            problems: Arc::new(logging::recent_problems),
        }),
        on_open,
        pipe_name: pipe_name(),
    };

    let (outcome, server) = start_server(opts, handle.clone(), Arc::clone(&notifier));
    let ready = match outcome {
        Outcome::Running(ready) => ready,
        Outcome::AlreadyRunning => {
            // The running one opens its window; this one never makes one.
            drop(server);
            tracing::info!("Net Lantern is already running; asked it to open");
            if options_given {
                message_box("Net Lantern is already running", OPTIONS_IGNORED);
            }
            std::process::exit(0);
        }
        Outcome::Failed(reason) => {
            drop(server);
            fatal(&handle, &notifier, &reason);
        }
    };

    let has_tray = match tray::build(app) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("cannot add the tray icon, so closing the window quits: {e}");
            false
        }
    };
    let login_url = ready.login_url;
    let port = ready.port;
    app.manage(Shared {
        server: Mutex::new(server),
        runtime: ready.runtime,
        store: ready.store,
        auth: ready.auth,
        port: ready.port,
        notifier: Arc::clone(&notifier),
        has_tray,
        hint_sent: AtomicBool::new(false),
    });

    // Created even when hidden, already on the login URL, so the first show is
    // signed in.
    let Some(url) = sign_in_target(false, &login_url) else {
        fatal(&handle, &notifier, "The dashboard address is not valid.");
    };
    let window = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
        .title("Net Lantern")
        .inner_size(1200.0, 800.0)
        .visible(!hidden)
        .on_navigation(move |url| match navigation(url, port) {
            Nav::Allow => true,
            Nav::Outside => {
                open_location_settings();
                false
            }
            Nav::Browser => {
                open_in_browser(url.clone());
                false
            }
            Nav::Deny => {
                // The URL could be anything; the scheme is enough to tell.
                tracing::warn!(
                    scheme = url.scheme(),
                    "the window refused to leave the dashboard"
                );
                false
            }
        })
        // Never another app window: the location link and release pages go
        // outside, anything else is refused.
        .on_new_window(|url, _| {
            match navigation(&url, 0) {
                Nav::Outside => open_location_settings(),
                Nav::Browser => open_in_browser(url),
                Nav::Allow | Nav::Deny => {
                    tracing::warn!(scheme = url.scheme(), "the window refused to open another");
                }
            }
            NewWindowResponse::Deny
        })
        .build();
    if let Err(e) = window {
        // The error can carry the window's URL, and so the token: only its kind.
        tracing::error!(kind = error_kind(&e), "the window could not be opened");
        fatal(
            &handle,
            &notifier,
            "The Net Lantern window could not be opened.",
        );
    }
    tracing::info!(hidden, "desktop app ready");
}

/// Starts the host on a Tokio runtime of its own thread and waits until it
/// is serving, has found another Net Lantern, or has failed.
fn start_server(
    opts: HostOptions,
    handle: AppHandle,
    notifier: Arc<TauriNotifier>,
) -> (Outcome, Option<Server>) {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("server".into())
        .spawn(move || serve(opts, &ready_tx, stop_rx, handle, notifier));
    let thread = match spawned {
        Ok(thread) => thread,
        Err(e) => {
            return (
                Outcome::Failed(format!("Could not start the server: {e}")),
                None,
            )
        }
    };
    let outcome = ready_rx
        .recv()
        .unwrap_or_else(|_| Outcome::Failed("The server stopped while starting.".to_string()));
    let server = Server {
        stop: Some(stop_tx),
        thread: Some(thread),
    };
    (outcome, Some(server))
}

/// The server thread: starts the host and serves until stopped. Every path
/// past a successful start goes through `serve_until`, so the pipe, phone
/// access and the monitor never outlive the server.
fn serve(
    opts: HostOptions,
    ready: &Sender<Outcome>,
    stop: oneshot::Receiver<()>,
    handle: AppHandle,
    notifier: Arc<TauriNotifier>,
) {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = ready.send(Outcome::Failed(format!("Could not start the server: {e}")));
            return;
        }
    };
    runtime.block_on(async move {
        let host = match Host::start(opts).await {
            Ok(Started::Running(host)) => host,
            Ok(Started::AlreadyRunning) => {
                let _ = ready.send(Outcome::AlreadyRunning);
                return;
            }
            Err(e) => {
                let _ = ready.send(Outcome::Failed(e.to_string()));
                return;
            }
        };
        let up = Ready {
            login_url: host.login_url(),
            port: host.port(),
            store: host.state().store.clone(),
            auth: Arc::clone(&host.state().auth),
            runtime: tokio::runtime::Handle::current(),
        };
        // Nobody waiting any more means stop at once, still through serve_until.
        let waited = ready.send(Outcome::Running(up)).is_ok();
        // From here, however this ends (even by a panic), the app ends too.
        let _end = EndWithServer { handle, notifier };
        let result = host
            .serve_until(async move {
                if waited {
                    let _ = stop.await;
                }
            })
            .await;
        if let Err(e) = &result {
            tracing::error!("Net Lantern stopped unexpectedly: {e}");
        }
    });
}

/// Held by the server thread once the server runs. When the thread ends (or
/// panics) without a Quit, the window has nothing left to show: says so and
/// ends the app.
struct EndWithServer {
    handle: AppHandle,
    notifier: Arc<TauriNotifier>,
}

impl Drop for EndWithServer {
    fn drop(&mut self) {
        if QUITTING.swap(true, Ordering::SeqCst) {
            return;
        }
        if std::thread::panicking() {
            tracing::error!("the server thread panicked");
        }
        tracing::error!("the server stopped without a Quit; exiting");
        self.notifier.show(STOPPED_TITLE, STOPPED_BODY);
        // The plugin shows it on another thread.
        std::thread::sleep(Duration::from_secs(1));
        self.handle.exit(1);
    }
}

/// Stops the server on a thread of its own and waits at most [`STOP_LIMIT`].
fn stop_server(server: Option<Server>) {
    let Some(server) = server else {
        return;
    };
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("stop".into())
        .spawn(move || {
            drop(server);
            let _ = done_tx.send(());
        });
    if let Err(e) = spawned {
        // The closure, and so the server, was dropped (and stopped) already.
        tracing::warn!("cannot stop the server on a thread of its own: {e}");
        return;
    }
    if wait_bounded(&done_rx, STOP_LIMIT) == Wait::TimedOut {
        tracing::warn!(
            "the server did not stop within {} s; exiting anyway",
            STOP_LIMIT.as_secs()
        );
    }
}

/// Logs why the app cannot start, stops the server if it runs, says so in a
/// notification and a message box, and exits with 1.
fn fatal(handle: &AppHandle, notifier: &TauriNotifier, reason: &str) -> ! {
    const TITLE: &str = "Net Lantern could not start";
    QUITTING.store(true, Ordering::SeqCst);
    tracing::error!("Net Lantern could not start: {reason}");
    if let Some(shared) = handle.try_state::<Shared>() {
        stop_server(lock(&shared.server).take());
    }
    // A notification can fail silently (the plugin drops the error), so a
    // message box too; it also gives the notification time to show.
    remove_tray(handle);
    notifier.show(TITLE, reason);
    message_box(TITLE, reason);
    std::process::exit(1);
}

/// A blocking error box. Plain `MessageBoxW` from user32, which every
/// Windows desktop process has; no extra dependency.
#[cfg(windows)]
fn message_box(title: &str, text: &str) {
    use std::ffi::c_void;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide(title));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call;
    // a null owner window is allowed.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn message_box(_title: &str, _text: &str) {
    // Give the notification time to show.
    std::thread::sleep(Duration::from_secs(3));
}

/// Opens Windows' location settings. Only the constant goes to Windows, never
/// a URL from the page.
fn open_location_settings() {
    #[cfg(windows)]
    if let Err(e) = std::process::Command::new("explorer.exe")
        .arg(LOCATION_SETTINGS)
        .spawn()
    {
        tracing::warn!("cannot open the Windows settings page: {e}");
    }
}

/// Opens a release page (already checked by [`release_page`]) in the
/// default browser, on a thread of its own so the caller never waits.
/// Errors are logged without the URL.
fn open_in_browser(url: Url) {
    let spawned = std::thread::Builder::new()
        .name("browser".into())
        .spawn(move || {
            if let Err(code) = shell_open(url.as_str()) {
                tracing::warn!("cannot open the release page in the browser (code {code})");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("cannot open the release page in the browser: {e}");
    }
}

/// `ShellExecuteW(NULL, "open", target, NULL, NULL, SW_SHOWNORMAL)`: Windows
/// opens the https URL with the default browser. The URL is the single
/// `lpFile` value, so no command line is built from it. (The `webbrowser`
/// crate was not used: on Windows it reads the browser's registered command
/// line and runs that with the URL spliced in.) The error is the
/// `ShellExecuteW` return code.
#[cfg(windows)]
fn shell_open(target: &str) -> Result<(), isize> {
    use std::ffi::c_void;

    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> *mut c_void;
    }
    const SW_SHOWNORMAL: i32 = 1;
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (operation, file) = (wide("open"), wide(target));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call;
    // null owner, parameters and directory are allowed.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Success is a value greater than 32.
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(code)
    }
}

#[cfg(not(windows))]
fn shell_open(_target: &str) -> Result<(), isize> {
    Err(0)
}

/// Takes the tray icon away before the process ends without Tauri's own
/// cleanup, so no dead icon stays in the notification area. Tauri removes it
/// on the main thread, so this gives it a moment.
fn remove_tray(handle: &AppHandle) {
    if let Some(tray) = handle.remove_tray_by_id(tray::ID) {
        drop(tray);
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// What kind of error a window build failed with; its text can hold the URL.
fn error_kind(e: &tauri::Error) -> &'static str {
    match e {
        tauri::Error::Runtime(_) => "runtime",
        tauri::Error::WindowLabelAlreadyExists(_) | tauri::Error::WebviewLabelAlreadyExists(_) => {
            "label in use"
        }
        _ => "other",
    }
}

/// Shows, unminimises and focuses the window, first signing it in again with
/// `fresh_url` (or a new login URL) if it lost its session. Returns at once:
/// the session check reads the webview's cookies, which must not happen on
/// the main thread, so it runs on a thread of its own and then hands the
/// window to the main thread.
fn open_window(handle: AppHandle, fresh_url: Option<String>) {
    let spawned = std::thread::Builder::new()
        .name("open".into())
        .spawn(move || {
            let signed_in = has_session(&handle);
            let url = if signed_in {
                None
            } else {
                fresh_url.or_else(|| {
                    handle
                        .try_state::<Shared>()
                        .map(|shared| host::login_url(&shared.auth, shared.port))
                })
            };
            let target = url.and_then(|url| sign_in_target(signed_in, &url));
            let main = handle.clone();
            if let Err(e) = handle.run_on_main_thread(move || show_window(&main, target)) {
                tracing::warn!("cannot open the window: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("cannot open the window: {e}");
    }
}

/// Whether the window holds a session the server still knows. Must not run
/// on the main thread.
fn has_session(handle: &AppHandle) -> bool {
    let (Some(shared), Some(window)) = (
        handle.try_state::<Shared>(),
        handle.get_webview_window(MAIN),
    ) else {
        return false;
    };
    let Ok(base) = Url::parse(&format!("http://127.0.0.1:{}/", shared.port)) else {
        return false;
    };
    match window.cookies_for_url(base) {
        Ok(cookies) => cookies
            .iter()
            .any(|c| c.name() == SESSION_COOKIE && shared.auth.is_session(c.value())),
        Err(e) => {
            tracing::warn!("cannot read the window's session, so signing it in again: {e}");
            false
        }
    }
}

/// On the main thread: navigates to `target` if given, then shows,
/// unminimises and focuses the window.
fn show_window(handle: &AppHandle, target: Option<Url>) {
    // An Open that races a Quit must not bring the window back.
    if QUITTING.load(Ordering::SeqCst) {
        return;
    }
    let Some(window) = handle.get_webview_window(MAIN) else {
        return;
    };
    if let Some(url) = target {
        // The error could hold the URL: say only that it failed.
        if window.navigate(url).is_err() {
            tracing::warn!("cannot sign the window in again");
        }
    }
    let shown = window
        .show()
        .and_then(|()| window.unminimize())
        .and_then(|()| window.set_focus());
    if let Err(e) = shown {
        tracing::warn!("cannot show the window: {e}");
    }
}

/// The window's close button hides it. The first time ever, a notification
/// says the app is still in the tray. Without a tray icon it quits instead.
fn on_close(window: &Window) {
    if let Err(e) = window.hide() {
        tracing::warn!("cannot hide the window: {e}");
    }
    let handle = window.app_handle();
    let Some(shared) = handle.try_state::<Shared>() else {
        return;
    };
    if !shared.has_tray {
        quit(handle);
        return;
    }
    if shared.hint_sent.swap(true, Ordering::SeqCst) {
        return;
    }
    let store = shared.store.clone();
    let notifier = Arc::clone(&shared.notifier);
    shared.runtime.spawn(async move {
        match store.global_setting(TRAY_HINT_KEY).await {
            Ok(Some(value)) if value == "on" => return,
            Ok(_) => {}
            Err(e) => tracing::warn!("cannot read whether the tray hint was shown: {e}"),
        }
        if let Err(e) = store.set_global_setting(TRAY_HINT_KEY, "on").await {
            tracing::warn!("cannot remember that the tray hint was shown: {e}");
        }
        notifier.show("Net Lantern", TRAY_HINT);
    });
}

/// Quit from the tray: hides the window, stops the server and waits for it
/// (phone access, the monitor, the server) off the main thread for at most
/// [`STOP_LIMIT`], then exits. If Tauri's exit stalls, the process ends after
/// [`EXIT_BACKSTOP`] anyway.
pub fn quit(handle: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    tracing::info!("quitting");
    if let Some(window) = handle.get_webview_window(MAIN) {
        let _ = window.hide();
    }
    let server = handle
        .try_state::<Shared>()
        .and_then(|shared| lock(&shared.server).take());
    let exit = handle.clone();
    let spawned = std::thread::Builder::new()
        .name("quit".into())
        .spawn(move || {
            stop_server(server);
            exit.exit(0);
            std::thread::sleep(EXIT_BACKSTOP);
            // Log files are written synchronously: nothing to flush.
            tracing::warn!("the app did not exit in time; ending the process");
            remove_tray(&exit);
            std::process::exit(0);
        });
    if spawned.is_err() {
        // The closure, and so the server, was dropped (and stopped) already.
        handle.exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn args_demo_forms() {
        assert_eq!(parse_args(os(&[])).demo, None);
        assert_eq!(parse_args(os(&["--demo"])).demo, Some(None));
        assert_eq!(
            parse_args(os(&["--demo=s/x.json"])).demo,
            Some(Some(PathBuf::from("s/x.json")))
        );
        assert_eq!(parse_args(os(&["--demo="])).demo, Some(None));
        // A bare --demo does not take the next argument as its file, and the
        // other flags still work next to it.
        let both = parse_args(os(&["--demo", "x.json", "--hidden", "--data-dir", "C:/nl"]));
        assert_eq!(both.demo, Some(None));
        assert!(both.hidden);
        assert_eq!(both.data_dir, Some(PathBuf::from("C:/nl")));
        assert_eq!(parse_args(os(&["--demox"])).demo, None);
    }

    #[test]
    fn args_hidden_detects_flag() {
        assert!(parse_args(os(&["--hidden"])).hidden);
        assert!(parse_args(os(&["--data-dir", "x", "--hidden"])).hidden);
        assert!(!parse_args(os(&[])).hidden);
        assert!(!parse_args(os(&["--hiddenx", "hidden"])).hidden);
    }

    #[test]
    fn args_read_the_data_dir_in_both_spellings() {
        assert_eq!(
            parse_args(os(&["--data-dir", "C:/nl"])).data_dir,
            Some(PathBuf::from("C:/nl"))
        );
        assert_eq!(
            parse_args(os(&["--data-dir=C:/nl", "--hidden"])).data_dir,
            Some(PathBuf::from("C:/nl"))
        );
        assert_eq!(parse_args(os(&["--data-dir"])).data_dir, None);
        assert_eq!(parse_args(os(&["--data-dir="])).data_dir, None);
    }

    #[test]
    fn options_a_running_instance_cannot_take_are_noticed() {
        for given in [
            &["--data-dir", "C:/nl"][..],
            &["--data-dir=C:/nl"],
            &["--demo"],
            &["--demo=s.json"],
            &["--hidden", "--demo"],
        ] {
            assert!(ignored_options(&os(given), None), "{given:?}");
        }
        assert!(ignored_options(&os(&[]), Some("C:/nl".into())));
        assert!(!ignored_options(&os(&[]), None));
        assert!(!ignored_options(&os(&["--hidden"]), None));
        assert!(!ignored_options(&os(&["--demox"]), Some(OsString::new())));
    }

    fn yes(_: &Path) -> bool {
        true
    }

    #[test]
    fn the_data_dir_arg_wins_over_the_environment() {
        let abs = std::env::temp_dir().join("nl-env");
        // The argument is taken as given, even relative.
        assert_eq!(
            data_dir_choice(Some("arg".into()), Some(abs.clone().into()), yes),
            (Some(PathBuf::from("arg")), false)
        );
        assert_eq!(
            data_dir_choice(None, Some(abs.clone().into()), yes),
            (Some(abs), false)
        );
        assert_eq!(data_dir_choice(None, None, yes), (None, false));
    }

    #[test]
    fn the_data_dir_setting_must_be_an_absolute_usable_folder() {
        let abs = std::env::temp_dir().join("nl-env");
        assert_eq!(
            data_dir_choice(None, Some(OsString::new()), yes),
            (None, true)
        );
        assert_eq!(
            data_dir_choice(None, Some("relative\\data".into()), yes),
            (None, true)
        );
        assert_eq!(
            data_dir_choice(None, Some(abs.into()), |_| false),
            (None, true)
        );
    }

    #[test]
    fn a_bounded_wait_gives_up_on_a_thread_that_never_finishes() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        // Holds the sender forever without sending.
        std::thread::spawn(move || {
            let _tx = tx;
            loop {
                std::thread::park();
            }
        });
        let started = std::time::Instant::now();
        assert_eq!(
            wait_bounded(&rx, Duration::from_millis(100)),
            Wait::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_bounded_wait_sees_a_finished_or_gone_thread() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(()).unwrap())
            .join()
            .unwrap();
        assert_eq!(wait_bounded(&rx, Duration::from_secs(5)), Wait::Done);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        drop(tx);
        assert_eq!(wait_bounded(&rx, Duration::from_secs(5)), Wait::Done);
    }

    #[test]
    fn the_window_only_loads_the_dashboard() {
        let nav = |s: &str| navigation(&s.parse().unwrap(), 47800);
        assert_eq!(nav("http://127.0.0.1:47800/"), Nav::Allow);
        assert_eq!(nav("http://127.0.0.1:47800/devices?x=1"), Nav::Allow);
        assert_eq!(nav("http://localhost:47800/"), Nav::Allow);
        assert_eq!(nav("http://127.0.0.1:47801/"), Nav::Deny);
        assert_eq!(nav("http://127.0.0.1/"), Nav::Deny);
        assert_eq!(nav("https://127.0.0.1:47800/"), Nav::Deny);
        assert_eq!(nav("http://example.com:47800/"), Nav::Deny);
        assert_eq!(nav("http://127.0.0.1.example.com:47800/"), Nav::Deny);
        assert_eq!(nav("https://github.com/"), Nav::Deny);
        assert_eq!(nav("file:///C:/Windows/win.ini"), Nav::Deny);
        assert_eq!(nav("about:blank"), Nav::Deny);
        assert_eq!(nav("ms-settings:privacy-location"), Nav::Outside);
    }

    /// `None` when the string is not a URL at all (the webview never hands
    /// those over), else the verdict.
    fn verdict(s: &str) -> Option<Nav> {
        s.parse::<Url>().ok().map(|url| navigation(&url, 47800))
    }

    #[test]
    fn only_the_exact_location_settings_link_leaves_the_app() {
        assert_eq!(verdict(LOCATION_SETTINGS), Some(Nav::Outside));
        for s in [
            "ms-settings:privacy-location%20x",
            "ms-settings:privacy-location x",
            "ms-settings:privacy-location\" C:\\Windows\\System32\\calc.exe",
            "ms-settings:privacy-location\" /select,C:\\",
            "ms-settings:privacy-locationx",
            "ms-settings:privacy-location/x",
            "ms-settings:privacy-location?x",
            "ms-settings:privacy-location#x",
            "ms-settings:PRIVACY-LOCATION",
            "ms-settings:",
            "ms-settings:privacy-camera",
            "ms-settings:privacy-location:x",
            "file:///C:/Windows/System32/cmd.exe",
            "file://server/share/x.exe",
            "cmd:/c calc",
            "shell:startup",
            "search-ms:query=x",
        ] {
            assert!(
                matches!(verdict(s), None | Some(Nav::Deny)),
                "{s:?} -> {:?}",
                verdict(s)
            );
        }
        // Not URLs at all.
        assert_eq!(verdict("\\\\server\\share\\x.exe"), None);
        assert_eq!(verdict("cmd /c calc"), None);
    }

    fn release(s: &str) -> bool {
        release_page(&s.parse().unwrap())
    }

    #[test]
    fn release_pages_are_the_only_github_links_that_leave_the_app() {
        // The prefix itself and pages under it.
        assert!(release(RELEASES_PAGE_PREFIX));
        assert!(release(
            "https://github.com/VighneshHM/net-lantern/releases/tag/v0.2.0"
        ));
        assert!(release(
            "https://github.com/VighneshHM/net-lantern/releases/latest?x=1#notes"
        ));
        // The parser lowercases the host and drops the default port; what is
        // checked (and opened) is the parsed string.
        assert!(release(
            "https://GITHUB.COM/VighneshHM/net-lantern/releases/tag/v1"
        ));
        assert!(release(
            "https://github.com:443/VighneshHM/net-lantern/releases/tag/v1"
        ));
        for s in [
            // No trailing slash: the prefix requires it.
            "https://github.com/VighneshHM/net-lantern/releases",
            "https://github.com/VighneshHM/net-lantern/issues",
            "https://github.com/VighneshHM/net-lantern-evil/releases/x",
            "https://github.com/VighneshHM/net-lanternx/releases/x",
            "https://github.com/vighneshhm/net-lantern/releases/x",
            "http://github.com/VighneshHM/net-lantern/releases/x",
            "https://github.com.evil.com/VighneshHM/net-lantern/releases/x",
            "https://github.com@evil.com/VighneshHM/net-lantern/releases/x",
            "https://user@github.com/VighneshHM/net-lantern/releases/x",
            "https://user:pw@github.com/VighneshHM/net-lantern/releases/x",
            "https://github.com:8443/VighneshHM/net-lantern/releases/x",
            "https://evil.com/https://github.com/VighneshHM/net-lantern/releases/x",
            "file:///C:/x",
            "ms-settings:privacy-location",
        ] {
            assert!(!release(s), "{s}");
        }
        // `..` is resolved by the parser before the check.
        let tricky: Url = "https://github.com/VighneshHM/net-lantern/releases/../../x"
            .parse()
            .unwrap();
        assert_eq!(tricky.as_str(), "https://github.com/VighneshHM/x");
        assert!(!release_page(&tricky));
        let encoded: Url = "https://github.com/VighneshHM/net-lantern/releases/%2e%2e/%2e%2e/x"
            .parse()
            .unwrap();
        assert_eq!(encoded.as_str(), "https://github.com/VighneshHM/x");
        assert!(!release_page(&encoded));
    }

    #[test]
    fn release_pages_go_to_the_browser_and_other_links_stay_refused() {
        assert_eq!(
            verdict("https://github.com/VighneshHM/net-lantern/releases/tag/v1"),
            Some(Nav::Browser)
        );
        assert_eq!(
            verdict("https://github.com/VighneshHM/net-lantern/issues"),
            Some(Nav::Deny)
        );
        assert_eq!(verdict(LOCATION_SETTINGS), Some(Nav::Outside));
        assert_eq!(verdict("http://127.0.0.1:47800/"), Some(Nav::Allow));
    }

    /// The URL parser trims surrounding spaces and lowercases the scheme, so
    /// these reach the check as the exact link; what goes to Windows is the
    /// constant either way, never the URL.
    #[test]
    fn spellings_the_parser_normalises_are_the_exact_link() {
        for s in [
            "ms-settings:privacy-location ",
            " ms-settings:privacy-location",
            "MS-SETTINGS:privacy-location",
        ] {
            let url: Url = s.parse().unwrap();
            assert_eq!(url.as_str(), LOCATION_SETTINGS, "{s:?}");
        }
    }

    #[test]
    fn a_window_with_a_session_is_not_signed_in_again() {
        let url = "http://127.0.0.1:47800/?t=abc";
        assert_eq!(sign_in_target(true, url), None);
        assert_eq!(
            sign_in_target(false, url).map(|u| u.to_string()),
            Some(url.to_string())
        );
    }

    #[test]
    fn login_url_is_never_logged() {
        let token = "0123456789abcdef";
        let good = format!("http://127.0.0.1:47800/?t={token}");
        let bad = format!("not a url ?t={token}");
        let lines = netlantern::logging::capture_lines(|| {
            let _ = sign_in_target(false, &good);
            let _ = sign_in_target(true, &good);
            assert_eq!(sign_in_target(false, &bad), None);
            tracing::info!("canary-marker");
        });
        assert!(
            lines.iter().any(|l| l.contains("canary-marker")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("login address")),
            "the bad URL is reported: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .all(|l| !l.contains("?t=") && !l.contains(token)),
            "{lines:?}"
        );
    }
}
