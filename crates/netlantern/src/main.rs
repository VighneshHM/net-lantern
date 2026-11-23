//! The `netlantern` executable. It serves the dashboard on 127.0.0.1, prints a
//! one-time login URL and opens it in the browser. A second launch asks the
//! running one to open its dashboard instead of starting another server.
//! `--demo` runs against a simulated network; without it Net Lantern scans the
//! real one (Windows only). `--scan-once`, `--diagnose-once`, `--wifi-once` and
//! `--phone-check` print one report and exit.

use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use netlantern::cli::Cli;
use netlantern::host::{
    open_backend, Backend, BackendParts, Host, HostError, HostOptions, Started,
};
#[cfg(windows)]
use netlantern::instance;
use netlantern::notify::desktop_notifier;
use netlantern::report;
use netlantern_core::{
    CongestionConfig, CoreError, DiagnoseConfig, DiagnoseManager, DiagnoseStart, EventBus,
    NoopNotifier, Notifier, RunState, ScanConfig, ScanManager, ScanMode, ScanState, StartOutcome,
    WifiService,
};
use netlantern_server::{HostKind, DEFAULT_PORTS};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();
    install_panic_hook();

    if cli.diagnose_once {
        return diagnose_once(&cli).await;
    }
    if cli.wifi_once {
        return wifi_once(&cli).await;
    }
    if cli.scan_once {
        return scan_once(&cli).await;
    }
    if cli.phone_check {
        return phone_check(&cli).await;
    }

    warn_ignored_data_dir(&cli, false);
    let notifier: Arc<dyn Notifier> = if cli.no_notify {
        Arc::new(NoopNotifier)
    } else {
        desktop_notifier()
    };
    let opts = HostOptions {
        backend: backend(&cli),
        port: cli.port,
        dev_origins: cli.dev_origin.clone(),
        notifier,
        autostart: None,
        host_kind: HostKind::Exe,
        on_open: Arc::new(|url| announce(&url, true)),
        pipe_name: pipe_name(),
    };
    let host = match Host::start(opts).await {
        Ok(Started::Running(host)) => host,
        Ok(Started::AlreadyRunning) => {
            print_out("Net Lantern is already running; opened its dashboard.");
            return ExitCode::SUCCESS;
        }
        Err(e) => return fail(&e),
    };

    announce(&host.login_url(), !cli.no_browser);

    match host.serve_until(shutdown_signal()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            print_err(&format!("Net Lantern stopped unexpectedly: {e}"));
            ExitCode::FAILURE
        }
    }
}

/// The single-instance pipe for this user (overridable for tests).
#[cfg(windows)]
fn pipe_name() -> String {
    instance::pipe_name()
}

#[cfg(not(windows))]
fn pipe_name() -> String {
    String::new()
}

/// What `cli` asks the server to run against.
fn backend(cli: &Cli) -> Backend {
    match &cli.demo {
        Some(scenario) => Backend::Demo {
            scenario: scenario.clone(),
            time_scale: cli.time_scale,
        },
        None => Backend::Real {
            data_dir: cli.data_dir.clone(),
        },
    }
}

/// Prints `e` and returns its exit code: 2 when real scanning is not
/// available here, 1 otherwise.
fn fail(e: &HostError) -> ExitCode {
    print_err(&e.to_string());
    match e {
        HostError::Unsupported => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

/// `--data-dir` does nothing in demo mode or a one-shot mode (`in_memory`);
/// says so.
fn warn_ignored_data_dir(cli: &Cli, in_memory: bool) {
    if cli.data_dir.is_some() && (cli.demo.is_some() || in_memory) {
        let why = if cli.demo.is_some() {
            "demo mode; demo data stays in memory"
        } else {
            "this one-shot mode; nothing is kept on disk"
        };
        print_err(&format!("Warning: --data-dir is ignored in {why}."));
    }
}

/// The platform, network access and a throwaway store for a one-shot mode.
/// The error is the exit code, after the reason has been printed.
fn build_backend(cli: &Cli) -> Result<BackendParts, ExitCode> {
    warn_ignored_data_dir(cli, true);
    let backend = match &cli.demo {
        Some(scenario) => Backend::Demo {
            scenario: scenario.clone(),
            time_scale: cli.time_scale,
        },
        None => Backend::Memory,
    };
    open_backend(&backend).map_err(|e| fail(&e))
}

/// `--scan-once`: one quick scan (a deep one with `--deep`) against a
/// throwaway store, the result as a table on stdout. No pipe, no server.
async fn scan_once(cli: &Cli) -> ExitCode {
    let (platform, netio, store) = match build_backend(cli) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let manager = ScanManager::new(
        platform,
        netio,
        Arc::new(EventBus::new(256)),
        store.clone(),
        ScanConfig::default(),
    );

    let mode = if cli.deep {
        print_err("Deep scan: only scan networks you own or have permission to scan.");
        match manager.set_scan_consent(true).await {
            Ok(_) => ScanMode::Deep,
            Err(CoreError::NotConnected) => {
                print_err("This computer is not connected to a network.");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                print_err(&format!("Could not start the deep scan: {e}"));
                return ExitCode::FAILURE;
            }
        }
    } else {
        ScanMode::Quick
    };

    let started = Instant::now();
    let id = match manager.start(mode).await {
        Ok(StartOutcome::Started(id) | StartOutcome::AlreadyRunning(id)) => id,
        Err(CoreError::NotConnected) => {
            print_err("This computer is not connected to a network.");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            print_err(&format!("Could not start the scan: {e}"));
            return ExitCode::FAILURE;
        }
    };

    let snapshot = loop {
        match manager.snapshot(id).await {
            Ok(Some(s)) if s.state != ScanState::Running => break s,
            Ok(Some(_)) => tokio::time::sleep(Duration::from_millis(100)).await,
            Ok(None) | Err(_) => {
                print_err("The scan stopped without a result.");
                return ExitCode::FAILURE;
            }
        }
    };
    let elapsed = started.elapsed();
    if snapshot.state != ScanState::Done {
        print_err(
            snapshot
                .error
                .as_deref()
                .unwrap_or("The scan did not finish."),
        );
        return ExitCode::FAILURE;
    }

    let devices = match manager.current_network().await {
        Ok(Some(network)) => store.list_devices(network).await,
        Ok(None) => Ok(Vec::new()),
        Err(e) => {
            print_err(&format!("Could not read the scan results: {e}"));
            return ExitCode::FAILURE;
        }
    };
    match devices {
        Ok(devices) => {
            // A closed stdout just ends the output.
            let mut out = report::render(&devices, elapsed);
            if cli.deep {
                out.push('\n');
                for line in report::deep_lines(&devices) {
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            let _ = write!(io::stdout(), "{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            print_err(&format!("Could not read the scan results: {e}"));
            ExitCode::FAILURE
        }
    }
}

/// `--diagnose-once`: one diagnosis against a throwaway backend, printed as
/// text. No pipe, no server. The verdict does not change the exit code.
async fn diagnose_once(cli: &Cli) -> ExitCode {
    let (platform, netio, _store) = match build_backend(cli) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let wifi = Arc::new(WifiService::new(
        Arc::clone(&platform),
        CongestionConfig::default(),
    ));
    let manager = DiagnoseManager::new(
        platform,
        netio,
        wifi,
        Arc::new(EventBus::new(256)),
        DiagnoseConfig::default(),
    );
    let (DiagnoseStart::Started(id) | DiagnoseStart::AlreadyRunning(id)) = manager.start().await;
    match manager.wait(id).await {
        Some(run) if run.state != RunState::Failed => {
            let _ = write!(io::stdout(), "{}", report::diagnosis_report(&run));
            ExitCode::SUCCESS
        }
        Some(run) => stopped(run.error.as_deref()),
        None => stopped(None),
    }
}

fn stopped(error: Option<&str>) -> ExitCode {
    print_err(&format!(
        "The diagnosis stopped: {}",
        error.unwrap_or("no result was recorded")
    ));
    ExitCode::FAILURE
}

/// `--wifi-once`: the current connection and one nearby scan, as text. A
/// location-permission message is a normal report, so the exit code is 0.
async fn wifi_once(cli: &Cli) -> ExitCode {
    let (platform, _netio, _store) = match build_backend(cli) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let wifi = WifiService::new(platform, CongestionConfig::default());
    let status = wifi.status().await;
    let nearby = wifi.nearby(Duration::ZERO).await;
    let _ = write!(io::stdout(), "{}", report::wifi_report(&status, &nearby));
    ExitCode::SUCCESS
}

/// `--phone-check`: the network category and this program's firewall rules for
/// the dashboard port, as text. Only reads; changes no setting.
async fn phone_check(cli: &Cli) -> ExitCode {
    let (platform, _netio, _store) = match build_backend(cli) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            print_err(&format!(
                "Could not tell where Net Lantern is running from: {e}"
            ));
            return ExitCode::FAILURE;
        }
    };
    let port = cli.port.unwrap_or(*DEFAULT_PORTS.start());
    let category = platform.network_category().await;
    let firewall = platform.firewall_status(&exe, port).await;
    let _ = write!(
        io::stdout(),
        "{}",
        report::phone_report(&category, &firewall)
    );
    ExitCode::SUCCESS
}

/// Logs to stderr at INFO. Tokens, session ids and the dashboard URL never go
/// through here: the URL is printed to stdout and handed to the browser only.
fn init_logging() {
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .finish();
    // Not `.init()`: that would also route `log` records into tracing, and the
    // webbrowser crate logs the command it runs, URL included, at debug level.
    let _ = tracing::subscriber::set_global_default(subscriber);
}

/// Logs a panic's message and location, then lets the previous hook run. The
/// next start closes the interrupted monitoring session as a crash.
fn install_panic_hook() {
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
        previous(info);
    }));
}

/// Prints the dashboard URL on stdout (the e2e harness reads this line), then
/// opens it in the browser if asked. The URL carries a one-time token, so it
/// is never logged.
fn announce(url: &str, open: bool) {
    print_out(&format!("Net Lantern running at {url}"));
    if open {
        if let Err(e) = webbrowser::open(url) {
            tracing::warn!("could not open the browser: {e}");
        }
    }
}

/// Completes on Ctrl+C.
async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_ok() {
        tracing::info!("shutting down");
    } else {
        // No handler means no clean way to stop; keep serving.
        std::future::pending::<()>().await;
    }
}

// `println!` and `eprintln!` panic when the stream is closed; a launcher that
// went away must not take the server down.
fn print_out(line: &str) {
    let _ = writeln!(io::stdout(), "{line}");
}

fn print_err(line: &str) {
    let _ = writeln!(io::stderr(), "{line}");
}
