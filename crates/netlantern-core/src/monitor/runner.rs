//! The monitor task: a tick every 30 s that probes, stores samples, keeps
//! presence and sessions, and turns readings into alerts (spec §6.8, §6.9),
//! plus the bus listener (in `listener.rs`) for new devices and diagnoses.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, PoisonError};

use futures::FutureExt;
use netlantern_platform::{NetIo, Platform};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use super::notify::Notifier;
use super::refresh::{self, Ctx};
use super::sampler::{self, Faults, StarredProbe, TickPlan};
use super::{readings, MonitorState, MonitorStatus, MonitorTick};
use crate::events::{EventBus, EventKind};
use crate::model::{DeviceId, NetworkId};
use crate::monitor::{
    conditions, AlertEngine, Clock, DeviceChecks, EndReason, LossWindow, MonitorConfig, SessionId,
    Target,
};
use crate::scan::ScanManager;
use crate::store::Store;

/// Runs monitoring in the background. See the module docs.
pub struct Monitor {
    inner: Arc<Inner>,
    running: Mutex<Option<Running>>,
}

struct Running {
    stop: watch::Sender<bool>,
    ticker: JoinHandle<()>,
    listener: JoinHandle<()>,
}

pub(super) struct Inner {
    pub(super) platform: Arc<dyn Platform>,
    pub(super) netio: Arc<dyn NetIo>,
    pub(super) store: Store,
    pub(super) bus: Arc<EventBus>,
    pub(super) scans: Arc<ScanManager>,
    pub(super) notifier: Arc<dyn Notifier>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) cfg: MonitorConfig,
    pub(super) faults: Faults,
    /// Held for a whole tick, and by the listener while it feeds the engine.
    pub(super) state: tokio::sync::Mutex<LoopState>,
    pub(super) status: Mutex<MonitorStatus>,
}

pub(super) struct Session {
    pub(super) id: SessionId,
    pub(super) network: NetworkId,
    pub(super) started_at: i64,
}

pub(super) struct LoopState {
    pub(super) engine: AlertEngine,
    /// The network whose alerts the engine holds.
    pub(super) engine_network: Option<NetworkId>,
    pub(super) loss: LossWindow,
    pub(super) session: Option<Session>,
    /// When the last tick finished.
    pub(super) last_tick_end: Option<i64>,
    pub(super) gateway_has_answered_ping: bool,
    /// Starred devices that have answered a ping this session. Only these
    /// record loss samples: a device that ignores ping (a firewalled PC, a
    /// smart plug) but answers ARP is not losing packets.
    pub(super) devices_answered_ping: HashSet<DeviceId>,
    pub(super) last_dns: Option<i64>,
    pub(super) last_presence: Option<i64>,
    pub(super) last_rollup: Option<i64>,
    /// Devices that were starred last tick.
    pub(super) watched: HashSet<DeviceId>,
    /// Whether `watched` holds an earlier tick's starred set on this network,
    /// so a device missing from it was starred since.
    pub(super) watched_primed: bool,
    /// The tick at which each device starred while monitoring ran was first
    /// seen starred: its offline grace counts from then at the earliest.
    pub(super) starred_since: HashMap<DeviceId, i64>,
}

pub(super) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn ms(d: std::time::Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

/// Whether `every` has passed since `last` (always, the first time).
fn due(last: Option<i64>, now: i64, every: std::time::Duration) -> bool {
    last.is_none_or(|l| now.saturating_sub(l) >= ms(every))
}

/// A ping result for the loss window: replied, or lost while the target was
/// still there (it answered ARP). A target that answered nothing is down,
/// not lossy, and records nothing.
fn loss_result(rtt: Option<f64>, arp: Option<bool>) -> Option<bool> {
    match (rtt, arp) {
        (Some(_), _) => Some(true),
        (None, Some(true)) => Some(false),
        _ => None,
    }
}

impl Monitor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        store: Store,
        bus: Arc<EventBus>,
        scans: Arc<ScanManager>,
        notifier: Arc<dyn Notifier>,
        clock: Arc<dyn Clock>,
        cfg: MonitorConfig,
    ) -> Self {
        let state = LoopState {
            engine: AlertEngine::new(cfg.consecutive),
            engine_network: None,
            loss: LossWindow::new(cfg.loss_window),
            session: None,
            last_tick_end: None,
            gateway_has_answered_ping: false,
            devices_answered_ping: HashSet::new(),
            last_dns: None,
            last_presence: None,
            last_rollup: None,
            watched: HashSet::new(),
            watched_primed: false,
            starred_since: HashMap::new(),
        };
        let status = MonitorStatus {
            state: MonitorState::WaitingForScan,
            network_id: None,
            session_started_at: None,
            last: None,
            checks: Vec::new(),
        };
        Self {
            inner: Arc::new(Inner {
                platform,
                netio,
                store,
                bus,
                scans,
                notifier,
                clock,
                cfg,
                faults: Faults::default(),
                state: tokio::sync::Mutex::new(state),
                status: Mutex::new(status),
            }),
            running: Mutex::new(None),
        }
    }

    /// Spawns the loop and the bus listener. A second call is a no-op.
    pub fn start(&self) {
        let mut running = lock(&self.running);
        if running.is_some() {
            return;
        }
        let (stop, stopped) = watch::channel(false);
        let rx = self.inner.bus.subscribe();
        let ticker = tokio::spawn(Arc::clone(&self.inner).run(stopped));
        let listener = tokio::spawn(Arc::clone(&self.inner).listen(rx));
        *running = Some(Running {
            stop,
            ticker,
            listener,
        });
    }

    pub fn status(&self) -> MonitorStatus {
        lock(&self.inner.status).clone()
    }

    /// Stops the loop (after the tick in progress) and ends any open session
    /// now, for `reason`.
    pub async fn stop(&self, reason: EndReason) {
        let running = lock(&self.running).take();
        if let Some(r) = running {
            let _ = r.stop.send(true);
            r.listener.abort();
            let _ = r.ticker.await;
        }
        let inner = &self.inner;
        let mut st = inner.state.lock().await;
        let network = st.session.as_ref().map(|s| s.network);
        inner
            .end_session(&mut st, inner.clock.now_ms(), reason)
            .await;
        let state = lock(&inner.status).state;
        inner.set_state(state, network, None);
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(r) = lock(&self.running).take() {
            r.ticker.abort();
            r.listener.abort();
        }
    }
}

impl Inner {
    async fn run(self: Arc<Self>, mut stopped: watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(self.cfg.tick);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = stopped.changed() => break,
                _ = interval.tick() => {}
            }
            if AssertUnwindSafe(self.tick()).catch_unwind().await.is_err() {
                tracing::error!("a monitoring tick panicked; the next tick runs as usual");
            }
            self.finish_tick().await;
        }
    }

    /// Stamp the end of a tick, in memory and on the open session. The sleep
    /// check measures from here, so a slow tick never looks like a sleep.
    async fn finish_tick(&self) {
        let mut st = self.state.lock().await;
        let end = self.clock.now_ms();
        st.last_tick_end = Some(end);
        if let Some(id) = st.session.as_ref().map(|s| s.id) {
            let r = self.store.touch_session(id, end).await;
            self.faults.ok("touch session", r);
        }
    }

    /// Publishes `monitor.state` when anything in it changed.
    fn set_state(&self, state: MonitorState, network_id: Option<NetworkId>, started: Option<i64>) {
        let mut status = lock(&self.status);
        if (status.state, status.network_id, status.session_started_at)
            == (state, network_id, started)
        {
            return;
        }
        status.state = state;
        status.network_id = network_id;
        status.session_started_at = started;
        self.bus.publish(EventKind::MonitorStateChanged {
            state,
            network_id,
            session_started_at: started,
        });
    }

    async fn end_session(&self, st: &mut LoopState, at: i64, reason: EndReason) {
        if let Some(s) = st.session.take() {
            let r = self.store.end_session(s.id, at, reason).await;
            self.faults.ok("end session", r);
        }
    }

    /// Not monitoring: end any session as `NetworkLeft`.
    async fn leave(&self, st: &mut LoopState, now: i64, state: MonitorState) {
        self.end_session(st, now, EndReason::NetworkLeft).await;
        self.set_state(state, None, None);
    }

    /// Opens a session on `network`; false when the store refused.
    async fn begin(&self, st: &mut LoopState, network: NetworkId, now: i64) -> bool {
        let opened = self.store.open_session(network, now).await;
        let Some(id) = self.faults.ok("open session", opened) else {
            return false;
        };
        if st.engine_network != Some(network) {
            st.engine = AlertEngine::new(self.cfg.consecutive);
            st.engine_network = Some(network);
            st.watched.clear();
            st.watched_primed = false;
            st.starred_since.clear();
        }
        let keys = self.store.active_alert_keys(network).await;
        // Debounce counts never carry over from an earlier session.
        st.engine.reset_pending();
        st.engine
            .resume(self.faults.ok("active alerts", keys).unwrap_or_default());
        st.loss.clear();
        st.gateway_has_answered_ping = false;
        st.devices_answered_ping.clear();
        st.last_presence = None;
        st.session = Some(Session {
            id,
            network,
            started_at: now,
        });
        self.set_state(MonitorState::Monitoring, Some(network), Some(now));
        true
    }

    async fn tick(&self) {
        let mut guard = self.state.lock().await;
        let st = &mut *guard;
        let now = self.clock.now_ms();

        // 1. Sleep check: the wall-clock gap since the last tick finished.
        if let (Some(s), Some(last)) = (&st.session, st.last_tick_end) {
            if now.saturating_sub(last) > ms(self.cfg.sleep_gap) {
                let network = s.network;
                self.end_session(st, last, EndReason::Sleep).await;
                st.engine.reset_pending();
                st.loss.clear();
                self.set_state(MonitorState::Monitoring, Some(network), None);
            }
        }

        // 2. Which network.
        let Ok(iface) = self.platform.active_interface().await else {
            return self.leave(st, now, MonitorState::NotConnected).await;
        };
        let network = match self.scans.current_network().await {
            Ok(Some(n)) => n,
            Ok(None) => return self.leave(st, now, MonitorState::WaitingForScan).await,
            Err(e) => {
                self.faults.note("current network", &e);
                return self.leave(st, now, MonitorState::NotConnected).await;
            }
        };

        // 3. Network change, 4. new session.
        if st.session.as_ref().is_some_and(|s| s.network != network) {
            self.end_session(st, now, EndReason::NetworkLeft).await;
        }
        if st.session.is_none() && !self.begin(st, network, now).await {
            return;
        }
        let Some(started_at) = st.session.as_ref().map(|s| s.started_at) else {
            return;
        };

        // 5. Probe.
        let listed = self
            .faults
            .ok("list devices", self.store.list_devices(network).await);
        let devices_known = listed.is_some();
        let mut devices = listed.unwrap_or_default();
        let rules = self.store.rule_settings(network).await;
        let rules = self.faults.ok("rule settings", rules).unwrap_or_default();
        let dns_due = due(st.last_dns, now, self.cfg.dns_every);
        if dns_due {
            st.last_dns = Some(now);
        }
        let probes = devices
            .iter()
            .filter(|d| d.starred)
            .filter_map(|d| {
                d.ip.map(|ip| StarredProbe {
                    id: d.id,
                    ip,
                    mac: d.mac,
                    checks: d.checks.clone(),
                })
            })
            .collect();
        let plan = TickPlan {
            at: now,
            iface: &iface,
            devices: probes,
            dns_due,
        };
        let t = sampler::tick(&*self.platform, &*self.netio, &self.cfg, &self.faults, plan).await;

        // 6. Write.
        st.gateway_has_answered_ping |= t.gateway_rtt.is_some();
        if st.gateway_has_answered_ping {
            if let Some(replied) = loss_result(t.gateway_rtt, t.gateway_arp) {
                st.loss.push(Target::Gateway, replied);
            }
        }
        for d in &t.devices {
            if d.rtt.is_some() {
                st.devices_answered_ping.insert(d.id);
            }
            if !st.devices_answered_ping.contains(&d.id) {
                continue;
            }
            if let Some(replied) = loss_result(d.rtt, d.arp) {
                st.loss.push(Target::Device(d.id), replied);
            }
        }
        let samples = conditions::samples(&t, st.gateway_has_answered_ping);
        let r = self.store.insert_samples(network, samples).await;
        self.faults.ok("insert samples", r);

        // 7. Presence of starred devices.
        let ctx = Ctx {
            store: &self.store,
            bus: &self.bus,
            faults: &self.faults,
            cfg: &self.cfg,
            network,
            session_started_at: started_at,
            now,
        };
        let seen: Vec<_> = t
            .devices
            .iter()
            .filter(|d| d.rtt.is_some() || d.arp == Some(true))
            .filter_map(|d| {
                devices
                    .iter()
                    .find(|x| x.id == d.id)?
                    .ip
                    .map(|ip| (d.id, ip))
            })
            .collect();
        refresh::mark_seen(&ctx, &mut devices, &seen).await;
        refresh::update_presence(&ctx, &mut devices, true).await;

        // 8. Presence refresh.
        let mut created = Vec::new();
        if due(st.last_presence, now, self.cfg.presence_every) {
            st.last_presence = Some(now);
            let first = self.store.first_scan_finished_at(network).await;
            let first = self.faults.ok("first scan", first).flatten();
            created =
                refresh::refresh(&ctx, &*self.platform, &iface, &mut devices, &rules, first).await;
        }

        // 9. Readings and decisions.
        let mut decisions = Vec::new();
        if devices_known {
            let starred: HashSet<DeviceId> =
                devices.iter().filter(|d| d.starred).map(|d| d.id).collect();
            if st.watched_primed {
                for id in starred.difference(&st.watched) {
                    st.starred_since.insert(*id, now);
                }
            }
            st.starred_since.retain(|id, _| starred.contains(id));
            st.devices_answered_ping.retain(|id| starred.contains(id));
            decisions = readings::forget_unstarred(
                &mut st.engine,
                &mut st.loss,
                &mut st.watched,
                &devices,
                now,
            );
            st.watched_primed = true;
        }
        let readings = readings::build(
            &t,
            iface.gateway,
            &st.loss,
            &st.engine,
            &devices,
            started_at,
            &st.starred_since,
            &self.cfg,
            created,
        );
        decisions.extend(st.engine.evaluate(now, readings, &rules));
        self.apply(network, decisions, &rules).await;

        // 10. Housekeeping.
        if due(st.last_rollup, now, self.cfg.rollup_every) {
            st.last_rollup = Some(now);
            let r = self.store.rollup(now).await;
            self.faults.ok("rollup", r);
            let r = self.store.prune(now, &self.cfg.retention).await;
            self.faults.ok("prune", r);
        }

        // 11. Publish.
        let tick = MonitorTick {
            ts: now,
            network_id: network,
            gateway_rtt_ms: t.gateway_rtt,
            internet_rtt_ms: t.internet_rtt.or(t.internet_tcp_ms),
            router_lost: st.loss.lost(&Target::Gateway),
            router_window: st.loss.size(),
            dns_ms: t.dns_ms.flatten(),
            wifi_rssi_dbm: t.wifi.as_ref().map(|w| w.rssi_dbm),
        };
        {
            let mut status = lock(&self.status);
            status.last = Some(tick.clone());
            status.checks = t
                .devices
                .into_iter()
                .map(|d| DeviceChecks {
                    device_id: d.id,
                    outcomes: d.checks,
                })
                .collect();
        }
        self.bus.publish(EventKind::MonitorTick { tick });
    }
}
