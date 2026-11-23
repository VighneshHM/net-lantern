//! Logging for Net Lantern: stderr, daily files kept for seven days, and two
//! in-memory rings of the latest lines for the support view.
//!
//! Tokens, session ids and the login or phone URL are never logged. `log`
//! records are not routed into tracing (no `.init()`): the `webbrowser` crate
//! logs the command it runs, URL included, at debug level.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::Layered;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{fmt, Layer, Registry};

/// The registry with the INFO ceiling every other layer sits on.
type Base = Layered<LevelFilter, Registry>;

/// Daily files older than this many days are deleted at start.
pub const KEEP_DAYS: u32 = 7;
const FILE_PREFIX: &str = "netlantern.log";
const LINES_KEPT: usize = 200;
const PROBLEMS_KEPT: usize = 50;

pub struct LogOptions {
    /// `<data folder>/logs`; `None` writes no files (demo and memory runs).
    pub dir: Option<PathBuf>,
    pub stderr: bool,
}

type GuardSlot = Arc<Mutex<Option<WorkerGuard>>>;

/// Keeps the file writer alive. Hold it for the life of the process; dropping
/// it flushes the files.
pub struct LogGuard {
    slot: GuardSlot,
}

/// A calendar date (year, month, day).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ymd(pub i32, pub u32, pub u32);

/// The two bounded rings of formatted lines.
pub(crate) struct Rings {
    lines: Mutex<VecDeque<String>>,
    problems: Mutex<VecDeque<String>>,
    lines_cap: usize,
    problems_cap: usize,
}

impl Rings {
    pub(crate) fn with_limits(lines: usize, problems: usize) -> Self {
        Self {
            lines: Mutex::new(VecDeque::new()),
            problems: Mutex::new(VecDeque::new()),
            lines_cap: lines,
            problems_cap: problems,
        }
    }

    fn push(&self, level: Level, line: String) {
        if level <= Level::WARN {
            push_bounded(&self.problems, self.problems_cap, line.clone());
        }
        push_bounded(&self.lines, self.lines_cap, line);
    }

    pub(crate) fn lines(&self) -> Vec<String> {
        snapshot(&self.lines)
    }

    pub(crate) fn problems(&self) -> Vec<String> {
        snapshot(&self.problems)
    }
}

fn push_bounded(ring: &Mutex<VecDeque<String>>, cap: usize, line: String) {
    let mut ring = ring.lock().unwrap_or_else(|e| e.into_inner());
    if ring.len() == cap {
        ring.pop_front();
    }
    ring.push_back(line);
}

fn snapshot(ring: &Mutex<VecDeque<String>>) -> Vec<String> {
    ring.lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .cloned()
        .collect()
}

fn global_rings() -> &'static Arc<Rings> {
    static RINGS: OnceLock<Arc<Rings>> = OnceLock::new();
    RINGS.get_or_init(|| Arc::new(Rings::with_limits(LINES_KEPT, PROBLEMS_KEPT)))
}

/// The guard slot the panic hook flushes.
fn flush_slot() -> &'static Mutex<Option<GuardSlot>> {
    static SLOT: OnceLock<Mutex<Option<GuardSlot>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// A layer that formats each event into the rings.
struct Tap(Arc<Rings>);

struct LineVisitor(String);

impl Visit for LineVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for Tap {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = LineVisitor(String::new());
        event.record(&mut visitor);
        let line = format!("{} {}: {}", meta.level(), meta.target(), visitor.0);
        self.0.push(*meta.level(), line);
    }
}

/// Installs the global subscriber once (a second call changes nothing). Files
/// roll daily as `netlantern.log.YYYY-MM-DD` and those older than
/// [`KEEP_DAYS`] days are deleted now. Level INFO.
pub fn init(opts: LogOptions) -> LogGuard {
    let (dispatch, guard) = build(opts, Arc::clone(global_rings()));
    if tracing::dispatcher::set_global_default(dispatch).is_ok() {
        *flush_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&guard.slot));
    }
    guard
}

/// The last 200 formatted lines at INFO and above, newest last.
pub fn recent_lines() -> Vec<String> {
    global_rings().lines()
}

/// The last 50 formatted lines at WARN and above, newest last.
pub fn recent_problems() -> Vec<String> {
    global_rings().problems()
}

/// Runs `f` with logging captured into fresh rings and returns the lines.
/// For tests in other crates and the executable.
#[doc(hidden)]
pub fn capture_lines(f: impl FnOnce()) -> Vec<String> {
    let rings = Arc::new(Rings::with_limits(LINES_KEPT, PROBLEMS_KEPT));
    let (dispatch, _guard) = build(
        LogOptions {
            dir: None,
            stderr: false,
        },
        Arc::clone(&rings),
    );
    tracing::dispatcher::with_default(&dispatch, f);
    rings.lines()
}

pub(crate) fn build(opts: LogOptions, rings: Arc<Rings>) -> (tracing::Dispatch, LogGuard) {
    let mut layers: Vec<Box<dyn Layer<Base> + Send + Sync>> = Vec::new();
    if opts.stderr {
        layers.push(
            fmt::layer()
                .with_writer(io::stderr)
                .with_ansi(io::stderr().is_terminal())
                .boxed(),
        );
    }
    let mut worker = None;
    if let Some(dir) = &opts.dir {
        // Logging must never stop the app: a folder that cannot be made just
        // means no files.
        if std::fs::create_dir_all(dir).is_ok() {
            prune_folder(dir);
            let appender = tracing_appender::rolling::daily(dir, FILE_PREFIX);
            let (writer, guard) = tracing_appender::non_blocking(appender);
            worker = Some(guard);
            layers.push(fmt::layer().with_writer(writer).with_ansi(false).boxed());
        }
    }
    layers.push(Tap(rings).boxed());
    let subscriber = Registry::default().with(LevelFilter::INFO).with(layers);
    (
        tracing::Dispatch::new(subscriber),
        LogGuard {
            slot: Arc::new(Mutex::new(worker)),
        },
    )
}

fn prune_folder(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for name in files_to_prune(&names, today_utc(), KEEP_DAYS) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// Today's date in UTC, which is what the daily appender names files by.
fn today_utc() -> Ymd {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0))
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(Ymd(y, m, d): Ymd) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> Ymd {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Ymd(
        i32::try_from(y).unwrap_or(0),
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

fn parse_date(s: &str) -> Option<Ymd> {
    let mut parts = s.split('-');
    let y: i32 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(Ymd(y, m, d))
}

/// The `netlantern.log.YYYY-MM-DD` files in `names` more than `keep_days`
/// days older than `today`. Anything else is left alone.
pub(crate) fn files_to_prune(names: &[String], today: Ymd, keep_days: u32) -> Vec<String> {
    let today = days_from_civil(today);
    names
        .iter()
        .filter(|name| {
            name.strip_prefix(FILE_PREFIX)
                .and_then(|rest| rest.strip_prefix('.'))
                .and_then(parse_date)
                .is_some_and(|date| today - days_from_civil(date) > i64::from(keep_days))
        })
        .cloned()
        .collect()
}

/// Logs a panic's message and location at ERROR, flushes the log files, then
/// lets the previous hook run. The next start closes the interrupted
/// monitoring session as a crash.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "no message".to_string());
        let location = info
            .location()
            .map_or_else(|| "unknown".to_string(), ToString::to_string);
        tracing::error!(%location, "panic: {message}");
        // Dropping the worker guard flushes the queued lines to the file.
        let slot = flush_slot()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(slot) = slot {
            drop(slot.lock().unwrap_or_else(|e| e.into_inner()).take());
        }
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn prunes_files_older_than_seven_days_only() {
        let all = names(&[
            "netlantern.log.2026-11-21",
            "netlantern.log.2026-11-15",
            "netlantern.log.2026-11-14",
            "netlantern.log.2026-11-13",
            "other.txt",
            "netlantern.log",
        ]);
        let pruned = files_to_prune(&all, Ymd(2026, 11, 21), KEEP_DAYS);
        assert_eq!(pruned, names(&["netlantern.log.2026-11-13"]));
    }

    #[test]
    fn pruning_crosses_month_and_year_boundaries() {
        let all = names(&["netlantern.log.2025-12-31", "netlantern.log.2025-12-24"]);
        let pruned = files_to_prune(&all, Ymd(2026, 1, 1), KEEP_DAYS);
        assert_eq!(pruned, names(&["netlantern.log.2025-12-24"]));
    }

    #[test]
    fn recent_rings_keep_the_newest_lines_up_to_their_limits() {
        let rings = Arc::new(Rings::with_limits(200, 50));
        let (dispatch, _guard) = build(
            LogOptions {
                dir: None,
                stderr: false,
            },
            Arc::clone(&rings),
        );
        tracing::dispatcher::with_default(&dispatch, || {
            for i in 0..250 {
                tracing::info!("line {i}");
            }
            tracing::debug!("below info");
        });
        let lines = rings.lines();
        assert_eq!(lines.len(), 200);
        assert!(lines[0].contains("line 50"), "{}", lines[0]);
        assert!(lines[199].contains("line 249"), "{}", lines[199]);
        assert!(!lines.iter().any(|l| l.contains("below info")));
    }

    #[test]
    fn problems_ring_only_takes_warn_and_error() {
        let rings = Arc::new(Rings::with_limits(200, 50));
        let (dispatch, _guard) = build(
            LogOptions {
                dir: None,
                stderr: false,
            },
            Arc::clone(&rings),
        );
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("fine");
            tracing::warn!("careful");
            tracing::error!("broken");
            for i in 0..60 {
                tracing::warn!("w{i}");
            }
        });
        let problems = rings.problems();
        assert_eq!(problems.len(), 50);
        assert!(problems.iter().all(|l| !l.contains("fine")));
        assert!(problems[49].contains("w59"));
        assert!(rings.lines().iter().any(|l| l.contains("fine")));
    }

    #[test]
    fn files_are_written_under_the_logs_folder() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let rings = Arc::new(Rings::with_limits(200, 50));
        let (dispatch, guard) = build(
            LogOptions {
                dir: Some(logs.clone()),
                stderr: false,
            },
            rings,
        );
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("hello from the test");
        });
        drop(guard);
        drop(dispatch);
        let mut found = false;
        for entry in std::fs::read_dir(&logs).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("netlantern.log.") {
                found |= std::fs::read_to_string(entry.path())
                    .unwrap()
                    .contains("hello from the test");
            }
        }
        assert!(found, "no netlantern.log.* file held the line");
    }

    #[test]
    fn old_files_are_pruned_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("netlantern.log.2000-01-01");
        std::fs::write(&old, "x").unwrap();
        let (_dispatch, _guard) = build(
            LogOptions {
                dir: Some(dir.path().to_path_buf()),
                stderr: false,
            },
            Arc::new(Rings::with_limits(1, 1)),
        );
        assert!(!old.exists());
    }
}
