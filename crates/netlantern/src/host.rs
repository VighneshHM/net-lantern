//! Starting and stopping Net Lantern, shared by `netlantern.exe` and the
//! desktop app: the single-instance pipe, the data folder and store, the port,
//! the [`AppState`], phone access, the monitor, and the shutdown order.
//!
//! The login URL carries a one-time token, so nothing here logs it.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(windows)]
use crate::instance::{self, Instance};
use netlantern_core::{
    Db, DiagnoseConfig, EndReason, MonitorConfig, Notifier, ScanConfig, Store, SystemClock,
};
use netlantern_platform::{NetIo, Platform, SimNetwork};
use netlantern_server::{
    bind, router, run, AccessPolicy, AppState, Auth, Autostart, HostInfo, HostKind, LanBinder,
    LogTaps, LoopbackLanBinder, MonitorParts, PhoneAccess, RealLanBinder, ServeError,
    DEFAULT_PORTS,
};
use tokio::net::TcpListener;

/// The simulated home network behind a bare `--demo`.
const HOME_SCENARIO: &str = include_str!("../../../scenarios/home.json");

/// What a run scans, and where it keeps what it finds.
#[derive(Clone, Debug)]
pub enum Backend {
    /// The real network (Windows only), with the store in `data_dir` or
    /// `%LOCALAPPDATA%\NetLantern`.
    Real { data_dir: Option<PathBuf> },
    /// A simulated network: the built-in home one or `scenario`. Demo data
    /// stays in memory.
    Demo {
        scenario: Option<PathBuf>,
        time_scale: Option<u32>,
    },
    /// The real network with a throwaway in-memory store (the one-shot modes).
    Memory,
}

/// The platform, network access and store a [`Backend`] opens.
pub type BackendParts = (Arc<dyn Platform>, Arc<dyn NetIo>, Store);

/// How to start the host.
pub struct HostOptions {
    pub backend: Backend,
    /// `None`: the first free port in [`DEFAULT_PORTS`].
    pub port: Option<u16>,
    pub dev_origins: Vec<String>,
    pub notifier: Arc<dyn Notifier>,
    /// `None` for the exe.
    pub autostart: Option<Arc<dyn Autostart>>,
    pub host_kind: HostKind,
    /// The latest log lines for the debug info; `None` shows none.
    pub logs: Option<LogTaps>,
    /// Called (on the pipe task) when a second launch asks this instance to
    /// open; receives a fresh login URL.
    pub on_open: Arc<dyn Fn(String) + Send + Sync>,
    /// [`instance::pipe_name`] in production; unused off Windows.
    pub pipe_name: String,
}

/// The outcome of [`Host::start`]. Returned once per run, so the size of the
/// `Running` variant does not matter.
#[allow(clippy::large_enum_variant)]
pub enum Started {
    Running(Host),
    /// Another Net Lantern owns the pipe; it has been asked to open.
    AlreadyRunning,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    NoFreePort(ServeError),
    #[error(
        "Port {0} is in use. Close the program using it or choose another with --port <number>."
    )]
    PortInUse(u16),
    #[error("{0}")]
    DataDir(String),
    #[error("{0}")]
    Store(String),
    #[error("{0}")]
    Demo(String),
    /// Real scanning off Windows.
    #[error("Real network scanning only works on Windows so far. Run netlantern --demo instead.")]
    Unsupported,
    /// Another Net Lantern owns the pipe but did not take the request.
    #[error("Net Lantern is already running, but its dashboard could not be opened: {0}")]
    OpenRunning(String),
}

/// A started Net Lantern: bound, monitoring, and ready to serve.
pub struct Host {
    state: AppState,
    port: u16,
    listener: TcpListener,
    phone: Arc<PhoneAccess>,
    /// Serves `open` requests from later launches; holds the pipe.
    #[cfg(windows)]
    pipe: Option<tokio::task::JoinHandle<()>>,
}

impl Host {
    /// Claims the pipe, opens the backend, binds the port, builds the state
    /// and phone access, and starts the monitor. Returns `AlreadyRunning`,
    /// after asking the running instance to open, when the pipe is taken.
    /// Must be called inside a Tokio runtime.
    pub async fn start(opts: HostOptions) -> Result<Started, HostError> {
        #[cfg(windows)]
        let pipe = match instance::acquire(&opts.pipe_name) {
            Ok(Instance::Primary(server)) => Some(server),
            Ok(Instance::Secondary) => {
                return match instance::notify_open(&opts.pipe_name).await {
                    Ok(()) => Ok(Started::AlreadyRunning),
                    Err(e) => Err(HostError::OpenRunning(e.to_string())),
                };
            }
            Err(e) => {
                // Better two servers than none: carry on without the guard.
                tracing::warn!("cannot check for a running Net Lantern: {e}");
                None
            }
        };

        // After the pipe: a second launch never opens the database.
        let (platform, netio, store) = open_backend(&opts.backend)?;

        let ports = opts.port.map_or(DEFAULT_PORTS, |port| port..=port);
        let (listener, port) = bind(ports).await.map_err(|e| match opts.port {
            Some(port) => HostError::PortInUse(port),
            None => HostError::NoFreePort(e),
        })?;
        tracing::info!(port, "serving on 127.0.0.1");

        let (demo, config) = match &opts.backend {
            Backend::Demo {
                time_scale: Some(factor),
                ..
            } => (true, MonitorConfig::default().scaled(*factor)),
            Backend::Demo { .. } => (true, MonitorConfig::default()),
            Backend::Real { .. } | Backend::Memory => (false, MonitorConfig::default()),
        };
        let mut state = AppState::with_monitor(
            platform,
            netio,
            store,
            ScanConfig::default(),
            DiagnoseConfig::default(),
            MonitorParts {
                config,
                notifier: opts.notifier,
                clock: Arc::new(SystemClock),
            },
            AccessPolicy::for_port(port, &opts.dev_origins),
        );
        state.host = HostInfo {
            kind: opts.host_kind,
            version: env!("CARGO_PKG_VERSION"),
            autostart: opts.autostart,
        };
        state.logs = opts.logs;
        let phone = phone_access(&state, port, demo);
        state.phone = Some(Arc::clone(&phone));
        phone.attach_router(router(state.clone()));

        #[cfg(windows)]
        let pipe = pipe.map(|server| {
            let auth = Arc::clone(&state.auth);
            let on_open = opts.on_open;
            tokio::spawn(instance::serve_open_requests(
                server,
                opts.pipe_name,
                move || on_open(login_url(&auth, port)),
            ))
        });
        #[cfg(not(windows))]
        let _ = (opts.on_open, opts.pipe_name);

        state.monitor.start();
        Ok(Started::Running(Self {
            state,
            port,
            listener,
            phone,
            #[cfg(windows)]
            pipe,
        }))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The dashboard URL with a fresh one-time launch token. Never log it.
    pub fn login_url(&self) -> String {
        login_url(&self.state.auth, self.port)
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Serves until `signal` completes, then stops phone access, the monitor
    /// (the session ends as `quit`) and the server, in that order, and lets go
    /// of the pipe so a later start can claim it.
    pub async fn serve_until(
        self,
        signal: impl Future<Output = ()> + Send + 'static,
    ) -> std::io::Result<()> {
        let Self {
            state,
            listener,
            phone,
            #[cfg(windows)]
            pipe,
            ..
        } = self;
        let monitor = Arc::clone(&state.monitor);
        // The listener is bound, so the server is up as soon as `run` polls it.
        tokio::spawn({
            let phone = Arc::clone(&phone);
            async move { phone.restore().await }
        });
        let shutdown = {
            let phone = Arc::clone(&phone);
            let monitor = Arc::clone(&monitor);
            async move {
                signal.await;
                phone.stop().await;
                monitor.stop(EndReason::Quit).await;
            }
        };
        let result = run(listener, state, shutdown).await;
        // Both are no-ops after a clean shutdown; they matter when the server
        // failed before the signal.
        phone.stop().await;
        monitor.stop(EndReason::Quit).await;
        #[cfg(windows)]
        if let Some(task) = pipe {
            // Dropping the task closes every instance of the pipe.
            task.abort();
            let _ = task.await;
        }
        if result.is_ok() {
            tracing::info!("stopped");
        }
        result
    }
}

/// `http://127.0.0.1:<port>/?t=<fresh launch token>`.
fn login_url(auth: &Auth, port: u16) -> String {
    format!("http://127.0.0.1:{port}/?t={}", auth.issue_launch_token())
}

/// Phone access for this run: on the LAN address for real, on loopback in
/// demo mode (the URL then works on this laptop only).
fn phone_access(state: &AppState, port: u16, demo: bool) -> Arc<PhoneAccess> {
    let binder: Arc<dyn LanBinder> = if demo {
        Arc::new(LoopbackLanBinder)
    } else {
        Arc::new(RealLanBinder)
    };
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        // Only the firewall verdict needs it; without it the verdict reads
        // as not decided.
        tracing::warn!("cannot tell where Net Lantern is running from: {e}");
        PathBuf::new()
    });
    PhoneAccess::new(
        Arc::clone(&state.platform),
        state.store.clone(),
        Arc::clone(&state.auth),
        Arc::clone(&state.bus),
        binder,
        exe,
        port,
        demo,
    )
}

/// Opens the platform, network access and store for `backend`.
pub fn open_backend(backend: &Backend) -> Result<BackendParts, HostError> {
    match backend {
        Backend::Demo {
            scenario,
            time_scale,
        } => {
            let sim =
                Arc::new(load_demo(scenario.as_deref(), *time_scale).map_err(HostError::Demo)?);
            // Demo mode never touches user data: its database lives in memory.
            let store = open_memory_store("the demo database")?;
            Ok((sim.clone(), sim, store))
        }
        Backend::Real { data_dir } => real_backend(Some(data_dir.as_deref())),
        Backend::Memory => real_backend(None),
    }
}

fn open_memory_store(what: &str) -> Result<Store, HostError> {
    Db::open_in_memory()
        .map(Store::new)
        .map_err(|e| HostError::Store(format!("Could not open {what}: {e}")))
}

/// The real platform. `on_disk` is the data folder choice (`Some(None)` for
/// the default); `None` keeps the store in memory.
#[cfg(windows)]
fn real_backend(on_disk: Option<Option<&Path>>) -> Result<BackendParts, HostError> {
    use netlantern_platform::{RealNetIo, WindowsPlatform};

    let store = match on_disk {
        Some(dir) => open_data_store(dir)?,
        None => open_memory_store("a temporary database")?,
    };
    Ok((
        Arc::new(WindowsPlatform::new()),
        Arc::new(RealNetIo::new()),
        store,
    ))
}

#[cfg(not(windows))]
fn real_backend(_on_disk: Option<Option<&Path>>) -> Result<BackendParts, HostError> {
    Err(HostError::Unsupported)
}

/// `explicit` (`--data-dir`), or `%LOCALAPPDATA%\NetLantern`.
pub fn data_dir(explicit: Option<&Path>, local_app_data: Option<&str>) -> Result<PathBuf, String> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
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
fn open_data_store(explicit: Option<&Path>) -> Result<Store, HostError> {
    use netlantern_core::StoreError;

    let dir = data_dir(explicit, std::env::var("LOCALAPPDATA").ok().as_deref())
        .map_err(HostError::DataDir)?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return Err(HostError::DataDir(format!(
            "Could not open Net Lantern's data in {}: {e}",
            dir.display()
        )));
    }
    match Db::open(&dir.join("netlantern.db")) {
        Ok(db) => Ok(Store::new(db)),
        Err(StoreError::NewerSchema { .. }) => Err(HostError::Store(
            "This data was written by a newer Net Lantern. Update Net Lantern, or start it \
             with --data-dir <folder> to use another folder."
                .to_string(),
        )),
        Err(e) => Err(HostError::Store(format!(
            "Could not open Net Lantern's data in {}: {e}",
            dir.display()
        ))),
    }
}

/// The scenario to simulate: the built-in home network, or `file`.
fn load_demo(file: Option<&Path>, time_scale: Option<u32>) -> Result<SimNetwork, String> {
    let sim = match file {
        None => SimNetwork::from_json(HOME_SCENARIO)
            .map_err(|e| format!("The built-in demo network is broken: {e}")),
        Some(path) => SimNetwork::load(path)
            .map_err(|e| format!("Could not load scenario file {}: {e}", path.display())),
    }?;
    Ok(match time_scale {
        Some(factor) => sim.with_time_scale(factor),
        None => sim,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_demo_network_loads() {
        assert_eq!(load_demo(None, None).unwrap().name(), "home");
    }

    #[test]
    fn a_time_scale_is_applied_to_the_demo() {
        assert!(load_demo(None, Some(60)).is_ok());
    }

    #[test]
    fn a_missing_scenario_file_is_named_in_the_error() {
        let path = std::env::temp_dir().join("netlantern-no-such-scenario.json");
        let message = load_demo(Some(&path), None).unwrap_err();
        assert!(message.contains(&path.display().to_string()), "{message}");
    }

    #[test]
    fn a_broken_scenario_file_is_named_in_the_error() {
        let path = std::env::temp_dir().join(format!("netlantern-bad-{}.json", std::process::id()));
        std::fs::write(&path, "{ not json").unwrap();
        let message = load_demo(Some(&path), None).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert!(message.contains("invalid scenario JSON"), "{message}");
    }

    #[test]
    fn the_data_dir_flag_wins_over_local_app_data() {
        assert_eq!(
            data_dir(Some(Path::new("elsewhere")), Some("base")).unwrap(),
            PathBuf::from("elsewhere")
        );
    }

    #[test]
    fn the_default_data_dir_is_under_local_app_data() {
        assert_eq!(
            data_dir(None, Some("base")).unwrap(),
            Path::new("base").join("NetLantern")
        );
    }

    #[test]
    fn a_missing_local_app_data_suggests_data_dir() {
        for unset in [None, Some("")] {
            let message = data_dir(None, unset).unwrap_err();
            assert!(message.contains("LOCALAPPDATA"), "{message}");
            assert!(message.contains("--data-dir"), "{message}");
        }
    }
}
