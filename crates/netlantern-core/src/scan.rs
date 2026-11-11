//! The quick scan: the network is identified by its gateway, then an ICMP sweep
//! (streaming devices as they appear), ARP resolution, TCP port probes and
//! discovery feed sightings into the store, and every device is streamed on the
//! event bus.
//!
//! This file holds the [`ScanManager`] (starting scans, the running guard,
//! snapshots and the finish critical section); the stages live in
//! [`stages`], and the pure merge helpers in [`merge`].

mod merge;
mod stages;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use netlantern_platform::{Interface, MacAddr, NetIo, Platform, PlatformError};

use crate::discover::DiscoverConfig;
use crate::error::CoreError;
use crate::events::{EventBus, EventKind};
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

/// The network the last scan identified.
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
    running: Mutex<Option<ScanId>>,
    /// The most recent scans run by this process (at most [`KEPT_SNAPSHOTS`]).
    snapshots: Mutex<HashMap<ScanId, ScanSnapshot>>,
    /// The network the last scan identified, its gateway's MAC, and the
    /// interface it was on.
    network: Mutex<Option<KnownNetwork>>,
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
            }),
        }
    }

    /// Starts a scan in the background, or reports the one already running.
    ///
    /// A running scan is reported first, even if the link has dropped since it
    /// began. Otherwise the network interface is read before anything is
    /// spawned, so a missing connection is reported here rather than as a
    /// failed scan.
    pub async fn start(&self, mode: ScanMode) -> Result<StartOutcome, CoreError> {
        if mode == ScanMode::Deep {
            return Err(CoreError::Unsupported("deep scan arrives in phase 4"));
        }
        if let Some(id) = *lock(&self.inner.running) {
            return Ok(StartOutcome::AlreadyRunning(id));
        }
        let _starting = self.inner.starting.lock().await;
        // Checked again: a scan may have been claimed while this call waited.
        if let Some(id) = *lock(&self.inner.running) {
            return Ok(StartOutcome::AlreadyRunning(id));
        }
        let iface = self.inner.platform.active_interface().await?;
        let targets = scan_targets(iface.ip, iface.prefix);
        let total = u32::try_from(targets.len()).unwrap_or(u32::MAX);
        let id = self.inner.store.insert_scan(mode, unix_now()).await?;

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
            *running = Some(id);
        }

        tokio::spawn(Arc::clone(&self.inner).run_scan(id, iface, targets));
        Ok(StartOutcome::Started(id))
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
        let id = (*running)?;
        lock(&self.inner.snapshots).get(&id).cloned()
    }

    /// The network this laptop is on, if it is a known one. `None` when it is
    /// offline or on a network no scan has identified.
    ///
    /// Passive: the gateway's MAC is read from the ARP table and matched
    /// against stored networks; nothing is sent and nothing is created. While
    /// the interface (address, prefix, gateway) is the last scan's, that scan's
    /// network is kept unless the table shows a gateway with another MAC (the
    /// same addressing on a different network).
    pub async fn current_network(&self) -> Result<Option<NetworkId>, CoreError> {
        let iface = match self.inner.platform.active_interface().await {
            Ok(iface) => iface,
            Err(PlatformError::NotConnected) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let known = (*lock(&self.inner.network)).filter(|k| k.key == InterfaceKey::of(&iface));
        let table = self.inner.platform.arp_table().await?;
        let gateway_mac = table
            .iter()
            .find(|e| e.ip == iface.gateway && e.state.is_present())
            .map(|e| e.mac);
        match (known, gateway_mac) {
            (Some(known), None) => Ok(Some(known.id)),
            (Some(known), Some(mac)) if mac == known.gateway_mac => Ok(Some(known.id)),
            (_, Some(mac)) => Ok(self
                .inner
                .store
                .find_network(mac, &subnet_text(&iface))
                .await?),
            (None, None) => Ok(None),
        }
    }
}

impl Inner {
    /// Runs one scan to the end and publishes `scan.done`, whatever happened,
    /// including a panic in the platform code.
    async fn run_scan(self: Arc<Self>, id: ScanId, iface: Interface, targets: Vec<Ipv4Addr>) {
        let outcome = AssertUnwindSafe(self.quick_scan(id, &iface, &targets))
            .catch_unwind()
            .await;
        let (state, error, isolated) = match outcome {
            Ok(Ok(isolated)) => (ScanState::Done, None, isolated),
            Ok(Err(Failure(error))) => {
                tracing::warn!(scan = id.0, %error, "scan failed");
                (ScanState::Failed, Some(error), false)
            }
            Err(_) => {
                tracing::warn!(scan = id.0, error = PANIC_MESSAGE, "scan panicked");
                (ScanState::Failed, Some(PANIC_MESSAGE.to_string()), false)
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

    fn update<T>(&self, id: ScanId, change: impl FnOnce(&mut ScanSnapshot) -> T) -> Option<T> {
        lock(&self.snapshots).get_mut(&id).map(change)
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
    fn subnet_text_is_the_network_address_and_prefix() {
        assert_eq!(subnet_text(&iface("192.168.1.23", 24)), "192.168.1.0/24");
        assert_eq!(subnet_text(&iface("10.0.5.9", 22)), "10.0.4.0/22");
        assert_eq!(subnet_text(&iface("10.0.5.9", 32)), "10.0.5.9/32");
        assert_eq!(subnet_text(&iface("10.0.5.9", 0)), "0.0.0.0/0");
    }
}
