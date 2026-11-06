//! The quick scan: an ICMP sweep, an ARP table read, and a merge of the two
//! into devices that are stored and streamed on the event bus.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::stream::{self, StreamExt};
use futures::FutureExt;
use netlantern_platform::{ArpEntry, Interface, PingReply, Platform, PlatformError};
use serde::Serialize;

use crate::devices::DeviceStore;
use crate::error::CoreError;
use crate::events::{EventBus, EventKind};
use crate::model::{DeviceSummary, ScanId, ScanMode, ScanStage, ScanState};
use crate::targets::scan_targets;

/// A sweep progress event is published after this many replies, and at the end.
const PROGRESS_EVERY: u32 = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanConfig {
    /// How many pings are in flight at once.
    pub icmp_concurrency: usize,
    /// How long each ping waits for a reply.
    pub icmp_timeout: Duration,
    /// Extra attempts for an address that has not answered yet.
    pub icmp_retries: u32,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            icmp_concurrency: 64,
            icmp_timeout: Duration::from_millis(800),
            icmp_retries: 1,
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

/// Where a scan is at, as returned by the API.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScanSnapshot {
    pub id: ScanId,
    pub mode: ScanMode,
    pub state: ScanState,
    pub stage: ScanStage,
    /// Addresses swept so far (equal to `total` once the sweep is over).
    pub done: u32,
    pub total: u32,
    pub devices_found: u32,
}

/// Starts scans and tracks them. Cheap to share: methods take `&self`, and a
/// scan runs as a background task that outlives the call that started it.
pub struct ScanManager {
    inner: Arc<Inner>,
}

struct Inner {
    platform: Arc<dyn Platform>,
    bus: Arc<EventBus>,
    devices: Arc<DeviceStore>,
    config: ScanConfig,
    /// The scan in progress, if any. At most one runs at a time.
    running: Mutex<Option<ScanId>>,
    snapshots: Mutex<HashMap<ScanId, ScanSnapshot>>,
    next_id: AtomicU64,
}

impl ScanManager {
    pub fn new(
        platform: Arc<dyn Platform>,
        bus: Arc<EventBus>,
        devices: Arc<DeviceStore>,
        config: ScanConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                platform,
                bus,
                devices,
                config,
                running: Mutex::new(None),
                snapshots: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
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
        let iface = self.inner.platform.active_interface().await?;
        let targets = scan_targets(iface.ip, iface.prefix);
        let total = u32::try_from(targets.len()).unwrap_or(u32::MAX);

        let id = {
            // Lock order: `running`, then `snapshots`, then the bus.
            let mut running = lock(&self.inner.running);
            // Checked again: another start may have claimed the guard while
            // the interface was being read.
            if let Some(id) = *running {
                return Ok(StartOutcome::AlreadyRunning(id));
            }
            let id = ScanId(self.inner.next_id.fetch_add(1, Ordering::Relaxed));
            lock(&self.inner.snapshots).insert(
                id,
                ScanSnapshot {
                    id,
                    mode,
                    state: ScanState::Running,
                    stage: ScanStage::Sweep,
                    done: 0,
                    total,
                    devices_found: 0,
                },
            );
            *running = Some(id);
            id
        };

        tokio::spawn(Arc::clone(&self.inner).run_scan(id, iface, targets));
        Ok(StartOutcome::Started(id))
    }

    pub fn snapshot(&self, id: ScanId) -> Option<ScanSnapshot> {
        lock(&self.inner.snapshots).get(&id).cloned()
    }
}

impl Inner {
    /// Runs one scan to the end and publishes `scan.done`, whatever happened,
    /// including a panic in the platform code.
    async fn run_scan(self: Arc<Self>, id: ScanId, iface: Interface, targets: Vec<Ipv4Addr>) {
        let outcome = AssertUnwindSafe(self.quick_scan(id, &iface, &targets))
            .catch_unwind()
            .await;
        let state = match outcome {
            Ok(Ok(())) => ScanState::Done,
            Ok(Err(_)) | Err(_) => ScanState::Failed,
        };

        // Finish in one critical section on the guard: the snapshot turns
        // final, `scan.done` goes out and the guard is freed together. Nobody
        // is told a finished scan is running, the next scan cannot publish
        // before this one's `scan.done`, and a client that reacts to
        // `scan.done` by starting another scan is not told one is running.
        // Lock order: `running`, then `snapshots`, then the bus.
        let mut running = lock(&self.running);
        let devices_found = self
            .update(id, |snapshot| {
                snapshot.state = state;
                snapshot.devices_found
            })
            .unwrap_or(0);
        self.bus.publish(EventKind::ScanDone {
            scan_id: id,
            state,
            devices_found,
        });
        *running = None;
    }

    async fn quick_scan(
        &self,
        id: ScanId,
        iface: &Interface,
        targets: &[Ipv4Addr],
    ) -> Result<(), PlatformError> {
        let total = u32::try_from(targets.len()).unwrap_or(u32::MAX);
        let replies = self.sweep(id, targets, total).await?;

        self.enter_stage(id, ScanStage::Arp, total);
        let arp: HashMap<Ipv4Addr, ArpEntry> = self
            .platform
            .arp_table()
            .await?
            .into_iter()
            .map(|entry| (entry.ip, entry))
            .collect();

        self.enter_stage(id, ScanStage::Merge, total);
        for device in merge_devices(iface, targets, &replies, &arp) {
            self.devices.upsert(device.clone());
            self.bus.publish(EventKind::ScanDevice {
                scan_id: id,
                device,
            });
            self.update(id, |snapshot| snapshot.devices_found += 1);
        }
        Ok(())
    }

    /// Pings every target and returns the replies by address.
    async fn sweep(
        &self,
        id: ScanId,
        targets: &[Ipv4Addr],
        total: u32,
    ) -> Result<HashMap<Ipv4Addr, PingReply>, PlatformError> {
        // A concurrency of zero would never poll a single ping.
        let concurrency = self.config.icmp_concurrency.max(1);
        let mut pings = stream::iter(targets.iter().copied())
            .map(|ip| async move { (ip, self.ping(ip).await) })
            .buffer_unordered(concurrency);

        let mut replies = HashMap::new();
        let mut done = 0;
        while let Some((ip, result)) = pings.next().await {
            if let Some(reply) = result? {
                replies.insert(ip, reply);
            }
            done += 1;
            if done % PROGRESS_EVERY == 0 || done == total {
                self.update(id, |snapshot| snapshot.done = done);
                self.bus.publish(EventKind::ScanProgress {
                    scan_id: id,
                    done,
                    total,
                    stage: ScanStage::Sweep,
                });
            }
        }
        Ok(replies)
    }

    /// One address: ping, and try again if nobody answered.
    async fn ping(&self, ip: Ipv4Addr) -> Result<Option<PingReply>, PlatformError> {
        for _ in 0..=self.config.icmp_retries {
            if let Some(reply) = self.platform.ping(ip, self.config.icmp_timeout).await? {
                return Ok(Some(reply));
            }
        }
        Ok(None)
    }

    /// Records and announces the start of a stage that follows the sweep.
    fn enter_stage(&self, id: ScanId, stage: ScanStage, total: u32) {
        self.update(id, |snapshot| {
            snapshot.stage = stage;
            snapshot.done = total;
        });
        self.bus.publish(EventKind::ScanProgress {
            scan_id: id,
            done: total,
            total,
            stage,
        });
    }

    fn update<T>(&self, id: ScanId, change: impl FnOnce(&mut ScanSnapshot) -> T) -> Option<T> {
        lock(&self.snapshots).get_mut(&id).map(change)
    }
}

/// Combines ping replies and ARP entries into the devices present on the swept
/// addresses, in address order.
///
/// A device is present if it replied to a ping or has a live ARP entry. Its MAC
/// comes from ARP; this laptop's own comes from its interface.
fn merge_devices(
    iface: &Interface,
    targets: &[Ipv4Addr],
    replies: &HashMap<Ipv4Addr, PingReply>,
    arp: &HashMap<Ipv4Addr, ArpEntry>,
) -> Vec<DeviceSummary> {
    targets
        .iter()
        .filter_map(|&ip| {
            let reply = replies.get(&ip);
            let neighbor = arp.get(&ip).filter(|entry| entry.state.is_present());
            if reply.is_none() && neighbor.is_none() {
                return None;
            }
            let is_self = ip == iface.ip;
            let mac = if is_self {
                Some(iface.mac)
            } else {
                neighbor.map(|entry| entry.mac)
            };
            Some(DeviceSummary {
                ip,
                mac,
                private_mac: mac.is_some_and(|mac| mac.is_locally_administered()),
                answers_ping: reply.is_some(),
                rtt_ms: reply.map(|r| u32::try_from(r.rtt.as_millis()).unwrap_or(u32::MAX)),
                is_self,
                is_gateway: ip == iface.gateway,
            })
        })
        .collect()
}

/// Locks `mutex`. The guarded state is plain data that is never left half
/// updated, so a poisoned lock is safe to keep using.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlantern_platform::{InterfaceKind, MacAddr, NeighborState};

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    fn iface() -> Interface {
        Interface {
            ip: ip("10.0.0.5"),
            prefix: 24,
            gateway: ip("10.0.0.1"),
            dns_servers: vec![],
            kind: InterfaceKind::Wifi,
            mac: MacAddr([0xF4, 0x8E, 0x38, 0x9A, 0x20, 0x7C]),
        }
    }

    fn reply() -> PingReply {
        PingReply {
            rtt: Duration::from_millis(3),
            ttl: 64,
        }
    }

    fn neighbor(addr: &str, state: NeighborState) -> (Ipv4Addr, ArpEntry) {
        (
            ip(addr),
            ArpEntry {
                ip: ip(addr),
                mac: MacAddr([0x50, 0xC7, 0xBF, 0x3A, 0x11, 0x02]),
                state,
            },
        )
    }

    #[test]
    fn neighbors_that_are_not_present_add_no_device_and_no_mac() {
        let targets = [ip("10.0.0.2"), ip("10.0.0.3"), ip("10.0.0.4")];
        // .2 is stale (still believed present); .3 answered ping but its ARP
        // entry never completed; .4 is marked unreachable and did not answer.
        let arp = HashMap::from([
            neighbor("10.0.0.2", NeighborState::Stale),
            neighbor("10.0.0.3", NeighborState::Incomplete),
            neighbor("10.0.0.4", NeighborState::Unreachable),
        ]);
        let replies = HashMap::from([(ip("10.0.0.3"), reply())]);

        let devices = merge_devices(&iface(), &targets, &replies, &arp);

        let summary: Vec<_> = devices
            .iter()
            .map(|d| (d.ip, d.mac.is_some(), d.answers_ping))
            .collect();
        assert_eq!(
            summary,
            [(ip("10.0.0.2"), true, false), (ip("10.0.0.3"), false, true)]
        );
    }

    #[test]
    fn arp_entries_outside_the_swept_addresses_are_ignored() {
        let targets = [ip("10.0.0.2")];
        let arp = HashMap::from([
            neighbor("10.0.0.2", NeighborState::Reachable),
            neighbor("10.0.0.255", NeighborState::Permanent),
            neighbor("224.0.0.22", NeighborState::Permanent),
        ]);

        let devices = merge_devices(&iface(), &targets, &HashMap::new(), &arp);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, ip("10.0.0.2"));
    }

    #[test]
    fn this_laptop_uses_its_interface_mac() {
        let targets = [ip("10.0.0.5")];
        let replies = HashMap::from([(ip("10.0.0.5"), reply())]);

        let devices = merge_devices(&iface(), &targets, &replies, &HashMap::new());

        assert_eq!(devices[0].mac, Some(iface().mac));
        assert!(devices[0].is_self && !devices[0].is_gateway);
    }

    #[test]
    fn sub_millisecond_rtts_round_down_to_zero() {
        let fast = PingReply {
            rtt: Duration::from_micros(400),
            ttl: 128,
        };
        let replies = HashMap::from([(ip("10.0.0.2"), fast)]);

        let devices = merge_devices(&iface(), &[ip("10.0.0.2")], &replies, &HashMap::new());

        assert_eq!(devices[0].rtt_ms, Some(0));
    }
}
