//! Quick and deep scans: the network is identified by its gateway, then an
//! ICMP sweep (streaming devices as they appear), ARP resolution, TCP port
//! probes and discovery feed sightings into the store, and every device is
//! streamed on the event bus. A deep scan, allowed per network, probes 100
//! ports and reads banners and page titles from the open ones.
//!
//! A running scan can be stopped, and stops by itself when the laptop leaves
//! the network or loses its connection; either way it ends `Cancelled` and
//! keeps the devices it recorded.
//!
//! This file holds the [`ScanManager`] (starting and stopping scans, the
//! running guard, snapshots and the finish critical section); the stages live in
//! [`stages`], and the pure merge helpers in [`merge`].

mod merge;
mod stages;

use stages::Identity;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use netlantern_platform::{Interface, MacAddr, NetIo, Platform, PlatformError};
use serde::Serialize;

use crate::discover::DiscoverConfig;
use crate::error::CoreError;
use crate::events::{EventBus, EventKind};
use crate::identify::ports::DEEP_PORTS;
use crate::model::{NetworkId, ScanId, ScanMode, ScanSnapshot, ScanStage, ScanState};
use crate::store::{Store, StoreError};
use crate::targets::scan_targets;

/// What a scan that panicked reports to the user.
const PANIC_MESSAGE: &str = "Internal error during the scan.";

/// How many scans' snapshots are kept in memory; older ones are read from the
/// store.
const KEPT_SNAPSHOTS: usize = 16;

/// What a scan reports when the store fails; the raw error is only logged.
const STORE_MESSAGE: &str = "Couldn't save the scan results.";

/// Why a scan stopped when the laptop moved to another network.
const NETWORK_CHANGED: &str = "The network changed during the scan.";

/// Why a scan stopped when the laptop lost its connection.
const CONNECTION_DROPPED: &str = "The connection dropped during the scan.";

/// A request to stop a scan: `Some(reason)` once asked, where a user's stop
/// has no reason.
type Stop = Option<Option<String>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanConfig {
    /// How many pings are in flight at once.
    pub icmp_concurrency: usize,
    /// How long each ping waits for a reply.
    pub icmp_timeout: Duration,
    /// Extra attempts for an address that has not answered yet.
    pub icmp_retries: u32,
    /// Ports probed on every present device (except this laptop), to help
    /// identify it.
    pub tcp_ports: Vec<u16>,
    /// How long each TCP connect waits.
    pub tcp_timeout: Duration,
    /// How many TCP probes are in flight at once.
    pub tcp_concurrency: usize,
    /// How many ARP requests are in flight at once.
    pub arp_resolve_concurrency: usize,
    /// How long each ARP request waits.
    pub arp_resolve_timeout: Duration,
    /// How often the ARP table is read during the sweep to stream devices.
    pub arp_poll_interval: Duration,
    pub discover: DiscoverConfig,
    /// Ports a deep scan probes on every present device (except this laptop).
    pub deep_ports: Vec<u16>,
    /// How many of a deep scan's TCP connects may be in flight to one device.
    pub deep_per_host: usize,
    /// How long a deep scan waits for a service greeting or a web page.
    pub banner_timeout: Duration,
    /// How many banner and page reads are in flight at once.
    pub banner_concurrency: usize,
    /// How often a running scan checks that the laptop is still on the
    /// network it is scanning.
    pub network_watch: Duration,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            icmp_concurrency: 64,
            icmp_timeout: Duration::from_millis(800),
            icmp_retries: 1,
            tcp_ports: vec![80, 443, 445, 22, 62078],
            tcp_timeout: Duration::from_millis(800),
            tcp_concurrency: 64,
            arp_resolve_concurrency: 32,
            arp_resolve_timeout: Duration::from_secs(3),
            arp_poll_interval: Duration::from_secs(1),
            discover: DiscoverConfig::default(),
            deep_ports: DEEP_PORTS.to_vec(),
            deep_per_host: 8,
            banner_timeout: Duration::from_secs(2),
            banner_concurrency: 16,
            network_watch: Duration::from_secs(2),
        }
    }
}

/// The result of asking for a scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    Started(ScanId),
    /// A scan is already in progress; its id is returned instead.
    AlreadyRunning(ScanId),
}

/// A known network whose router looks replaced: the laptop's gateway has a
/// MAC no network has, behind the SSID and subnet of this one. Until the user
/// says whether it is the same network, it is not scanned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReplacedRouter {
    pub network_id: NetworkId,
    /// The SSID the network was last seen under.
    pub label: String,
}

/// Starts scans and tracks them. Cheap to share: methods take `&self`, and a
/// scan runs as a background task that outlives the call that started it.
pub struct ScanManager {
    inner: Arc<Inner>,
}

/// What identifies "the same network" for [`ScanManager::current_network`]
/// without asking the store.
#[derive(Clone, Copy, PartialEq, Eq)]
struct InterfaceKey {
    ip: Ipv4Addr,
    prefix: u8,
    gateway: Ipv4Addr,
}

impl InterfaceKey {
    fn of(iface: &Interface) -> Self {
        Self {
            ip: iface.ip,
            prefix: iface.prefix,
            gateway: iface.gateway,
        }
    }
}

/// The network last identified: by a scan, or by the gateway's MAC in the
/// ARP table.
#[derive(Clone, Copy)]
struct KnownNetwork {
    key: InterfaceKey,
    gateway_mac: MacAddr,
    id: NetworkId,
}

struct Inner {
    platform: Arc<dyn Platform>,
    netio: Arc<dyn NetIo>,
    bus: Arc<EventBus>,
    store: Store,
    config: ScanConfig,
    /// Serializes `start` calls between the first look at `running` and
    /// claiming it, so the scan row is only created by the call that wins.
    starting: tokio::sync::Mutex<()>,
    /// The scan in progress, if any. At most one runs at a time.
    running: Mutex<Option<Running>>,
    /// The most recent scans run by this process (at most [`KEPT_SNAPSHOTS`]).
    snapshots: Mutex<HashMap<ScanId, ScanSnapshot>>,
    /// The network last identified, its gateway's MAC, and the interface it
    /// was on.
    network: Mutex<Option<KnownNetwork>>,
    /// Serializes identification and replaced-router answers, so the check
    /// for a pending decision and the network it creates or moves are one
    /// step.
    identifying: tokio::sync::Mutex<()>,
}

/// The scan in progress, and how to stop it.
struct Running {
    id: ScanId,
    /// Asks the scan to stop; the receiver is in the scan's task.
    stop: tokio::sync::watch::Sender<Stop>,
    /// The scan's outcome is settled: it is finishing, and a stop no longer
    /// changes it.
    sealed: bool,
}

/// Why a scan stage failed, in words for the user.
#[derive(Debug)]
struct Failure(String);

impl From<PlatformError> for Failure {
    fn from(e: PlatformError) -> Self {
        Failure(e.to_string())
    }
}

impl From<StoreError> for Failure {
    /// Database details mean nothing to the user: they are logged, and the
    /// scan reports [`STORE_MESSAGE`].
    fn from(e: StoreError) -> Self {
        tracing::warn!(error = %e, "the store failed during a scan");
        Failure(STORE_MESSAGE.to_string())
    }
}

impl ScanManager {
    pub fn new(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        bus: Arc<EventBus>,
        store: Store,
        config: ScanConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                platform,
                netio,
                bus,
                store,
                config,
                starting: tokio::sync::Mutex::new(()),
                running: Mutex::new(None),
                snapshots: Mutex::new(HashMap::new()),
                network: Mutex::new(None),
                identifying: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Starts a scan in the background, or reports the one already running.
    ///
    /// A running scan is reported first, even if the link has dropped since it
    /// began. Then the network interface is read before anything is spawned,
    /// so a missing connection is reported here ([`CoreError::NotConnected`])
    /// rather than as a failed scan. Then a known network whose router looks
    /// replaced ([`ScanManager::replaced_router`]) waits for the user's answer
    /// ([`CoreError::NetworkDecisionNeeded`]), and a deep scan needs consent
    /// for the network the laptop is on ([`CoreError::ConsentNeeded`]
    /// otherwise); in each case no scan is created. The running scan checks
    /// both again against the network it identifies.
    pub async fn start(&self, mode: ScanMode) -> Result<StartOutcome, CoreError> {
        if let Some(id) = self.inner.running_id() {
            return Ok(StartOutcome::AlreadyRunning(id));
        }
        let _starting = self.inner.starting.lock().await;
        // Checked again: a scan may have been claimed while this call waited.
        if let Some(id) = self.inner.running_id() {
            return Ok(StartOutcome::AlreadyRunning(id));
        }
        let iface = self.inner.platform.active_interface().await?;
        // A platform that fails or panics here is left to the scan, which
        // identifies the network itself and reports the failure.
        let pending = AssertUnwindSafe(self.inner.replaced_router_on(&iface))
            .catch_unwind()
            .await;
        match pending {
            Ok(Ok(Some(ReplacedRouter { network_id, label }))) => {
                return Err(CoreError::NetworkDecisionNeeded {
                    candidate: network_id,
                    label,
                });
            }
            Ok(Err(e @ CoreError::Store(_))) => return Err(e),
            Ok(Ok(None) | Err(_)) | Err(_) => {}
        }
        if mode == ScanMode::Deep && !matches!(self.scan_consent().await?, Some((_, true))) {
            return Err(CoreError::ConsentNeeded);
        }
        let targets = scan_targets(iface.ip, iface.prefix);
        let total = u32::try_from(targets.len()).unwrap_or(u32::MAX);
        let id = self.inner.store.insert_scan(mode, unix_now()).await?;
        let (stop, stop_rx) = tokio::sync::watch::channel::<Stop>(None);

        {
            // Lock order: `running`, then `snapshots`, then the bus. Only a
            // holder of `starting` sets `running`, so it is still free here.
            let mut running = lock(&self.inner.running);
            let mut snapshots = lock(&self.inner.snapshots);
            // Ids grow, so the smallest are the oldest; the new scan is not
            // running yet, and every older one has finished.
            while snapshots.len() >= KEPT_SNAPSHOTS {
                let Some(oldest) = snapshots.keys().min().copied() else {
                    break;
                };
                snapshots.remove(&oldest);
            }
            snapshots.insert(
                id,
                ScanSnapshot {
                    id,
                    mode,
                    state: ScanState::Running,
                    stage: ScanStage::Sweep,
                    done: 0,
                    total,
                    devices_found: 0,
                    error: None,
                    isolated: false,
                },
            );
            drop(snapshots);
            *running = Some(Running {
                id,
                stop,
                sealed: false,
            });
        }

        tokio::spawn(Arc::clone(&self.inner).run_scan(id, mode, iface, targets, stop_rx));
        Ok(StartOutcome::Started(id))
    }

    /// Tells the running scan to stop. True when `id` is the scan in
    /// progress and it has now been told to stop: it then ends `Cancelled`,
    /// keeping the devices recorded so far. False for a finished or unknown
    /// id, including a scan already finishing on its own.
    pub fn cancel(&self, id: ScanId) -> bool {
        match &*lock(&self.inner.running) {
            Some(running) if running.id == id && !running.sealed => {
                // `send_replace` stores the value even with no receiver left.
                running.stop.send_replace(Some(None));
                true
            }
            _ => false,
        }
    }

    /// The network this laptop is on and whether deep scans are allowed on
    /// it. Passive, like [`ScanManager::current_network`]: `None` when the
    /// laptop is offline or on a network not yet identified.
    pub async fn scan_consent(&self) -> Result<Option<(NetworkId, bool)>, CoreError> {
        let Some(network) = self.current_network().await? else {
            return Ok(None);
        };
        let granted = self.inner.store.scan_consent(network).await?;
        Ok(Some((network, granted)))
    }

    /// Allows or forbids deep scans on the network this laptop is on.
    ///
    /// The network is identified actively, as a scan does (asking the router
    /// for its MAC if the ARP table lacks it), and created if new; it is then
    /// remembered for [`ScanManager::current_network`]. A router that does not
    /// answer is [`CoreError::Unidentified`]; a known network whose router
    /// looks replaced is [`CoreError::NetworkDecisionNeeded`].
    pub async fn set_scan_consent(&self, granted: bool) -> Result<NetworkId, CoreError> {
        let iface = self.inner.platform.active_interface().await?;
        let identity = self
            .inner
            .identify(&iface)
            .await
            .map_err(|Failure(msg)| CoreError::Unidentified(msg))?;
        let (network, gateway_mac) = match identity {
            Identity::Network(network, mac) => (network, mac),
            Identity::Decision { candidate, label } => {
                return Err(CoreError::NetworkDecisionNeeded { candidate, label });
            }
        };
        self.inner.store.set_scan_consent(network, granted).await?;
        self.inner.remember(&iface, gateway_mac, network);
        Ok(network)
    }

    /// The known network whose router looks replaced, while the user has not
    /// said whether this is it.
    ///
    /// Passive: ARP table only, no probes. `None` when not connected, the
    /// gateway is not in the table, or no decision is pending (no SSID, a
    /// network already has this gateway, or none has this SSID and subnet).
    pub async fn replaced_router(&self) -> Result<Option<ReplacedRouter>, CoreError> {
        let iface = match self.inner.platform.active_interface().await {
            Ok(iface) => iface,
            Err(PlatformError::NotConnected) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        self.inner.replaced_router_on(&iface).await
    }

    /// Answers [`ScanManager::replaced_router`]: `keep` moves the known
    /// network (its devices and names) to the current gateway MAC, otherwise
    /// a new network is created. Returns the network now in use, remembered
    /// for [`ScanManager::current_network`].
    ///
    /// The network is identified actively, as a scan does, and the question
    /// read again: [`CoreError::Unidentified`] when the router does not
    /// answer or no decision is pending.
    pub async fn decide_replaced_router(&self, keep: bool) -> Result<NetworkId, CoreError> {
        let iface = self.inner.platform.active_interface().await?;
        let (network, gateway_mac) = self.inner.decide(&iface, keep).await?;
        self.inner.remember(&iface, gateway_mac, network);
        Ok(network)
    }

    /// A recent scan this process ran, from memory; any other from the store.
    pub async fn snapshot(&self, id: ScanId) -> Result<Option<ScanSnapshot>, CoreError> {
        if let Some(snapshot) = lock(&self.inner.snapshots).get(&id).cloned() {
            return Ok(Some(snapshot));
        }
        Ok(self.inner.store.scan(id).await?)
    }

    /// The scan in progress, or else the most recent scan of the network this
    /// laptop is on (from this process or an earlier one). `None` when no scan
    /// is running and the laptop is offline or on a network never scanned.
    ///
    /// A running scan is always reported: it may not have identified its
    /// network yet.
    pub async fn latest(&self) -> Result<Option<ScanSnapshot>, CoreError> {
        if let Some(running) = self.running_snapshot() {
            return Ok(Some(running));
        }
        let Some(network) = self.current_network().await? else {
            return Ok(None);
        };
        let Some(stored) = self.inner.store.latest_scan_for(network).await? else {
            return Ok(None);
        };
        // A scan this process ran keeps its final progress figures.
        let here = lock(&self.inner.snapshots).get(&stored.id).cloned();
        Ok(Some(here.unwrap_or(stored)))
    }

    /// The snapshot of the scan in progress, if any.
    fn running_snapshot(&self) -> Option<ScanSnapshot> {
        // Lock order: `running`, then `snapshots`.
        let running = lock(&self.inner.running);
        let id = running.as_ref()?.id;
        lock(&self.inner.snapshots).get(&id).cloned()
    }

    /// The network this laptop is on, if it is a known one. `None` when it is
    /// offline or on a network no scan has identified.
    ///
    /// Passive: the gateway's MAC is read from the ARP table and matched
    /// against stored networks; nothing is sent and nothing is created. While
    /// the interface (address, prefix, gateway) is the one a network was last
    /// identified on (by a scan, or by an earlier call here), that network is
    /// kept, even with the gateway gone from the table (a hung router), unless
    /// the table shows a gateway with another MAC (the same addressing on a
    /// different network).
    pub async fn current_network(&self) -> Result<Option<NetworkId>, CoreError> {
        let iface = match self.inner.platform.active_interface().await {
            Ok(iface) => iface,
            Err(PlatformError::NotConnected) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let key = InterfaceKey::of(&iface);
        let known = (*lock(&self.inner.network)).filter(|k| k.key == key);
        let table = self.inner.platform.arp_table().await?;
        let gateway_mac = table
            .iter()
            .find(|e| e.ip == iface.gateway && e.state.is_present())
            .map(|e| e.mac);
        match (known, gateway_mac) {
            (Some(known), None) => Ok(Some(known.id)),
            (Some(known), Some(mac)) if mac == known.gateway_mac => Ok(Some(known.id)),
            (_, Some(mac)) => {
                let found = self
                    .inner
                    .store
                    .find_network(mac, &subnet_text(&iface))
                    .await?;
                // Remember what the table showed, so a gateway entry that
                // goes missing later (a hung router) keeps this network while
                // the interface stays the same.
                let mut network = lock(&self.inner.network);
                match found {
                    Some(id) => {
                        *network = Some(KnownNetwork {
                            key,
                            gateway_mac: mac,
                            id,
                        });
                    }
                    // Another gateway on the same addressing: the network
                    // remembered for it is no longer this one.
                    None => {
                        let unchanged = |n: &KnownNetwork| {
                            known.is_some_and(|k| {
                                (k.key, k.gateway_mac, k.id) == (n.key, n.gateway_mac, n.id)
                            })
                        };
                        if network.as_ref().is_some_and(unchanged) {
                            *network = None;
                        }
                    }
                }
                Ok(found)
            }
            (None, None) => Ok(None),
        }
    }
}

impl Inner {
    /// Remembers the network identified on `iface`, for
    /// [`ScanManager::current_network`].
    fn remember(&self, iface: &Interface, gateway_mac: MacAddr, id: NetworkId) {
        *lock(&self.network) = Some(KnownNetwork {
            key: InterfaceKey::of(iface),
            gateway_mac,
            id,
        });
    }

    /// [`ScanManager::replaced_router`] on `iface`.
    async fn replaced_router_on(
        &self,
        iface: &Interface,
    ) -> Result<Option<ReplacedRouter>, CoreError> {
        let table = self.platform.arp_table().await?;
        let Some(mac) = table
            .iter()
            .find(|e| e.ip == iface.gateway && e.state.is_present())
            .map(|e| e.mac)
        else {
            return Ok(None);
        };
        let subnet = subnet_text(iface);
        // A known gateway needs no Wi-Fi read.
        if self.store.find_network(mac, &subnet).await?.is_some() {
            return Ok(None);
        }
        let ssid = self.current_ssid().await;
        let pending = self.pending_decision(mac, &subnet, ssid.as_deref()).await?;
        Ok(pending.map(|(network_id, label)| ReplacedRouter { network_id, label }))
    }

    /// The id of the scan in progress, if any.
    fn running_id(&self) -> Option<ScanId> {
        lock(&self.running).as_ref().map(|r| r.id)
    }

    /// Runs one scan to the end and publishes `scan.done`, whatever happened:
    /// a panic in the platform code, a stop, or the laptop leaving the
    /// network. A panic is reported as failed with the internal-error text and
    /// is not relabelled by the interface check.
    async fn run_scan(
        self: Arc<Self>,
        id: ScanId,
        mode: ScanMode,
        iface: Interface,
        targets: Vec<Ipv4Addr>,
        mut stop: tokio::sync::watch::Receiver<Stop>,
    ) {
        // A stop or a network change drops the scan future, abandoning the
        // probes in flight. Store calls already sent still finish on the
        // database thread, before the ones below.
        let ended = tokio::select! {
            biased;
            reason = stop_requested(&mut stop) => Ended::Stopped(reason),
            outcome = AssertUnwindSafe(self.scan(id, mode, &iface, &targets)).catch_unwind() => {
                Ended::Ran(outcome)
            }
            reason = self.watch_network(InterfaceKey::of(&iface)) => {
                Ended::Stopped(Some(reason.to_string()))
            }
        };

        // A failure may be the link going down or the laptop moving before
        // the watcher looked (probes fail within milliseconds): one look at
        // the interface tells, at most `network_watch` long. A stop asked for
        // meanwhile still wins below.
        let ended = match ended {
            Ended::Ran(Ok(Err(failure))) => {
                let look = self.network_left(InterfaceKey::of(&iface));
                let within = self.config.network_watch.max(Duration::from_millis(1));
                match tokio::time::timeout(within, look).await {
                    Ok(Some(reason)) => Ended::Stopped(Some(reason.to_string())),
                    _ => Ended::Ran(Ok(Err(failure))),
                }
            }
            ended => ended,
        };

        // Seal the outcome: from here `cancel` is refused, and a stop asked
        // for before the seal (even after the scan's last step) wins, so a
        // `cancel` that returned true always ends the scan cancelled.
        let asked = {
            let mut running = lock(&self.running);
            if let Some(running) = running.as_mut().filter(|r| r.id == id) {
                running.sealed = true;
            }
            stop.borrow().clone()
        };
        let ended = match (ended, asked) {
            (Ended::Ran(_), Some(reason)) => Ended::Stopped(reason),
            (ended, _) => ended,
        };

        let (state, error, isolated) = match ended {
            Ended::Ran(Ok(Ok(isolated))) => (ScanState::Done, None, isolated),
            Ended::Ran(Ok(Err(Failure(error)))) => {
                tracing::warn!(scan = id.0, %error, "scan failed");
                (ScanState::Failed, Some(error), false)
            }
            Ended::Ran(Err(_)) => {
                tracing::warn!(scan = id.0, error = PANIC_MESSAGE, "scan panicked");
                (ScanState::Failed, Some(PANIC_MESSAGE.to_string()), false)
            }
            Ended::Stopped(reason) => {
                tracing::info!(scan = id.0, reason = reason.as_deref(), "scan cancelled");
                (ScanState::Cancelled, reason, false)
            }
        };
        let devices_found = self.update(id, |s| s.devices_found).unwrap_or(0);

        // The row is final before `scan.done` goes out, so whoever reacts to
        // it reads the finished scan (and its devices' online state).
        let finished = self
            .store
            .finish_scan(
                id,
                state,
                error.clone(),
                isolated,
                devices_found,
                unix_now(),
            )
            .await;
        // A result that could not be saved is not reported as done.
        let (state, error, isolated) = match finished {
            Ok(()) => (state, error, isolated),
            Err(e) => {
                tracing::warn!(scan = id.0, error = %e, "could not record the end of the scan");
                (ScanState::Failed, Some(STORE_MESSAGE.to_string()), false)
            }
        };

        // Finish in one critical section on the guard: the snapshot turns
        // final, `scan.done` goes out and the guard is freed together. Nobody
        // is told a finished scan is running, the next scan cannot publish
        // before this one's `scan.done`, and a client that reacts to
        // `scan.done` by starting another scan is not told one is running.
        // Lock order: `running`, then `snapshots`, then the bus.
        let mut running = lock(&self.running);
        self.update(id, |snapshot| {
            snapshot.state = state;
            snapshot.error = error.clone();
            snapshot.isolated = isolated;
        });
        self.bus.publish(EventKind::ScanDone {
            scan_id: id,
            state,
            error,
            isolated,
            devices_found,
        });
        *running = None;
    }

    /// Resolves with a reason once the laptop is no longer on the network
    /// `scanned` (another address, prefix or gateway) or has no connection.
    /// Checked every `network_watch`; other errors are ignored, as is a
    /// panicking platform.
    async fn watch_network(&self, scanned: InterfaceKey) -> &'static str {
        // `sleep` copes with a huge period (it never ends), where an
        // interval's deadline arithmetic would overflow.
        let every = self.config.network_watch.max(Duration::from_millis(1));
        loop {
            tokio::time::sleep(every).await;
            if let Some(reason) = self.network_left(scanned).await {
                return reason;
            }
        }
    }

    /// One look at the interface: why the laptop is no longer on the network
    /// `scanned`, if it is not. Other errors and a panicking platform are
    /// logged and count as still on it.
    async fn network_left(&self, scanned: InterfaceKey) -> Option<&'static str> {
        let read = AssertUnwindSafe(self.platform.active_interface())
            .catch_unwind()
            .await;
        match read {
            Ok(Ok(iface)) if InterfaceKey::of(&iface) != scanned => Some(NETWORK_CHANGED),
            Ok(Err(PlatformError::NotConnected)) => Some(CONNECTION_DROPPED),
            Ok(Ok(_)) => None,
            Ok(Err(error)) => {
                tracing::debug!(%error, "couldn't check the network during a scan");
                None
            }
            Err(_) => {
                tracing::debug!("the network check panicked during a scan");
                None
            }
        }
    }

    fn update<T>(&self, id: ScanId, change: impl FnOnce(&mut ScanSnapshot) -> T) -> Option<T> {
        lock(&self.snapshots).get_mut(&id).map(change)
    }
}

/// How the scan future ended: it ran to its end (or panicked), or it was
/// stopped, with the reason for the user (none for a user's stop).
enum Ended {
    Ran(std::thread::Result<Result<bool, Failure>>),
    Stopped(Option<String>),
}

/// Resolves with the reason once a stop is asked for. Never resolves if the
/// sender is gone (the scan is finishing anyway).
async fn stop_requested(stop: &mut tokio::sync::watch::Receiver<Stop>) -> Option<String> {
    let asked = match stop.wait_for(Option::is_some).await {
        Ok(asked) => asked.clone(),
        Err(_) => None,
    };
    match asked {
        Some(reason) => reason,
        None => std::future::pending().await,
    }
}

/// The interface's subnet as stored: `"192.168.1.0/24"`.
fn subnet_text(iface: &Interface) -> String {
    let prefix = iface.prefix.min(32);
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
    let network = Ipv4Addr::from(u32::from(iface.ip) & mask);
    format!("{network}/{prefix}")
}

/// Milliseconds since the Unix epoch, like every stored timestamp.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Locks `mutex`. The guarded state is plain data that is never left half
/// updated, so a poisoned lock is safe to keep using.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlantern_platform::{InterfaceKind, MacAddr};

    fn iface(ip: &str, prefix: u8) -> Interface {
        Interface {
            ip: ip.parse().unwrap(),
            prefix,
            gateway: "10.0.0.1".parse().unwrap(),
            dns_servers: vec![],
            kind: InterfaceKind::Wifi,
            mac: MacAddr([0xF4, 0x8E, 0x38, 0x9A, 0x20, 0x7C]),
        }
    }

    #[test]
    fn the_network_is_checked_every_two_seconds_by_default() {
        assert_eq!(ScanConfig::default().network_watch, Duration::from_secs(2));
    }

    #[test]
    fn subnet_text_is_the_network_address_and_prefix() {
        assert_eq!(subnet_text(&iface("192.168.1.23", 24)), "192.168.1.0/24");
        assert_eq!(subnet_text(&iface("10.0.5.9", 22)), "10.0.4.0/22");
        assert_eq!(subnet_text(&iface("10.0.5.9", 32)), "10.0.5.9/32");
        assert_eq!(subnet_text(&iface("10.0.5.9", 0)), "0.0.0.0/0");
    }
}
