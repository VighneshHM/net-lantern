//! The `netlantern` executable. It serves the dashboard on 127.0.0.1, prints a
//! one-time login URL and opens it in the browser. A second launch asks the
//! running one to open its dashboard instead of starting another server.
//! `--demo` runs against a simulated network; without it Net Lantern scans the
//! real one (Windows only). `--scan-once`, `--diagnose-once` and `--wifi-once`
//! print one report and exit.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use netlantern::cli::Cli;
#[cfg(windows)]
use netlantern::instance::{self, Instance};
use netlantern::report;
#[cfg(windows)]
use netlantern_core::StoreError;
use netlantern_core::{
    CongestionConfig, CoreError, Db, DiagnoseConfig, DiagnoseManager, DiagnoseStart, EventBus,
    RunState, ScanConfig, ScanManager, ScanMode, ScanState, StartOutcome, Store, WifiService,
};
use netlantern_platform::{NetIo, Platform, SimNetwork};
use netlantern_server::{bind, run, AccessPolicy, AppState, Auth, DEFAULT_PORTS};

/// The simulated home network behind a bare `--demo`.
const HOME_SCENARIO: &str = include_str!("../../../scenarios/home.json");

/// What a run scans, and where it keeps what it finds.
type Backend = (Arc<dyn Platform>, Arc<dyn NetIo>, Store);

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging();

    if cli.diagnose_once {
        return diagnose_once(&cli).await;
    }
    if cli.wifi_once {
        return wifi_once(&cli).await;
    }
    if cli.scan_once {
        return scan_once(&cli).await;
    }

    #[cfg(windows)]
    let pipe = match instance::acquire(&instance::pipe_name()) {
        Ok(Instance::Primary(server)) => Some(server),
        Ok(Instance::Secondary) => return open_running_instance().await,
        Err(e) => {
            // Better two servers than none: carry on without the guard.
            tracing::warn!("cannot check for a running Net Lantern: {e}");
            None
        }
    };

    // After the pipe: a second launch never opens the database.
    let (platform, netio, store) = match build_backend(&cli, false) {
        Ok(backend) => backend,
        Err(code) => return code,
    };

    let ports = cli.port.map_or(DEFAULT_PORTS, |port| port..=port);
    let (listener, port) = match bind(ports).await {
        Ok(bound) => bound,
        Err(e) => {
            print_err(&match cli.port {
                Some(port) => format!(
                    "Port {port} is in use. \
                     Close the program using it or choose another with --port <number>."
                ),
                None => e.to_string(),
            });
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(port, "serving on 127.0.0.1");

    let policy = AccessPolicy::for_port(port, &cli.dev_origin);
    let state = AppState::new(
        platform,
        netio,
        store,
        ScanConfig::default(),
        DiagnoseConfig::default(),
        policy,
    );

    #[cfg(windows)]
    if let Some(server) = pipe {
        let auth = Arc::clone(&state.auth);
        tokio::spawn(instance::serve_open_requests(
            server,
            instance::pipe_name(),
            move || announce(&auth, port, true),
        ));
    }

    announce(&state.auth, port, !cli.no_browser);

    match run(listener, state, shutdown_signal()).await {
        Ok(()) => {
            tracing::info!("stopped");
            ExitCode::SUCCESS
        }
        Err(e) => {
            print_err(&format!("Net Lantern stopped unexpectedly: {e}"));
            ExitCode::FAILURE
        }
    }
}

/// Builds the platform, network access and store for this run. `in_memory`
/// keeps the real store off the disk (the one-shot modes). The error is the exit
/// code, after the reason has been printed.
fn build_backend(cli: &Cli, in_memory: bool) -> Result<Backend, ExitCode> {
    if cli.data_dir.is_some() && (cli.demo.is_some() || in_memory) {
        let why = if cli.demo.is_some() {
            "demo mode; demo data stays in memory"
        } else {
            "this one-shot mode; nothing is kept on disk"
        };
        print_err(&format!("Warning: --data-dir is ignored in {why}."));
    }
    if let Some(demo) = &cli.demo {
        let sim = match load_demo(demo.as_deref()) {
            Ok(sim) => Arc::new(sim),
            Err(message) => {
                print_err(&message);
                return Err(ExitCode::FAILURE);
            }
        };
        // Demo mode never touches user data: its database lives in memory.
        let store = open_memory_store("the demo database")?;
        return Ok((sim.clone(), sim, store));
    }
    real_backend(cli, in_memory)
}

fn open_memory_store(what: &str) -> Result<Store, ExitCode> {
    Db::open_in_memory().map(Store::new).map_err(|e| {
        print_err(&format!("Could not open {what}: {e}"));
        ExitCode::FAILURE
    })
}

#[cfg(windows)]
fn real_backend(cli: &Cli, in_memory: bool) -> Result<Backend, ExitCode> {
    use netlantern_platform::{RealNetIo, WindowsPlatform};

    let store = if in_memory {
        open_memory_store("a temporary database")?
    } else {
        open_data_store(cli)?
    };
    Ok((
        Arc::new(WindowsPlatform::new()),
        Arc::new(RealNetIo::new()),
        store,
    ))
}

#[cfg(not(windows))]
fn real_backend(_cli: &Cli, _in_memory: bool) -> Result<Backend, ExitCode> {
    print_err("Real network scanning only works on Windows so far. Run netlantern --demo instead.");
    Err(ExitCode::from(2))
}

/// `--data-dir`, or `%LOCALAPPDATA%\NetLantern`.
#[cfg_attr(not(windows), allow(dead_code))]
fn data_dir(cli: &Cli, local_app_data: Option<&str>) -> Result<PathBuf, String> {
    if let Some(dir) = &cli.data_dir {
        return Ok(dir.clone());
    }
    match local_app_data.filter(|v| !v.is_empty()) {
        Some(base) => Ok(Path::new(base).join("NetLantern")),
        None => Err(
            "Net Lantern cannot tell where to keep its data: the LOCALAPPDATA \
                     setting is missing. Start it with --data-dir <folder> to choose a folder."
                .to_string(),
        ),
    }
}

#[cfg(windows)]
fn open_data_store(cli: &Cli) -> Result<Store, ExitCode> {
    let dir = match data_dir(cli, std::env::var("LOCALAPPDATA").ok().as_deref()) {
        Ok(dir) => dir,
        Err(message) => {
            print_err(&message);
            return Err(ExitCode::FAILURE);
        }
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        print_err(&format!(
            "Could not open Net Lantern's data in {}: {e}",
            dir.display()
        ));
        return Err(ExitCode::FAILURE);
    }
    match Db::open(&dir.join("netlantern.db")) {
        Ok(db) => Ok(Store::new(db)),
        Err(StoreError::NewerSchema { .. }) => {
            print_err(
                "This data was written by a newer Net Lantern. Update Net Lantern, or start it \
                 with --data-dir <folder> to use another folder.",
            );
            Err(ExitCode::FAILURE)
        }
        Err(e) => {
            print_err(&format!(
                "Could not open Net Lantern's data in {}: {e}",
                dir.display()
            ));
            Err(ExitCode::FAILURE)
        }
    }
}

/// `--scan-once`: one quick scan against a throwaway store, the result as a
/// table on stdout. No pipe, no server.
async fn scan_once(cli: &Cli) -> ExitCode {
    let (platform, netio, store) = match build_backend(cli, true) {
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

    let started = Instant::now();
    let id = match manager.start(ScanMode::Quick).await {
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
            let _ = write!(io::stdout(), "{}", report::render(&devices, elapsed));
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
    let (platform, netio, _store) = match build_backend(cli, true) {
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
    let (platform, _netio, _store) = match build_backend(cli, true) {
        Ok(backend) => backend,
        Err(code) => return code,
    };
    let wifi = WifiService::new(platform, CongestionConfig::default());
    let status = wifi.status().await;
    let nearby = wifi.nearby(Duration::ZERO).await;
    let _ = write!(io::stdout(), "{}", report::wifi_report(&status, &nearby));
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

/// The scenario to simulate: the built-in home network, or `file`.
fn load_demo(file: Option<&Path>) -> Result<SimNetwork, String> {
    match file {
        None => SimNetwork::from_json(HOME_SCENARIO)
            .map_err(|e| format!("The built-in demo network is broken: {e}")),
        Some(path) => SimNetwork::load(path)
            .map_err(|e| format!("Could not load scenario file {}: {e}", path.display())),
    }
}

/// Issues a fresh one-time launch token and prints the dashboard URL on
/// stdout (the e2e harness reads this line), then opens it in the browser if
/// asked. The URL carries the token, so it is never logged.
fn announce(auth: &Auth, port: u16, open: bool) {
    let url = format!("http://127.0.0.1:{port}/?t={}", auth.issue_launch_token());
    print_out(&format!("Net Lantern running at {url}"));
    if open {
        if let Err(e) = webbrowser::open(&url) {
            tracing::warn!("could not open the browser: {e}");
        }
    }
}

/// Asks the Net Lantern that is already running to open its dashboard.
#[cfg(windows)]
async fn open_running_instance() -> ExitCode {
    match instance::notify_open(&instance::pipe_name()).await {
        Ok(()) => {
            print_out("Net Lantern is already running; opened its dashboard.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            print_err(&format!(
                "Net Lantern is already running, but its dashboard could not be opened: {e}"
            ));
            ExitCode::FAILURE
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_demo_network_loads() {
        assert_eq!(load_demo(None).unwrap().name(), "home");
    }

    #[test]
    fn a_missing_scenario_file_is_named_in_the_error() {
        let path = std::env::temp_dir().join("netlantern-no-such-scenario.json");
        let message = load_demo(Some(&path)).unwrap_err();
        assert!(message.contains(&path.display().to_string()), "{message}");
    }

    #[test]
    fn a_broken_scenario_file_is_named_in_the_error() {
        let path = std::env::temp_dir().join(format!("netlantern-bad-{}.json", std::process::id()));
        std::fs::write(&path, "{ not json").unwrap();
        let message = load_demo(Some(&path)).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains("invalid scenario JSON"), "{message}");
    }

    #[test]
    fn the_data_dir_flag_wins_over_local_app_data() {
        let cli = Cli::parse_from(["netlantern", "--data-dir", "elsewhere"]);
        assert_eq!(
            data_dir(&cli, Some("base")).unwrap(),
            PathBuf::from("elsewhere")
        );
    }

    #[test]
    fn the_default_data_dir_is_under_local_app_data() {
        let cli = Cli::parse_from(["netlantern"]);
        assert_eq!(
            data_dir(&cli, Some("base")).unwrap(),
            Path::new("base").join("NetLantern")
        );
    }

    #[test]
    fn a_missing_local_app_data_suggests_data_dir() {
        let cli = Cli::parse_from(["netlantern"]);
        for unset in [None, Some("")] {
            let message = data_dir(&cli, unset).unwrap_err();
            assert!(message.contains("LOCALAPPDATA"), "{message}");
            assert!(message.contains("--data-dir"), "{message}");
        }
    }
}
