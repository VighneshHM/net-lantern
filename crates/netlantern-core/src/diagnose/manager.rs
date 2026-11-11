//! The diagnosis runner: checks one layer at a time from the laptop outward,
//! streams each judged layer on the event bus, and stops at the first layer
//! that fails.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use netlantern_platform::{NetIo, Platform};
use tokio::sync::watch;

use super::judge::{
    judge_dns, judge_internet, judge_link, judge_router, judge_website, skipped, verdict,
};
use super::model::{
    DiagnoseConfig, DiagnosisId, DiagnosisRun, Layer, LayerResult, LayerStatus, RunState,
};
use super::probe;
use crate::events::{EventBus, EventKind};
use crate::wifi::WifiService;

/// What a run that panicked reports to the user.
const PANIC_MESSAGE: &str = "Internal error during the diagnosis.";

/// How many runs are kept in memory; older ones are dropped.
const KEPT_RUNS: usize = 8;

/// The layers in the order they are checked.
const LAYERS: [Layer; 5] = [
    Layer::Link,
    Layer::Router,
    Layer::Internet,
    Layer::Dns,
    Layer::Website,
];

/// The result of asking for a diagnosis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnoseStart {
    Started(DiagnosisId),
    /// A diagnosis is already in progress; its id is returned instead.
    AlreadyRunning(DiagnosisId),
}

/// Starts diagnoses and tracks them. Cheap to share: methods take `&self`, and
/// a run is a background task that outlives the call that started it.
pub struct DiagnoseManager {
    inner: Arc<Inner>,
}

struct Inner {
    platform: Arc<dyn Platform>,
    netio: Arc<dyn NetIo>,
    wifi: Arc<WifiService>,
    bus: Arc<EventBus>,
    cfg: DiagnoseConfig,
    state: Mutex<State>,
    /// Bumped whenever a run finishes, to wake [`DiagnoseManager::wait`].
    finished: watch::Sender<u64>,
}

struct State {
    next_id: u64,
    /// The run in progress, if any. At most one runs at a time.
    running: Option<DiagnosisId>,
    /// The most recent runs, oldest first (at most [`KEPT_RUNS`]).
    runs: VecDeque<DiagnosisRun>,
}

impl State {
    fn run_mut(&mut self, id: DiagnosisId) -> Option<&mut DiagnosisRun> {
        self.runs.iter_mut().find(|r| r.id == id)
    }
}

impl DiagnoseManager {
    pub fn new(
        platform: Arc<dyn Platform>,
        netio: Arc<dyn NetIo>,
        wifi: Arc<WifiService>,
        bus: Arc<EventBus>,
        cfg: DiagnoseConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                platform,
                netio,
                wifi,
                bus,
                cfg,
                state: Mutex::new(State {
                    next_id: 1,
                    running: None,
                    runs: VecDeque::new(),
                }),
                finished: watch::Sender::new(0),
            }),
        }
    }

    /// Starts a diagnosis in the background, or reports the one already
    /// running. Never fails: not being connected is a result of the run.
    pub async fn start(&self) -> DiagnoseStart {
        let mut state = lock(&self.inner.state);
        if let Some(id) = state.running {
            return DiagnoseStart::AlreadyRunning(id);
        }
        let id = DiagnosisId(state.next_id);
        state.next_id += 1;
        // Only one run at a time, so every kept run has finished.
        while state.runs.len() >= KEPT_RUNS {
            state.runs.pop_front();
        }
        state.runs.push_back(DiagnosisRun {
            id,
            state: RunState::Running,
            started_at: unix_ms(),
            finished_at: None,
            steps: Vec::new(),
            verdict: None,
            error: None,
        });
        state.running = Some(id);
        drop(state);

        tokio::spawn(Arc::clone(&self.inner).run(id));
        DiagnoseStart::Started(id)
    }

    /// A kept run, with the steps judged so far.
    pub fn get(&self, id: DiagnosisId) -> Option<DiagnosisRun> {
        lock(&self.inner.state)
            .runs
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    /// The most recent run, running or not.
    pub fn latest(&self) -> Option<DiagnosisRun> {
        lock(&self.inner.state).runs.back().cloned()
    }

    /// The run once it is no longer running; `None` for an unknown id.
    pub async fn wait(&self, id: DiagnosisId) -> Option<DiagnosisRun> {
        // Subscribed before looking, so a finish in between is not missed.
        let mut finished = self.inner.finished.subscribe();
        loop {
            let run = self.get(id)?;
            if run.state != RunState::Running {
                return Some(run);
            }
            // The sender lives as long as `self`, so this cannot fail.
            if finished.changed().await.is_err() {
                return self.get(id);
            }
        }
    }
}

impl Inner {
    /// Runs one diagnosis to the end and publishes `diagnose.done`, whatever
    /// happened, including a panic in the platform code.
    async fn run(self: Arc<Self>, id: DiagnosisId) {
        // Whatever happens below, even a panic in the finish itself, the run
        // stops counting as in progress.
        let _release = ReleaseOnDrop(&self, id);
        // The verdict is worked out inside the caught future too, so a panic
        // there still ends in a failed run and a `diagnose.done`.
        let outcome = AssertUnwindSafe(async {
            self.diagnose(id).await;
            let steps = self.steps(id);
            verdict(&steps)
        })
        .catch_unwind()
        .await;
        if outcome.is_err() {
            tracing::warn!(run = id.0, error = PANIC_MESSAGE, "diagnosis panicked");
        }

        // Finish in one critical section: the run turns final, `diagnose.done`
        // goes out and the guard is freed together, so nobody is told a
        // finished run is running.
        let mut state = lock(&self.state);
        #[cfg(test)]
        if tests::PANIC_IN_FINISH.with(std::cell::Cell::get) {
            panic!("injected fault: the finish section panicked");
        }
        let done = state.run_mut(id).map(|run| {
            run.finished_at = Some(unix_ms());
            match outcome {
                Ok(v) => {
                    run.state = RunState::Done;
                    run.verdict = Some(v);
                }
                Err(_) => {
                    run.state = RunState::Failed;
                    run.error = Some(PANIC_MESSAGE.to_string());
                }
            }
            EventKind::DiagnoseDone {
                run_id: id,
                state: run.state,
                verdict: run.verdict.clone(),
                error: run.error.clone(),
            }
        });
        if let Some(done) = done {
            self.bus.publish(done);
        }
        state.running = None;
        drop(state);
        self.finished.send_modify(|n| *n += 1);
    }

    /// The steps recorded so far for run `id`.
    fn steps(&self, id: DiagnosisId) -> Vec<LayerResult> {
        lock(&self.state)
            .run_mut(id)
            .map(|run| run.steps.clone())
            .unwrap_or_default()
    }

    /// Checks each layer in turn. Returns once all five steps are recorded.
    async fn diagnose(&self, id: DiagnosisId) {
        let cfg = &self.cfg;

        let progress = probe::link_progress();
        self.within(probe::fill_link(
            &progress,
            &*self.platform,
            &self.wifi,
            cfg,
        ))
        .await;
        let ((link, iface, radio_scanned), late) = progress.into_parts();
        if !self.record(id, judge_link(&link, cfg), late) {
            return;
        }
        // A link that didn't fail has an interface.
        let Some(iface) = iface else {
            self.skip_from(id, 1);
            return;
        };
        // A radio scan just ran: let the radio get back to the network before
        // the router pings, or its scan time can read as packet loss.
        if radio_scanned {
            tokio::time::sleep(cfg.post_scan_settle).await;
        }

        let progress = probe::router_progress(iface.gateway, cfg);
        self.within(probe::fill_router(
            &progress,
            &*self.platform,
            iface.gateway,
            cfg,
        ))
        .await;
        let (router, late) = progress.into_parts();
        if !self.record(id, judge_router(&router, cfg), late) {
            return;
        }

        let progress = probe::internet_progress(cfg);
        self.within(probe::fill_internet(
            &progress,
            &*self.platform,
            &*self.netio,
            cfg,
        ))
        .await;
        let (internet, late) = progress.into_parts();
        if !self.record(id, judge_internet(&internet, cfg), late) {
            return;
        }

        let progress = probe::dns_progress(&iface.dns_servers, cfg);
        self.within(probe::fill_dns(
            &progress,
            &*self.netio,
            &iface.dns_servers,
            cfg,
        ))
        .await;
        let (dns, late) = progress.into_parts();
        if !self.record(id, judge_dns(&dns), late) {
            return;
        }

        let progress = probe::website_progress(cfg);
        self.within(probe::fill_website(&progress, &*self.netio, cfg))
            .await;
        let (website, late) = progress.into_parts();
        self.record(id, judge_website(&website), late);
    }

    /// Runs a layer's `fill` under the per-layer time limit. What it left in
    /// its progress is then the probe, with unfinished checks failed.
    async fn within(&self, fill: impl std::future::Future<Output = ()>) {
        if tokio::time::timeout(self.cfg.layer_timeout, fill)
            .await
            .is_err()
        {
            tracing::warn!("a diagnosis layer hit its time limit");
        }
    }

    /// Appends a judged step and publishes it. On a failure the remaining
    /// layers are recorded as skipped; returns whether to go on.
    fn record(&self, id: DiagnosisId, mut step: LayerResult, timed_out: Vec<String>) -> bool {
        step.evidence.extend(timed_out);
        let failed = step.status == LayerStatus::Fail;
        let next = LAYERS
            .iter()
            .position(|&l| l == step.layer)
            .map_or(LAYERS.len(), |i| i + 1);
        self.append(id, step);
        if failed {
            self.skip_from(id, next);
        }
        !failed
    }

    /// Records every layer from index `first` on as skipped.
    fn skip_from(&self, id: DiagnosisId, first: usize) {
        for &layer in &LAYERS[first.min(LAYERS.len())..] {
            self.append(id, skipped(layer));
        }
    }

    /// Adds `step` to the run and publishes it under one lock, so a reader
    /// who hears of a step finds it in the run.
    fn append(&self, id: DiagnosisId, step: LayerResult) {
        let mut state = lock(&self.state);
        if let Some(run) = state.run_mut(id) {
            run.steps.push(step.clone());
        }
        self.bus
            .publish(EventKind::DiagnoseStep { run_id: id, step });
    }
}

/// Clears the running slot if it still holds run `.1` and wakes waiters when
/// dropped, so a panic while finishing can't block every later run. A slot
/// already taken by a newer run is left alone.
struct ReleaseOnDrop<'a>(&'a Inner, DiagnosisId);

impl Drop for ReleaseOnDrop<'_> {
    fn drop(&mut self) {
        let mut state = lock(&self.0.state);
        if state.running == Some(self.1) {
            state.running = None;
            // Still running means the finish section never completed: fail
            // the run and say so, so `wait()` and the event stream end.
            let done = state
                .run_mut(self.1)
                .filter(|run| run.state == RunState::Running)
                .map(|run| {
                    run.state = RunState::Failed;
                    run.error = Some(PANIC_MESSAGE.to_string());
                    run.finished_at = Some(unix_ms());
                    EventKind::DiagnoseDone {
                        run_id: self.1,
                        state: run.state,
                        verdict: None,
                        error: run.error.clone(),
                    }
                });
            if let Some(done) = done {
                self.0.bus.publish(done);
            }
            drop(state);
            self.0.finished.send_modify(|n| *n += 1);
        }
    }
}

/// Milliseconds since the Unix epoch.
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Locks `mutex`. The guarded state is plain data that is never left half
/// updated, so a poisoned lock is safe to keep using.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::Path;

    use netlantern_platform::SimNetwork;

    use super::*;
    use crate::wifi::CongestionConfig;

    thread_local! {
        /// Makes the finish section of runs on this thread panic.
        pub(super) static PANIC_IN_FINISH: Cell<bool> = const { Cell::new(false) };
    }

    #[tokio::test(start_paused = true)]
    async fn a_panic_after_the_steps_still_finishes_the_run() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios/home.json");
        let sim = Arc::new(SimNetwork::load(path).unwrap());
        let bus = Arc::new(EventBus::new(256));
        let wifi = Arc::new(WifiService::new(sim.clone(), CongestionConfig::default()));
        let mgr = DiagnoseManager::new(
            sim.clone(),
            sim,
            wifi,
            Arc::clone(&bus),
            DiagnoseConfig::default(),
        );
        let mut rx = bus.subscribe();

        PANIC_IN_FINISH.with(|f| f.set(true));
        let id = match mgr.start().await {
            DiagnoseStart::Started(id) => id,
            other => panic!("expected a new run, got {other:?}"),
        };
        let r = mgr.wait(id).await.unwrap();
        PANIC_IN_FINISH.with(|f| f.set(false));

        assert_eq!(r.state, RunState::Failed);
        assert_eq!(r.error.as_deref(), Some(PANIC_MESSAGE));
        assert!(r.finished_at.is_some());
        let mut dones = 0;
        while let Ok(ev) = rx.try_recv() {
            if let EventKind::DiagnoseDone { state, .. } = ev.kind {
                assert_eq!(state, RunState::Failed);
                dones += 1;
            }
        }
        assert_eq!(dones, 1);
        assert!(matches!(mgr.start().await, DiagnoseStart::Started(_)));
    }
}
