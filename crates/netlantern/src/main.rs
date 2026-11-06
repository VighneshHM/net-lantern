//! The `netlantern` executable. Phase 0 only runs against a simulated network
//! (`--demo`): it serves the dashboard on 127.0.0.1, prints a one-time login
//! URL and opens it in the browser. A second launch asks the running one to
//! open its dashboard instead of starting another server.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use netlantern::cli::Cli;
#[cfg(windows)]
use netlantern::instance::{self, Instance};
use netlantern_core::ScanConfig;
use netlantern_platform::SimNetwork;
use netlantern_server::{bind, run, AccessPolicy, AppState, Auth, DEFAULT_PORTS};

/// The simulated home network behind a bare `--demo`.
const HOME_SCENARIO: &str = include_str!("../../../scenarios/home.json");

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let Some(demo) = &cli.demo else {
        print_err("Real network scanning arrives in phase 1. For now run: netlantern --demo");
        return ExitCode::from(2);
    };
    init_logging();

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

    let sim = match load_demo(demo.as_deref()) {
        Ok(sim) => sim,
        Err(message) => {
            print_err(&message);
            return ExitCode::FAILURE;
        }
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
    let state = AppState::new(Arc::new(sim), ScanConfig::default(), policy);

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
}
