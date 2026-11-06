//! The stages of one quick scan: network, sweep, ARP, ports, discovery, merge.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;

use futures::stream::{self, StreamExt};
use netlantern_platform::{ArpEntry, Interface, MacAddr, NeighborState, PingReply, TcpProbe};
use tokio::time::{Instant, MissedTickBehavior};

use super::merge::{is_isolated, keep_live, merge_sightings, present_addresses, sighting};
use super::{lock, subnet_text, unix_now, Failure, Inner, InterfaceKey, KnownNetwork};
use crate::discover::discover;
use crate::events::EventKind;
use crate::model::{DeviceId, NetworkId, ScanId, ScanStage, Sighting};

/// A sweep progress event is published after this many replies, and at the end.
const PROGRESS_EVERY: u32 = 16;

/// One scan in progress: what it has learned so far, and the stage timer.
struct Run<'a> {
    inner: &'a Inner,
    id: ScanId,
    iface: &'a Interface,
    targets: &'a [Ipv4Addr],
    target_set: HashSet<Ipv4Addr>,
    total: u32,
    network: NetworkId,
    /// Live ARP entries seen during the sweep, by swept address.
    swept_arp: HashMap<Ipv4Addr, ArpEntry>,
    /// Addresses already streamed during the sweep.
    published: HashSet<Ipv4Addr>,
    /// Every device recorded so far, for the progress snapshot.
    found: HashSet<DeviceId>,
    stage: (&'static str, Instant),
}

impl Inner {
    /// The quick scan, start to finish except `scan.done`. Returns whether the
    /// network looks isolated.
    pub(super) async fn quick_scan(
        &self,
        id: ScanId,
        iface: &Interface,
        targets: &[Ipv4Addr],
    ) -> Result<bool, Failure> {
        let begun = Instant::now();
        let (network, gateway_mac) = self.identify_network(iface).await?;
        self.store.set_scan_network(id, network).await?;
        *lock(&self.network) = Some(KnownNetwork {
            key: InterfaceKey::of(iface),
            gateway_mac,
            id: network,
        });

        let mut run = Run {
            inner: self,
            id,
            iface,
            targets,
            target_set: targets.iter().copied().collect(),
            total: u32::try_from(targets.len()).unwrap_or(u32::MAX),
            network,
            swept_arp: HashMap::new(),
            published: HashSet::new(),
            found: HashSet::new(),
            stage: ("network", begun),
        };
        // The gateway is known to be alive, and is never asked again.
        if run.target_set.contains(&iface.gateway) {
            run.swept_arp.insert(
                iface.gateway,
                ArpEntry {
                    ip: iface.gateway,
                    mac: gateway_mac,
                    state: NeighborState::Reachable,
                },
            );
        }

        let replies = run.sweep().await?;
        let (present, arp) = run.arp(&replies).await?;
        let ports = run.ports(&present).await?;

        run.enter(ScanStage::Discover);
        let signals = discover(&*self.netio, iface, &present, &self.config.discover).await;

        run.enter(ScanStage::Merge);
        let sightings = merge_sightings(iface, &present, &replies, &arp, ports, signals);
        let mut merged = HashSet::new();
        for s in sightings {
            merged.insert(run.record(s).await?);
        }
        let devices_found = u32::try_from(merged.len()).unwrap_or(u32::MAX);
        self.update(id, |snapshot| snapshot.devices_found = devices_found);
        run.end_stage();
        tracing::info!(
            scan = id.0,
            elapsed_ms = begun.elapsed().as_millis() as u64,
            devices_found,
            "scan finished"
        );

        Ok(is_isolated(iface, &present))
    }

    /// The network the gateway identifies, created on first sight, and the
    /// gateway's MAC (from the ARP table, else an ARP request).
    async fn identify_network(&self, iface: &Interface) -> Result<(NetworkId, MacAddr), Failure> {
        let table = self.platform.arp_table().await?;
        let listed = table
            .iter()
            .find(|e| e.ip == iface.gateway && e.state.is_present())
            .map(|e| e.mac);
        let mac = match listed {
            Some(mac) => Some(mac),
            None => {
                self.platform
                    .arp_resolve(iface.gateway, self.config.arp_resolve_timeout)
                    .await?
            }
        };
        let Some(mac) = mac else {
            return Err(Failure(format!(
                "Your router at {} didn't answer, so this network can't be identified.",
                iface.gateway
            )));
        };
        let network = self
            .store
            .network_for(mac, &subnet_text(iface), unix_now())
            .await?;
        Ok((network, mac))
    }

    /// One address: ping, and try again if nobody answered.
    async fn ping(&self, ip: Ipv4Addr) -> Result<Option<PingReply>, Failure> {
        for _ in 0..=self.config.icmp_retries {
            if let Some(reply) = self.platform.ping(ip, self.config.icmp_timeout).await? {
                return Ok(Some(reply));
            }
        }
        Ok(None)
    }
}

impl Run<'_> {
    /// Pings every target and returns the replies by address. Meanwhile the
    /// ARP table is read every `arp_poll_interval`, and each address that
    /// replied and has a live entry is recorded and streamed straight away.
    async fn sweep(&mut self) -> Result<HashMap<Ipv4Addr, PingReply>, Failure> {
        self.enter_at(ScanStage::Sweep, 0);
        let (inner, targets) = (self.inner, self.targets);
        let config = &inner.config;
        // A concurrency of zero would never poll a single ping.
        let mut pings = stream::iter(targets.iter().copied())
            .map(|ip| async move { (ip, inner.ping(ip).await) })
            .buffer_unordered(config.icmp_concurrency.max(1));
        let mut poll = tokio::time::interval(
            config
                .arp_poll_interval
                .max(std::time::Duration::from_millis(1)),
        );
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let mut replies = HashMap::new();
        let mut done = 0;
        loop {
            tokio::select! {
                next = pings.next() => {
                    let Some((ip, result)) = next else { break };
                    if let Some(reply) = result? {
                        replies.insert(ip, reply);
                    }
                    done += 1;
                    if done % PROGRESS_EVERY == 0 || done == self.total {
                        self.progress(ScanStage::Sweep, done);
                    }
                }
                _ = poll.tick() => self.stream_new_devices(&replies).await?,
            }
        }
        Ok(replies)
    }

    /// Records and streams every address that has replied and has a live ARP
    /// entry (or is this laptop), once. A private MAC the network does not
    /// know yet waits for the merge.
    async fn stream_new_devices(
        &mut self,
        replies: &HashMap<Ipv4Addr, PingReply>,
    ) -> Result<(), Failure> {
        for entry in self.inner.platform.arp_table().await? {
            if self.target_set.contains(&entry.ip) && entry.state.is_present() {
                self.swept_arp.insert(entry.ip, entry);
            }
        }
        let mut ready: Vec<Ipv4Addr> = replies
            .keys()
            .copied()
            .filter(|ip| !self.published.contains(ip))
            .filter(|ip| *ip == self.iface.ip || self.swept_arp.contains_key(ip))
            .collect();
        ready.sort();
        for ip in ready {
            // A private MAC this network has never seen may be a known device
            // that changed its address; only the name found in discovery can
            // tell, so it is recorded in the merge, with its evidence.
            if let Some(entry) = self.swept_arp.get(&ip) {
                if entry.mac.is_locally_administered()
                    && !self.inner.store.mac_known(self.network, entry.mac).await?
                {
                    continue;
                }
            }
            let s = sighting(
                self.iface,
                ip,
                replies.get(&ip),
                &self.swept_arp,
                Vec::new(),
                Default::default(),
            );
            self.record(s).await?;
            self.published.insert(ip);
        }
        Ok(())
    }

    /// Resolves the swept addresses the ARP table knows nothing about, then
    /// returns the present addresses and the ARP entries to use for them.
    async fn arp(
        &mut self,
        replies: &HashMap<Ipv4Addr, PingReply>,
    ) -> Result<(Vec<Ipv4Addr>, HashMap<Ipv4Addr, ArpEntry>), Failure> {
        self.enter(ScanStage::Arp);
        let inner = self.inner;
        let config = &inner.config;
        let listed = self.read_table().await?;
        let unlisted: Vec<Ipv4Addr> = self
            .targets
            .iter()
            .copied()
            .filter(|ip| *ip != self.iface.ip)
            // Addresses seen in the table during the sweep count as listed.
            .filter(|ip| !listed.contains_key(ip) && !self.swept_arp.contains_key(ip))
            .collect();
        let mut resolving = stream::iter(unlisted)
            .map(|ip| async move {
                let mac = inner
                    .platform
                    .arp_resolve(ip, config.arp_resolve_timeout)
                    .await;
                (ip, mac)
            })
            .buffer_unordered(config.arp_resolve_concurrency.max(1));
        let mut resolved = Vec::new();
        while let Some((ip, mac)) = resolving.next().await {
            if let Some(mac) = mac? {
                resolved.push(ArpEntry {
                    ip,
                    mac,
                    state: NeighborState::Reachable,
                });
            }
        }
        drop(resolving);

        // The table again, now holding what the requests learned. Anything
        // seen alive during this scan stays alive.
        let mut arp = self.read_table().await?;
        for entry in resolved.into_iter().chain(self.swept_arp.values().copied()) {
            keep_live(&mut arp, entry);
        }
        let present = present_addresses(self.targets, replies, &arp);
        Ok((present, arp))
    }

    /// The ARP table, limited to the swept addresses.
    async fn read_table(&self) -> Result<HashMap<Ipv4Addr, ArpEntry>, Failure> {
        let mut table = HashMap::new();
        for entry in self.inner.platform.arp_table().await? {
            if self.target_set.contains(&entry.ip) {
                keep_live(&mut table, entry);
            }
        }
        Ok(table)
    }

    /// Probes the configured TCP ports on every present address but this
    /// laptop's, and returns the open ones by address.
    async fn ports(
        &mut self,
        present: &[Ipv4Addr],
    ) -> Result<HashMap<Ipv4Addr, Vec<u16>>, Failure> {
        self.enter(ScanStage::Ports);
        let inner = self.inner;
        let config = &inner.config;
        let probes: Vec<(Ipv4Addr, u16)> = present
            .iter()
            .copied()
            .filter(|ip| *ip != self.iface.ip)
            .flat_map(|ip| config.tcp_ports.iter().map(move |port| (ip, *port)))
            .collect();
        let mut probing = stream::iter(probes)
            .map(|(ip, port)| async move {
                let result = inner.platform.tcp_probe(ip, port, config.tcp_timeout).await;
                (ip, port, result)
            })
            .buffer_unordered(config.tcp_concurrency.max(1));
        let mut open: HashMap<Ipv4Addr, Vec<u16>> = HashMap::new();
        while let Some((ip, port, result)) = probing.next().await {
            if result? == TcpProbe::Open {
                open.entry(ip).or_default().push(port);
            }
        }
        for ports in open.values_mut() {
            ports.sort_unstable();
        }
        Ok(open)
    }

    /// Stores a sighting and streams the device as stored.
    async fn record(&mut self, s: Sighting) -> Result<DeviceId, Failure> {
        let device = self
            .inner
            .store
            .record_sighting(self.network, self.id, s, unix_now())
            .await?;
        let id = device.id;
        self.found.insert(id);
        let found = u32::try_from(self.found.len()).unwrap_or(u32::MAX);
        self.inner
            .update(self.id, |snapshot| snapshot.devices_found = found);
        self.inner.bus.publish(EventKind::ScanDevice {
            scan_id: self.id,
            device,
        });
        Ok(id)
    }

    /// Starts a stage that follows the sweep: it is announced already at 100%.
    fn enter(&mut self, stage: ScanStage) {
        self.enter_at(stage, self.total);
    }

    fn enter_at(&mut self, stage: ScanStage, done: u32) {
        self.end_stage();
        self.stage = (stage_name(stage), Instant::now());
        self.inner
            .update(self.id, |snapshot| snapshot.stage = stage);
        self.progress(stage, done);
    }

    /// Logs how long the current stage took.
    fn end_stage(&self) {
        let (name, since) = self.stage;
        tracing::info!(
            scan = self.id.0,
            stage = name,
            elapsed_ms = since.elapsed().as_millis() as u64,
            "scan stage finished"
        );
    }

    fn progress(&self, stage: ScanStage, done: u32) {
        self.inner.update(self.id, |snapshot| snapshot.done = done);
        self.inner.bus.publish(EventKind::ScanProgress {
            scan_id: self.id,
            done,
            total: self.total,
            stage,
        });
    }
}

fn stage_name(stage: ScanStage) -> &'static str {
    match stage {
        ScanStage::Sweep => "sweep",
        ScanStage::Arp => "arp",
        ScanStage::Ports => "ports",
        ScanStage::Discover => "discover",
        ScanStage::Merge => "merge",
    }
}
