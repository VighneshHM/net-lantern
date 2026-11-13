# Net Lantern Phase 3: Monitoring and Alerts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Net Lantern keeps watching the network after a scan. It samples the router, the internet and starred devices every 30 s and stores the history with rollups and retention. It tracks which devices are present, with extra grace for sleeping phones. It raises alerts that are debounced, grouped into incidents and held back while the router is down, and it sends a desktop notification. The Dashboard shows live tiles and a latency graph, and the Alerts screen shows incidents with their timelines.

**Architecture:**
- **Spikes first** (spec §12, §16):
  - a Windows toast from the unpackaged exe, which decides how notifications work in this phase;
  - a minimal Tauri v2 app that hosts the server library in-process, the starting point for phase 7.
- **`netlantern-platform`:** scenarios gain a **timeline** (devices that drop out, come back, join, or a router that goes down) and a time scale, so monitoring can be tested in the simulator.
- **`netlantern-core`:**
  - **Schema v2:** samples, rollups, monitoring sessions, presence, checks, alerts, incidents, and incident events.
  - **`monitor` module, pure parts:**
    - `conditions` turns the raw results of each check into failing/ok readings per alert key;
    - `presence` decides online/offline with grace;
    - `rules` is the alert engine: the lifecycle, the debounce, and suppression while the router is down.
  - **`monitor` module, the `Monitor` task:** runs the schedule, writes samples, applies the engine's decisions to the store, publishes events and calls a `Notifier`.
- **Server:** adds the monitor, metrics, alerts, incidents and rules routes, and checks on devices.
- **Exe:** starts the monitor, ends the session on quit, and uses the spike's notifier.
- **UI:** the Dashboard gets tiles, a uPlot latency graph, starred devices and recent alerts. The Alerts screen gets incidents, rules and notification settings. Device details gain a checks editor.

**Tech Stack:** Phase 2's stack plus:
- `tauri-winrt-notification` 0.7 (Windows toasts; the spike decides how it is used);
- `uplot` 1.6.32 (the latency graph, spec §9);
- Tauri v2 (`tauri` 2, `tauri-build` 2), in the spike only, outside the cargo workspace and outside CI.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`. Relevant sections:
- the Phase 3 row of §12;
- §2.1 (alert latency, false alerts, footprint, storage);
- §6.5 (starring, checks), §6.8, §6.9 (rules, incidents, suppression, lifecycle diagram) and §6.12 (Monitoring, Alerts, Retention rows);
- §7 (tables), §8 (routes, events), §9 (Dashboard, Alerts), journeys A and B in §9.1;
- §10 (sleep, crashes), §11 (the scenario list), the Phase 3 learning goals in §13, and §16 (toast and Tauri risks).

Earlier plans: `docs/superpowers/plans/2026-11-07-phase-2-diagnosis-wifi.md` and the two before it.

**Branch:** `phase-3-monitor-alerts`, cut from `phase-2-diagnosis-wifi` (a3ffd17). Phases 0–2 are not merged yet, so it stacks on them.

## Global Constraints

- **Windows 11, no admin rights for anything.** CI runs on `windows-latest`. Never parse the text output of OS tools.
- **IPv4 only.**
- **Outside traffic, now scheduled.** While monitoring runs, Net Lantern contacts the internet as follows. Each item is new to the README list from Phase 2:
  - ICMP echo to `1.1.1.1` every 30 s;
  - when that ping gets no reply, a TCP connect to `1.1.1.1:443`, closed straight away;
  - a DNS A query for `www.microsoft.com` to the first system DNS server every 60 s.

  Nothing else leaves the LAN on a schedule. Starred-device checks only contact the device's own IP. No scan data, device data or identifiers are sent.
- **The scheduler never calls `wifi_scan`.** Wi-Fi samples come from `wifi_current` only (spec §6.7, §6.8).
- **Monitoring defaults.** Spec §6.12 values are kept. Values new in this plan are recorded in §6.12 in Task 15:

  | Setting | Value | New? |
  |---|---|---|
  | Tick (gateway + 1.1.1.1 ping, starred checks, Wi-Fi) | 30 s | no |
  | DNS check | every 60 s | no |
  | Presence refresh | every 5 min | no |
  | Offline grace | 2 min; phones and tablets 10 min | 2 min for others is new (from the §6.9 rule) |
  | Monitor ping timeout | 1 s | new |
  | TCP fallback to 1.1.1.1:443 | timeout 2 s | new |
  | ARP resolve in monitoring | timeout 3 s | new |
  | Presence refresh concurrency | 16 pings, 8 ARP resolves | new |
  | Debounce | 2 consecutive failing ticks (router down, internet down, latency) | no |
  | Latency high | > 100 ms on 2 consecutive ticks | no |
  | Packet loss | ≥ 3 of the last 30 pings | no |
  | Sleep gap | a wall-clock gap > 90 s between ticks ends the session as `sleep` | new |
  | Rollup and prune | every 5 min | new |
  | Retention | raw 24 h, rollups 30 days, presence 90 days | no |
  | Checks per starred device | at most 5 | new |

- **Targets (§2.1), checked at the checkpoint:**
  - a starred device going offline raises an alert within **2.5 min**;
  - no alerts from a sleeping phone over **24 h**;
  - **≤ 1** false alert per day;
  - the server process uses **< 1%** average CPU and **< 50 MB** RAM;
  - the database stays **< 50 MB** after 30 days with 10 starred devices.
- **Never log tokens or session ids.** Text taken from the network is cut to 100 characters.
- **API:**
  - `API_VERSION` stays **1**. New routes, fields and events are additions.
  - Every error body is `{"code","message","hint"}`.
  - Timestamps are milliseconds since the Unix epoch.
  - JSON enums are `snake_case`.
- **UI:** status is always shown as text, touch targets are ≥ 44 px, it works at phone width with no horizontal page scroll, and colours come only from the tokens.
- **Never change Windows settings for the author.** Notification and location settings are explained in the UI, not changed.
- **Commit messages are one plain sentence.** No prefix, no body, no `Co-Authored-By` and no Claude or Anthropic attribution.

## Review Focus

1. **A sleeping starred phone.** It answers ARP but not ping, and now and then answers nothing for up to 8 minutes. Expected:
   - no "offline" alert;
   - no packet-loss alert, because the loss rule skips phones and tablets;
   - the phone stays online, because ARP counts as presence.

   Tested in Task 9 (`sleeping-phone`, 24 h simulated).
2. **The router ignores ping** (`router-silent`). Expected:
   - no router-down alert while it answers ARP;
   - no packet-loss alert on the router, because a gateway that has answered no ping this session records no loss samples.

   Tested in Task 6 (conditions) and Task 9.
3. **The network blocks ICMP to the internet** (`icmp-blocked`). Expected: no internet-down alert, because the TCP connect to 1.1.1.1:443 counts as a reply. Tested in Task 6 and Task 9.
4. **The laptop wakes after hours asleep, or comes back to a network.** Expected:
   - the old session ended as `sleep` or `network_left`;
   - presence grace counts from the new session's start, so no wave of "offline" alerts fires for devices last seen before the sleep;
   - the graph shows a gap, not an outage.

   Tested in Task 6 (presence) and Task 9.
5. **Restarts and first scans.** Expected:
   - alerts open before a restart are resumed, not duplicated;
   - a crashed session is closed as `crash`;
   - the first scan of a network raises no "new device" alerts;
   - a device seen for the first time after that scan raises exactly one.

   Tested in Task 5, Task 8 and Task 9.

## Not in this phase

- **The Settings screen, `GET/PUT /api/settings`, and editable intervals or retention.** Phase 6. In this phase, rules and the notifications switch live under `/api/rules`.
- **Tray, start at login, and Tauri notifications.** Phase 7. The Tauri spike proves hosting only.
- **Log files and copy debug info.** Phase 7. Panics keep going to stderr through `tracing`.
- **Alerts for failed TCP or HTTP checks.** Their status is shown on the device; spec §6.9 has no rule for them.
- **Playwright tests for journeys A and B.** Phase 7 (spec §12, "journeys A–C pass end to end"). They are covered here by the simulator tests in Task 9 and Task 10.
- **Hop-by-hop or per-device bandwidth.** These are non-goals.

---

### Task 1: Phase 2 leftovers and the real-network results

**Files:**
- Modify:
  - `crates/netlantern-core/src/wifi.rs` (`WifiService::status`)
  - `crates/netlantern-core/src/diagnose/judge.rs` (link evidence near line 104)
  - `crates/netlantern-core/src/diagnose/manager.rs` (`ReleaseOnDrop`)
  - `ui/src/api/events.ts` (`followSnapshot`)
  - `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§2.1 note, §16 location row)
- Test:
  - `crates/netlantern-core/tests/wifi_sim.rs`
  - `crates/netlantern-core/src/diagnose/judge.rs` (unit)
  - `crates/netlantern-core/tests/diagnose_sim.rs`
  - `ui/src/api/events.test.ts`

**Interfaces:** No new names; behaviour fixes only (the Phase 2 ledger's four parked residuals).

**Behaviour:**
1. **`status()` on a Wi-Fi interface.**
   - `Err(Unavailable)` from `wifi_current` becomes `connection: None` only when the interface kind is **not** `Wifi`.
   - On a Wi-Fi interface the error is returned, so the API answers 503 `wifi_unavailable` instead of "Not connected to Wi-Fi."
2. **Link evidence.** For `WifiReading::Unavailable(reason)` the evidence is `Signal details unavailable ({reason})`. Here `reason` is the inner text: `PlatformError::Unavailable { reason }`'s `reason`, not its `Display`, which starts with "unavailable: ". The prefix is stripped where the reading is built.
3. **A panic in the finish section.** If `ReleaseOnDrop` runs while the run is still `Running`, it:
   - marks the run `Failed` with `Internal error during the diagnosis.`;
   - publishes `DiagnoseDone`;
   - bumps `finished`.

   So `wait()` returns.
4. **`followSnapshot`.** It merges a snapshot only when no run is followed or the snapshot's `id` is ≥ the followed run's `id`. An older run's late snapshot is ignored.

- [ ] **Step 1: Write the failing tests**

```rust
// wifi_sim.rs
#[tokio::test] async fn unavailable_wifi_on_a_wifi_interface_is_an_error() { /* stub platform: kind Wifi, wifi_current Err(Unavailable) → status() is Err(CoreError::Platform(Unavailable{..})) */ }
#[tokio::test] async fn unavailable_wifi_on_ethernet_is_no_connection() { /* kind Ethernet → Ok(WifiStatus{interface_kind: Some(Ethernet), connection: None}) */ }
// judge.rs
#[test] fn unavailable_evidence_has_no_doubled_prefix() { /* Unavailable("radio off") → evidence == ["Signal details unavailable (radio off)"] */ }
// diagnose_sim.rs
#[tokio::test(start_paused = true)] async fn a_panic_after_the_steps_still_finishes_the_run() { /* inject a panic in the finish path via a test hook (cfg(test) fn) → wait() returns Some(run) with state Failed and one DiagnoseDone */ }
```
```ts
// events.test.ts
it("ignores a late snapshot of an older run", async () => { /* follow run 5; resync returns run 4 running → store still shows run 5 */ });
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core` and `cd ui && npm test -- events`. Expected: the new tests FAIL.
- [ ] **Step 3:** Make the four fixes. For item 3, add a `#[cfg(test)]`-only fault hook in `manager.rs` that panics inside the finish section; production code has no hook.
- [ ] **Step 4:** Record the Phase 2 real-network results in the spec:
  - **§2.1:** add one line under the table: "Phase 2, author's home Wi-Fi (2026-11-09): with no WAN cable in the router, the verdict was 'Your internet connection is down.'; with DNS set to an unused address, it was 'Your DNS server isn't working.' Both correct."
  - **§16:** replace the location-rule note under the table with this matrix:

    | Location services | Let desktop apps access your location | Wi-Fi details |
    |---|---|---|
    | Off | — | hidden |
    | On | Off | hidden |
    | On | On | shown |

    Add: "The unpackaged exe needs both switches on. Diagnosis still passes with signal details hidden."
- [ ] **Step 5:** Run `cargo test --workspace` and `cd ui && npm test`. Expected: PASS.
- [ ] **Step 6: Commit:** `Fix the four Phase 2 leftovers and record the real-network diagnosis and location results in the spec.`

---

### Task 2: Spike: a Windows toast from the unpackaged exe

**Files:**
- Create: `crates/netlantern/examples/toast_spike.rs`
- Modify:
  - `crates/netlantern/Cargo.toml`: `[target.'cfg(windows)'.dependencies] tauri-winrt-notification = "0.7"`, plus the `windows` features `Win32_System_Registry` if the register variant needs them.
  - `docs/superpowers/specs/2026-10-31-netlantern-design.md` §16 (toast row outcome).

**Interfaces:**
- Produces: a recorded outcome, `A`, `B` or `C`, that Task 12 implements:
  - **A.** A custom AppUserModelID `NetLantern.NetLantern` works once it is registered under `HKCU\Software\Classes\AppUserModelId\NetLantern.NetLantern`, with the string value `DisplayName = "Net Lantern"`.
  - **B.** Only `Toast::POWERSHELL_APP_ID` works. The toast is attributed to Windows PowerShell.
  - **C.** Nothing shows a toast.

**Steps:**
- [ ] **Step 1:** Write `toast_spike.rs`. It takes one of these arguments:
  - `--powershell`: shows a toast with title "Net Lantern test" and body "If you can read this, toasts work." using `Toast::POWERSHELL_APP_ID`.
  - `--app-id`: shows the same toast with the custom id, without registering anything.
  - `--register`: writes the HKCU key above.
  - `--unregister`: deletes that key.

  It prints what it did and any error, and nothing else.
- [ ] **Step 2:** Build it with `cargo build -p netlantern --example toast_spike`, then run `--powershell` and `--app-id`. Ask the author whether each toast appeared on screen.
- [ ] **Step 3:** Only with the author's yes in chat, run `--register`, then `--app-id` again. Ask whether it appeared, with "Net Lantern" as the sender. Run `--unregister` afterwards, whatever the result.
- [ ] **Step 4:** In the spec §16 toast row, record the outcome letter and what was seen, with the date. Also record the decision:
  - **A:** the exe registers the key at startup (idempotent, per-user, its own key only; the phase 7 uninstaller removes it).
  - **B:** use the PowerShell id, with titles prefixed "Net Lantern: ".
  - **C:** dashboard alerts only until the Tauri plugin.
- [ ] **Step 5: Commit:** `Add a toast spike for the unpackaged exe and record which notification route works.`

---

### Task 3: Spike: a minimal Tauri app hosting the server library

**Files:**
- Create, under `apps/desktop/src-tauri/`:
  - `Cargo.toml`. It has its own empty `[workspace]` table, so it stays out of the cargo workspace and CI. Dependencies: `tauri = "2"`, `tauri-build = "2"`, `tokio`, `netlantern-server`, `netlantern-core` and `netlantern-platform` by path.
  - `build.rs`, `tauri.conf.json`, `src/main.rs`.
  - `icons/icon.ico`: any valid placeholder icon; phase 7 replaces it.
- Modify:
  - `.gitignore`: add `apps/desktop/src-tauri/target/` and `apps/desktop/src-tauri/gen/`.
  - Spec §16 (Tauri row outcome).

**Interfaces:**
- Consumes: from `netlantern_server`: `bind`, `run`, `AppState::new`, `AccessPolicy::for_port`, `Auth::issue_launch_token` and `DEFAULT_PORTS`. Also `SimNetwork::from_json` on `scenarios/home.json` (`include_str!`) and `Db::open_in_memory`.
- Produces: a recorded outcome only.

**Behaviour:**
- On setup, start a tokio runtime thread and bind the port. Spawn `run(listener, state, shutdown)`, where shutdown fires on a oneshot sent when the app exits.
- Create one `WebviewWindow` "main", 1200×800, titled "Net Lantern", at `WebviewUrl::External("http://127.0.0.1:{port}/?t={token}")`.
- `frontendDist` in `tauri.conf.json` points at `../../../ui/dist`. It is unused, but `tauri-build` requires it.

**Steps:**
- [ ] **Step 1:** Write the app. Build the UI first with `cd ui && npm run build`, then `cd apps/desktop/src-tauri && cargo build --release`.
- [ ] **Step 2:** Run it. Check, with the author watching:
  - the window shows the dashboard, logged in;
  - a quick scan of the simulated network works;
  - closing the window ends the process within 2 s.

  Read the RAM with the window open (Task Manager, or `Get-Process`) and the release binary size.
- [ ] **Step 3:** Record in the spec §16 Tauri row: whether it works, the RAM, the binary size, the build time, and any surprise (port, origin, cookie). If the guard refuses the webview's origin, record that too; the fix belongs in phase 7.
- [ ] **Step 4: Commit:** `Add a minimal Tauri spike that hosts the server in-process and record what it showed.`

---

### Task 4: Scenario timelines and time scale in the simulator

**Files:**
- Modify: `crates/netlantern-platform/src/sim/{mod.rs,scenario.rs}`
- Create, in `scenarios/` (built on `home.json`'s interface, gateway, `internet`, `dns`, `http` and `wifi` blocks):
  - `printer-offline.json`
  - `sleeping-phone.json`
  - `new-device.json`
  - `router-outage.json`
- Test: `crates/netlantern-platform/tests/sim.rs`

**Interfaces:**
- Consumes: Phase 2's `SimNetwork`.
- Produces:
  - Scenario JSON gains `"timeline": [TimelineEntry]` (default empty):
    - `TimelineEntry { ip: Ipv4Addr, from_s: u64, until_s: Option<u64>, every_s: Option<u64>, set: Override }`
    - `Override { present: Option<bool>, ping_ms: Option<Option<u64>>, arp: Option<bool> }`. JSON `"ping_ms": null` means "stop answering ping"; an absent field means "unchanged".
  - An entry is active at elapsed second `t` when:
    - `from_s ≤ t < until_s` (with `until_s` absent meaning forever);
    - with `every_s` set, the window repeats: active when `(t - from_s) % every_s < (until_s - from_s)`, for `t ≥ from_s`.

    Later entries override earlier ones, field by field.
  - `present: false` makes a device absent everywhere: ping, ARP resolve, the ARP table, TCP, mDNS, SSDP, NetBIOS and reverse DNS. A device whose only entry is `{from_s: 600, set: {present: true}}` and whose base record says `"present": false` "joins" at 600 s. So `ScenarioDevice` gains `present: bool`, default `true`.
  - **When the gateway is absent** (`present` false, or `arp` false with `ping_ms` null), every outside host stops answering, `dns_resolve` times out, and `http_get` fails with `Io("no route to the internet")`.
  - `SimNetwork::with_time_scale(self, factor: u32) -> SimNetwork`. Elapsed scenario seconds = real elapsed × factor. Default 1.
  - Elapsed time is measured from construction with `tokio::time::Instant`, so paused-clock tests control it.

**Scenarios:**

| File | Devices beyond the gateway and laptop | Timeline |
|---|---|---|
| `printer-offline.json` | `192.168.1.40`, an HP printer: MAC with an HP OUI, `ping_ms` 5, `open_ports` [80, 631], mDNS `_ipp._tcp`, instance "Office printer" | `.40` `present: false` from 600 to 1200 |
| `sleeping-phone.json` | `192.168.1.57`, the iPhone from `home.json` (classified `phone`), `ping_ms: null`, `arp: true` | `.57` `{present: false}`: `from_s` 3000, `until_s` 3480, `every_s` 3600 (8 min every hour) |
| `new-device.json` | `192.168.1.77`, MAC with an Espressif OUI, `present: false` | `.77` `{present: true}` from 600 |
| `router-outage.json` | the printer from `printer-offline.json` | gateway `{present: false}` from 600 to 900; printer `{present: false}` from 600 to 1500 |

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(start_paused = true)] async fn a_timeline_entry_takes_a_device_away_and_back() { /* printer-offline: ping .40 Some at 0; advance 600 s → None and arp_table lacks .40; advance to 1200 → Some again */ }
#[tokio::test(start_paused = true)] async fn repeating_windows_repeat() { /* sleeping-phone: arp_resolve .57 Some at 2999 s, None at 3000 and 3479, Some at 3480, None at 6600 */ }
#[tokio::test(start_paused = true)] async fn a_device_can_join_later() { /* new-device: arp_table lacks .77 before 600 s, has it after */ }
#[tokio::test(start_paused = true)] async fn an_absent_gateway_takes_the_internet_with_it() { /* router-outage at 700 s: ping 1.1.1.1 None, tcp 1.1.1.1:443 Filtered, dns_resolve Err, http_get Err */ }
#[tokio::test(start_paused = true)] async fn time_scale_speeds_up_the_timeline() { /* printer-offline with_time_scale(60): after 10 real s, .40 is absent */ }
#[test] fn every_scenario_file_loads() { /* extend the existing test with the four new files */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-platform --test sim`. Expected: FAIL.
- [ ] **Step 3:** Implement the timeline as a pure `fn effective(&self, ip, elapsed_s) -> Option<EffectiveDevice>`, used by every `Platform` and `NetIo` method. Write the scenario files.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS. Scenarios without a timeline behave exactly as before.
- [ ] **Step 5: Commit:** `Let simulated scenarios change over time so devices can drop out, return or join and the router can go down.`

---

### Task 5: Schema v2, monitor config, clock, samples, rollups, sessions and retention

**Files:**
- Create:
  - `crates/netlantern-core/src/monitor/{mod.rs,config.rs,clock.rs,model.rs}` (types only in this task)
  - `crates/netlantern-core/src/store/{samples.rs,sessions.rs}`
- Modify:
  - `crates/netlantern-core/src/store/{migrations.rs,db.rs,mod.rs}`
  - `crates/netlantern-core/src/lib.rs`
- Test:
  - `crates/netlantern-core/tests/store.rs` (extend)
  - `crates/netlantern-core/tests/storage_budget.rs` (new)

**Interfaces:**
- Produces, re-exported from `netlantern_core`:
  - `struct MonitorConfig`:
    - fields: `tick`, `dns_every`, `presence_every`, `rollup_every`, `ping_timeout`, `tcp_fallback_timeout`, `arp_timeout` (all `Duration`); `internet_target: Ipv4Addr`; `dns_name: String`; `offline_grace`, `offline_grace_sleepy` (`Duration`); `latency_high_ms: f64`; `consecutive: u32`; `loss_window: u32`; `loss_threshold: u32`; `sleep_gap: Duration`; `retention: Retention`; `presence_ping_concurrency: usize`; `presence_arp_concurrency: usize`; `max_checks: usize`.
    - `Default` gives the Global Constraints values.
    - `fn scaled(&self, factor: u32) -> MonitorConfig` divides every `Duration` by `factor` (minimum 10 ms) except `retention`.
  - `struct Retention { raw: Duration /*24 h*/, rollups: Duration /*30 d*/, presence: Duration /*90 d*/ }`
  - `trait Clock: Send + Sync { fn now_ms(&self) -> i64; }`
    - `struct SystemClock`;
    - `struct TokioClock { epoch_ms: i64, start: tokio::time::Instant }` with `TokioClock::new(epoch_ms)`, where `now_ms` = `epoch_ms` + elapsed. Used by every monitor test.
  - `enum Target { Gateway, Internet, Dns, Wifi, Device(DeviceId) }`:
    - serialized as the strings `"gateway"`, `"internet"`, `"dns"`, `"wifi"` and `"device:<id>"`;
    - `Display` and `FromStr` match the serialized form.
  - `enum Metric { RttMs, LookupMs, RssiDbm, LinkRateMbps }`, snake_case.
  - `struct Sample { target: Target, metric: Metric, ts: i64, value: Option<f64> }`. For `RttMs` and `LookupMs`, `None` means no reply. Other metrics never store `None`.
  - `struct SessionId(pub i64)`, transparent.
  - `enum EndReason { Sleep, NetworkLeft, Quit, Crash }`, snake_case.
  - `struct SessionSpan { started_at: i64, ended_at: Option<i64>, end_reason: Option<EndReason> }`
  - `struct MetricPoint { ts: i64, min: Option<f64>, avg: Option<f64>, max: Option<f64>, count: u32, lost: u32 }`
  - `enum Resolution { Raw, FiveMin }`
  - `struct MetricSeries { resolution: Resolution, points: Vec<MetricPoint>, sessions: Vec<SessionSpan> }`
  - **`enum Check`**, `#[serde(tag = "kind", rename_all = "snake_case")]`:
    - `Tcp { port: u16 }`
    - `Http { port: u16, path: String, https: bool }`

    Ping is implicit for every starred device and never stored. Task 7 stores checks; Task 6 and Task 9 use them.
  - **`struct CheckOutcome { check: Check, ok: bool, at: i64, detail: String }`**. `detail` is "open", "no answer", "HTTP {status}", or "error: {msg ≤100}".
  - All derive `Clone, Debug, PartialEq, Serialize, Deserialize`.
- `Store` methods:
  - `insert_samples(&self, network: NetworkId, samples: Vec<Sample>) -> Result<(), StoreError>`
  - `rollup(&self, now: i64) -> Result<(), StoreError>`. Folds raw samples of every finished 5-minute bucket (`bucket_start = ts - ts % 300_000`, with `bucket_start + 300_000 ≤ now`) into `samples_rollup`. It uses `INSERT OR REPLACE`, so running it twice is harmless. `lost` = rows with a NULL value; `min`, `avg` and `max` are over non-NULL values.
  - `prune(&self, now: i64, r: &Retention) -> Result<(), StoreError>`. Deletes raw samples older than `r.raw`, rollups older than `r.rollups`, presence rows older than `r.presence`, and ended sessions older than `r.rollups`.
  - `metrics(&self, network, target: Target, metric: Metric, from: i64, to: i64) -> Result<MetricSeries, StoreError>`:
    - when `to - from ≤ 24 h`: `Raw` resolution, one point per sample (`min = avg = max = value`, `count` 1, `lost` 1 when NULL);
    - otherwise: `FiveMin` from rollups;
    - in both cases, `sessions` lists every session overlapping `[from, to]`, oldest first.
  - `open_session(&self, network, now) -> Result<SessionId, StoreError>`
  - `touch_session(&self, id, now)`: sets `last_tick_at`.
  - `end_session(&self, id, at: i64, reason: EndReason)`
  - `pub(crate) fn end_dangling_sessions(conn)`: called from `Db::start` after `fail_interrupted_scans`. It sets `ended_at = last_tick_at` and `end_reason = 'crash'` where `ended_at IS NULL`.

**Schema v2** (appended to `MIGRATIONS`; v1 files upgrade with the existing backup):

```sql
ALTER TABLE devices ADD COLUMN presence TEXT;               -- 'online' | 'offline' | NULL = unknown (falls back to the scan rule)
ALTER TABLE devices ADD COLUMN presence_changed_at INTEGER;
CREATE TABLE presence(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, ts INTEGER NOT NULL, online INTEGER NOT NULL);
CREATE INDEX presence_device ON presence(device_id, ts);
CREATE TABLE monitor_sessions(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id),
    started_at INTEGER NOT NULL, last_tick_at INTEGER NOT NULL, ended_at INTEGER, end_reason TEXT);
CREATE INDEX monitor_sessions_network ON monitor_sessions(network_id, started_at);
CREATE TABLE samples(network_id INTEGER NOT NULL, target TEXT NOT NULL, metric TEXT NOT NULL, ts INTEGER NOT NULL, value REAL);
CREATE INDEX samples_lookup ON samples(network_id, target, metric, ts);
CREATE TABLE samples_rollup(network_id INTEGER NOT NULL, target TEXT NOT NULL, metric TEXT NOT NULL, bucket_start INTEGER NOT NULL,
    min REAL, avg REAL, max REAL, count INTEGER NOT NULL, lost INTEGER NOT NULL,
    PRIMARY KEY(network_id, target, metric, bucket_start));
CREATE TABLE checks(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, position INTEGER NOT NULL,
    kind TEXT NOT NULL, params TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, PRIMARY KEY(device_id, position));
CREATE TABLE incidents(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id), root_alert_id INTEGER,
    opened_at INTEGER NOT NULL, resolved_at INTEGER);
CREATE TABLE alerts(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id),
    incident_id INTEGER NOT NULL REFERENCES incidents(id), rule TEXT NOT NULL, key TEXT NOT NULL, severity TEXT NOT NULL,
    title TEXT NOT NULL, detail TEXT NOT NULL, suppressed INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL,
    opened_at INTEGER NOT NULL, acked_at INTEGER, resolved_at INTEGER);
CREATE INDEX alerts_network ON alerts(network_id, state, id);
CREATE UNIQUE INDEX alerts_one_active ON alerts(network_id, key) WHERE state != 'resolved';
CREATE TABLE incident_events(incident_id INTEGER NOT NULL REFERENCES incidents(id), alert_id INTEGER,
    ts INTEGER NOT NULL, kind TEXT NOT NULL, text TEXT NOT NULL);
CREATE INDEX incident_events_incident ON incident_events(incident_id, ts);
```

- [ ] **Step 1: Write the failing tests**

```rust
// store.rs
#[tokio::test] async fn a_v1_database_upgrades_to_v2_with_a_backup() { /* open file at v1 (apply first entry only), reopen with Db::open → user_version 2, .bak exists, devices kept */ }
#[tokio::test] async fn rollup_folds_finished_buckets_only_and_is_idempotent() { /* samples at 0..299 s (two with NULL) and at 300 s; rollup(now=301_000) twice → one row: count 3? use exact fixture: values [10, None, 30, None] in bucket 0 → min 10, avg 20, max 30, count 4, lost 2; bucket 300_000 not rolled */ }
#[tokio::test] async fn metrics_use_raw_points_up_to_a_day_and_rollups_beyond() { /* range 1 h → Raw, points == samples; range 7 d → FiveMin */ }
#[tokio::test] async fn metrics_list_the_sessions_in_range() { /* two sessions, second open → sessions in order, second ended_at None */ }
#[tokio::test] async fn prune_keeps_what_retention_says() { /* raw at now-25h gone, now-23h kept; rollup at now-31d gone; presence at now-91d gone */ }
#[tokio::test] async fn a_session_left_open_is_closed_as_a_crash_on_open() { /* file db: open_session, touch at 5000, drop; reopen → end_reason crash, ended_at 5000 */ }
#[test] fn targets_round_trip_as_text() { /* "device:12" <-> Target::Device(DeviceId(12)); "gateway"; bad text → Err */ }
#[test] fn scaled_config_divides_durations_but_not_retention() { /* default.scaled(60).tick == 500 ms; retention unchanged; offline_grace 2 s */ }
// storage_budget.rs
#[tokio::test] async fn thirty_days_with_ten_starred_devices_stays_under_50_mb() {
    /* temp file db; one network; insert 24 h of raw samples at 30 s for 15 series
       (gateway, internet, dns@60 s, wifi rssi + link rate, 10 devices) and 30 days of rollups for the same
       series (insert rows directly into samples_rollup with the store's Db::call); presence rows for
       10 devices × 1 change/hour × 90 d; checkpoint WAL; assert file size < 50 * 1024 * 1024 and print it */
}
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core --test store --test storage_budget`. Expected: FAIL.
- [ ] **Step 3:** Implement the migration, types, clock and store methods. Write samples in one transaction per call.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS. Record the printed database size for Task 15.
- [ ] **Step 5: Commit:** `Add schema v2 with samples, rollups, monitoring sessions and retention, plus the monitor config and clock.`

---

### Task 6: Pure monitor logic: conditions, presence and the alert engine

**Files:**
- Create: `crates/netlantern-core/src/monitor/{conditions.rs,presence.rs,rules.rs,copy.rs}`
- Modify: `crates/netlantern-core/src/monitor/{mod.rs,model.rs}`, `crates/netlantern-core/src/lib.rs`
- Test: unit tests in each file

**Interfaces:**
- Consumes: Task 5's types, plus `DeviceType`, `DeviceId` and `Device`.
- Produces:
  - **`enum AlertKey`**: `RouterDown`, `InternetDown`, `LatencyHigh`, `PacketLoss(Target)` (only `Gateway` or `Device`), `StarredOffline(DeviceId)`, `NewDevice(DeviceId)`.
    - Text form: `"router_down"`, `"internet_down"`, `"latency_high"`, `"packet_loss:gateway"`, `"packet_loss:device:12"`, `"starred_offline:12"`, `"new_device:12"`, via `Display` and `FromStr`.
    - `fn rule(&self) -> Rule`. `RouterDown` and `InternetDown` both map to `Rule::InternetDown`: the "internet down" switch covers both.
    - `fn severity(&self) -> Severity`.
    - `fn depends_on_router(&self) -> bool`: false for `RouterDown`, `PacketLoss(Gateway)` and `NewDevice`; true otherwise.
  - **`enum Rule { StarredOffline, InternetDown, LatencyHigh, PacketLoss, NewDevice }`**, snake_case.
  - **`enum Severity { Critical, Warning, Info }`**: `RouterDown`, `InternetDown` and `StarredOffline` are critical; latency and loss are warnings; new device is info.
  - **`struct RuleSettings { starred_offline: bool, internet_down: bool, latency_high: bool, packet_loss: bool, new_device: bool, notifications: bool }`**. `Default` is all `true`. Also `fn enabled(&self, rule: Rule) -> bool`.
  - **`struct Reading { key: AlertKey, failing: bool, ts: i64, title: String, detail: String }`**: one evaluation of one key.
  - **`conditions.rs`**:
    - `struct TickResult { at: i64, gateway_rtt: Option<f64>, gateway_arp: Option<bool>, internet_rtt: Option<f64>, internet_tcp_ms: Option<f64>, dns_ms: Option<Option<f64>>, wifi: Option<WifiConnection>, devices: Vec<DeviceTick> }`
    - `struct DeviceTick { id: DeviceId, rtt: Option<f64>, arp: Option<bool>, checks: Vec<CheckOutcome> }`
    - `struct LossWindow`: a ring of the last `loss_window` reply/no-reply results per target. `push(target, replied: bool)`; `lost(target) -> u32`.
    - `fn router_reading(t: &TickResult, gateway: Ipv4Addr) -> Reading`: failing when `gateway_rtt` is `None` and `gateway_arp == Some(false)`.
    - `fn internet_reading(t, cfg) -> Reading`: failing when the router is not failing, `internet_rtt` is `None` and `internet_tcp_ms` is `None`.
    - `fn latency_reading(t, cfg) -> Reading`: failing when the internet answered and its RTT (the ping, else the TCP connect time) is above `latency_high_ms`.
    - `fn loss_reading(target, window: &LossWindow, name: &str, cfg) -> Reading`: failing when `lost ≥ loss_threshold`.
    - `fn samples(t: &TickResult, gateway_has_answered_ping: bool) -> Vec<Sample>`:
      - gateway `RttMs`, only when it has answered at least one ping this session;
      - internet `RttMs` (the ping, else the TCP connect ms, else `None`);
      - `Dns` `LookupMs` when measured;
      - Wi-Fi `RssiDbm` and `LinkRateMbps` (the rx rate) when connected;
      - each device's `RttMs`.
  - **`presence.rs`**:
    - `fn grace(device_type: DeviceType, cfg) -> Duration`: `offline_grace_sleepy` for `Phone` and `Tablet`, else `offline_grace`.
    - `fn is_online(last_seen: i64, session_started_at: i64, now: i64, grace: Duration) -> bool`: `now - max(last_seen, session_started_at) < grace`.
    - `fn loss_applies(device_type) -> bool`: false for `Phone` and `Tablet`.
  - **`rules.rs`**: `struct AlertEngine`.
    - `fn new(consecutive: u32) -> Self`. `RouterDown`, `InternetDown` and `LatencyHigh` need `consecutive` failing readings in a row (`cfg.consecutive`, 2); every other key opens on its first failing reading, because its window or grace is already built into the reading.
    - `fn resume(&mut self, open: Vec<(AlertKey, bool /*suppressed*/)>)`: alerts open in the store before a restart.
    - `fn evaluate(&mut self, now: i64, readings: Vec<Reading>, rules: &RuleSettings) -> Vec<Decision>`
    - `fn forget(&mut self, key: &AlertKey, now: i64) -> Option<Decision>`: resolves an open key that is no longer watched (a device unstarred).
    - `enum Decision`:
      - `Open { key, title, detail, severity, suppressed: bool, detected_at: i64, last_good_at: Option<i64>, at: i64 }`
      - `Unsuppress { key, at }`
      - `Resolve { key, at }`
    - `fn should_notify(d: &Decision, rules: &RuleSettings) -> bool`: true for `Open` with `!suppressed`, and for `Unsuppress`, when `rules.notifications`.
  - **`copy.rs`** holds the exact titles and details below and the type labels.

**Engine rules** (spec §6.9 lifecycle):
- **Pending.** A failing reading for a key that isn't open increments its pending count. It records `detected_at` (the first failing `ts`) and keeps the last ok `ts` as `last_good_at`. An ok reading clears pending.
- **Open.** When the pending count reaches the key's required count, emit `Open`. It is `suppressed` when `key.depends_on_router()` and `RouterDown` is open and unsuppressed.
- **Resolve.** An ok reading for an open key emits `Resolve`; this holds for Open, Acked and Suppressed alike.
- **Order within one `evaluate`.**
  1. Resolves.
  2. If this call resolved `RouterDown`: every still-open suppressed key whose reading in this call is failing → `Unsuppress`, and one with no reading this call → `Unsuppress` too (it was failing when last seen).
  3. Opens.
- **Disabled rules.** Readings for a disabled rule are ignored. Any open key of a disabled rule emits `Resolve`.
- **New devices.** `NewDevice` readings are failing while the device is still flagged new. Each opens once and resolves when the flag clears.

**Copy** (exact). `{gateway}` is an IP. `{name}` is the device's `display_name`. `{type}` is a type label: Router, Laptop, Phone, Tablet, Printer, TV, Speaker, Camera, NAS, Console, IoT, or "unknown type". `{vendor}` is the vendor, "private address" for a private MAC, or "unknown vendor".

| Key | Title | Detail |
|---|---|---|
| `RouterDown` | `Your router isn't answering` | `No reply from {gateway} to ping or ARP.` |
| `InternetDown` | `The internet is down` | `Your router answers, but 1.1.1.1 doesn't (ping or HTTPS port).` |
| `LatencyHigh` | `Internet latency is high` | `{ms} ms to 1.1.1.1 on the last 2 checks (limit 100 ms).` |
| `PacketLoss(Gateway)` | `Packet loss to your router` | `{lost} of the last {window} pings got no reply.` |
| `PacketLoss(Device)` | `Packet loss to {name}` | same as above |
| `StarredOffline` | `{name} is offline` | `Not seen for {min} min.` |
| `NewDevice` | `New device: {name} — {vendor}, {type}` | `First seen at {ip}.` |

- [ ] **Step 1: Write the failing tests**

```rust
// conditions.rs
#[test] fn a_router_answering_arp_is_not_down() { /* gateway_rtt None, gateway_arp Some(true) → !failing */ }
#[test] fn tcp_443_counts_as_internet_up() { /* internet_rtt None, internet_tcp_ms Some(30.0) → internet !failing */ }
#[test] fn internet_is_not_blamed_while_the_router_is_down() { /* router failing → internet reading !failing */ }
#[test] fn a_silent_gateway_records_no_loss_samples() { /* samples(t, false) has no gateway RttMs */ }
#[test] fn loss_window_counts_the_last_thirty_only() { /* push 3 lost then 30 replied → lost 0 */ }
#[test] fn three_lost_of_thirty_is_failing() { /* … */ }
// presence.rs
#[test] fn phones_get_ten_minutes() { /* grace(Phone) == 10 min, grace(Printer) == 2 min */ }
#[test] fn grace_counts_from_the_session_start_after_a_sleep() { /* last_seen 8 h ago, session started 1 min ago, grace 2 min → online */ }
#[test] fn offline_after_grace() { /* last_seen 121 s ago, session long ago → offline */ }
// rules.rs
#[test] fn one_failure_is_pending_and_recovery_clears_it() { /* RouterDown failing once, then ok → no decisions */ }
#[test] fn two_failures_open_with_detected_and_last_good() { /* ok@0, fail@30, fail@60 → Open{detected_at 30, last_good_at Some(0), at 60} */ }
#[test] fn device_alerts_are_suppressed_while_the_router_is_down() { /* RouterDown open; StarredOffline failing → Open{suppressed: true}; should_notify false */ }
#[test] fn router_recovery_unsuppresses_what_still_fails_and_resolves_what_recovered() { /* two suppressed keys; router ok, key A failing, key B ok → [Resolve RouterDown, Resolve B, Unsuppress A] */ }
#[test] fn turning_a_rule_off_resolves_its_alerts() { /* LatencyHigh open; rules.latency_high=false → Resolve */ }
#[test] fn resumed_alerts_resolve_on_recovery_and_do_not_reopen() { /* resume([(StarredOffline(1), false)]); failing reading → no Open; ok → Resolve */ }
#[test] fn notifications_off_means_no_notify() { /* should_notify false when rules.notifications false */ }
#[test] fn keys_round_trip_as_text() { /* every variant */ }
// copy.rs
#[test] fn new_device_title_matches_journey_b() { /* name "192.168.1.57", vendor "Espressif", Iot → "New device: 192.168.1.57 — Espressif, IoT" */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core --lib monitor`. Expected: FAIL.
- [ ] **Step 3:** Implement. No I/O, no clock reads, no store calls: these are pure functions and pure state.
- [ ] **Step 4:** Run `cargo test -p netlantern-core`. Expected: PASS.
- [ ] **Step 5: Commit:** `Add the pure monitoring logic: check conditions, presence with sleep grace, and the alert engine with debounce and router suppression.`

---

### Task 7: Store: presence, checks and rule settings

**Files:**
- Create: `crates/netlantern-core/src/store/{presence.rs,checks.rs,rules.rs}`
- Modify:
  - `crates/netlantern-core/src/store/devices.rs` (`Device` online rule, `record` sets presence)
  - `crates/netlantern-core/src/model.rs` (`Device` gains fields)
  - `crates/netlantern-core/src/store/scans.rs` (`first_scan_finished_at`)
- Test: `crates/netlantern-core/tests/store.rs`

**Interfaces:**
- Consumes: Task 5's schema and Task 6's `RuleSettings`.
- Produces:
  - **`Device` gains** `checks: Vec<Check>` and `presence_changed_at: Option<i64>`. Its `online`:
    - `presence = 'online'` → true; `'offline'` → false;
    - NULL → the Phase 1 scan rule, unchanged.
  - `Check` and `CheckOutcome` are Task 5's types.
  - **`record_sighting`** (Phase 1) also sets `presence = 'online'` and `presence_changed_at = now` when the stored presence was not already online. That change also inserts a `presence` row.
  - **`Store` methods:**
    - `set_presence(&self, network, device: DeviceId, online: bool, ts: i64) -> Result<bool /*changed*/, StoreError>`. Updates the columns and inserts a `presence` row only on change.
    - `mark_seen(&self, network, device: DeviceId, ip: Ipv4Addr, ts: i64) -> Result<(), StoreError>`. Sets `last_seen = ts` and `last_ip = ip`.
    - `record_presence_sighting(&self, network, ip: Ipv4Addr, mac: MacAddr, ts: i64) -> Result<PresenceSighting, StoreError>`, with `struct PresenceSighting { device: Device, created: bool }`:
      - for a known MAC on this network: `mark_seen` plus the MAC's `last_seen`;
      - otherwise, it creates a device: vendor from the OUI, type from `classify` over `Signals { vendor, private_mac, .. }`, `is_new = 1`, `presence = 'online'`, and no `last_scan_id`.
    - `set_checks(&self, network, device: DeviceId, checks: Vec<Check>) -> Result<Option<Device>, StoreError>`. Replaces the device's checks; `None` for an unknown device.
    - `rule_settings(&self, network) -> Result<RuleSettings, StoreError>` and `set_rule_settings(&self, network, RuleSettings)`. They use the existing `settings` table with `scope = "network:<id>"`, keys `rule.<snake_name>` and `notifications`, and values `"on"`/`"off"`. A missing key reads as on.
    - `first_scan_finished_at(&self, network) -> Result<Option<i64>, StoreError>`: the `finished_at` of the network's oldest `done` scan.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn presence_overrides_the_scan_rule_once_set() { /* device from scan: online; set_presence(false) → get_device online false; presence row written once; set_presence(false) again → changed false, still one row */ }
#[tokio::test] async fn a_new_sighting_turns_an_offline_device_online() { /* set_presence(false); record_sighting → online true, presence_changed_at = now */ }
#[tokio::test] async fn an_unknown_mac_in_presence_creates_a_new_device() { /* record_presence_sighting(new MAC) → created true, is_new, vendor from OUI */ }
#[tokio::test] async fn a_known_mac_at_a_new_ip_moves_the_device() { /* created false, device.ip == new ip */ }
#[tokio::test] async fn checks_round_trip_in_order() { /* set [Tcp 80, Http{8080,"/status",false}] → Device.checks equal; JSON {"kind":"tcp","port":80} */ }
#[tokio::test] async fn rule_settings_default_on_and_persist_per_network() { /* default all true; set latency_high false on net A; net B still true */ }
#[tokio::test] async fn first_scan_finished_at_is_the_oldest_done_scan() { /* failed scan, done at 100, done at 200 → Some(100) */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core --test store`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS. Phase 1's device and API tests still pass, including their `online` expectations.
- [ ] **Step 5: Commit:** `Store device presence, starred-device checks and per-network rule settings.`

---

### Task 8: Store: alerts, incidents and timelines

**Files:**
- Create: `crates/netlantern-core/src/store/alerts.rs`
- Modify:
  - `crates/netlantern-core/src/monitor/model.rs` (API types)
  - `crates/netlantern-core/src/store/mod.rs`
  - `crates/netlantern-core/src/lib.rs`
- Test: `crates/netlantern-core/tests/alerts_store.rs`

**Interfaces:**
- Consumes: Task 6's `Decision`, `AlertKey`, `Rule` and `Severity`.
- Produces:
  - **IDs:** `struct AlertId(pub i64)` and `struct IncidentId(pub i64)`, both transparent.
  - **`enum AlertState { Open, Acked, Resolved }`**, snake_case.
  - **`struct Alert`**: `{ id, rule: Rule, key: String, severity, title, detail, suppressed: bool, state: AlertState, incident_id: IncidentId, opened_at: i64, acked_at: Option<i64>, resolved_at: Option<i64> }`.
  - **`enum TimelineKind`**, serialized as `kind`, snake_case: `LastGood`, `Detected`, `Alerted`, `Affected`, `Diagnosed`, `Acknowledged`, `Recovered`.
  - **`struct TimelineEvent { ts: i64, kind: TimelineKind, text: String, alert_id: Option<AlertId> }`**
  - **`struct Incident { id, opened_at, resolved_at: Option<i64>, root: Alert, alerts: Vec<Alert> /* all, root included, oldest first */, timeline: Vec<TimelineEvent> /* by ts, then insertion */ }`**
  - **`enum AlertFilter { All, Active /*open or acked*/, Open, Acked, Resolved }`**
  - **`Store` methods:**
    - `apply_decisions(&self, network, Vec<Decision>) -> Result<Vec<Alert>, StoreError>`: one transaction; returns every alert it changed, in order. It works as follows:
      - **`Open`, not suppressed:**
        - create an incident with this alert as root;
        - add events: `LastGood` "Last good check" (if `last_good_at`), `Detected` "Problem detected" at `detected_at`, and `Alerted` "Alert raised" at `at`.
      - **`Open`, suppressed:**
        - attach the alert to the open `router_down` alert's incident;
        - add `Affected` "{title} (held back while the router is down)" at `at`.
      - **`Unsuppress`:**
        - clear `suppressed` and create a new incident with it as root;
        - add `Detected` at its `opened_at` and `Alerted` at `at`;
        - on the router incident, add `Affected` "{title} is still failing after the router recovered".
      - **`Resolve`:**
        - set `Resolved` and `resolved_at`, and add `Recovered` "Recovered" to its incident;
        - if it is the incident's root, set `incidents.resolved_at`.

      A `Resolve` or `Unsuppress` for a key with no active alert is skipped. An `Open` for a key that already has an active alert is skipped; the unique index backs this up.
    - `ack_alert(&self, network, id: AlertId, ts) -> Result<AckOutcome, StoreError>`, with `enum AckOutcome { Acked(Alert), AlreadyResolved(Alert), NotFound }`. Adds an `Acknowledged` event. Acking an acked alert returns it unchanged.
    - `list_alerts(&self, network, filter: AlertFilter, limit: u32) -> Result<Vec<Alert>, StoreError>`: newest first.
    - `incident(&self, network, id: IncidentId) -> Result<Option<Incident>, StoreError>`
    - `active_alert_keys(&self, network) -> Result<Vec<(AlertKey, bool)>, StoreError>`: for `AlertEngine::resume`.
    - `add_diagnosis_to_open_incidents(&self, network, headline: &str, ts) -> Result<Vec<IncidentId>, StoreError>`: adds `Diagnosed` "Diagnosis: {headline}" to every unresolved incident.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn opening_an_alert_opens_an_incident_with_its_timeline() { /* Open RouterDown → incident root == alert; timeline kinds [LastGood, Detected, Alerted] */ }
#[tokio::test] async fn a_suppressed_alert_joins_the_router_incident() { /* RouterDown open, then Open{StarredOffline, suppressed} → same incident_id; Affected event text */ }
#[tokio::test] async fn unsuppress_gives_the_alert_its_own_incident() { /* … new incident root; router incident has the "still failing" Affected event */ }
#[tokio::test] async fn resolving_the_root_resolves_the_incident() { /* incident.resolved_at set; Recovered event */ }
#[tokio::test] async fn a_second_open_for_an_active_key_is_skipped() { /* apply Open twice → one alert */ }
#[tokio::test] async fn ack_then_resolve() { /* Acked(alert.state Acked, acked_at); later Resolve → Resolved; ack again → AlreadyResolved */ }
#[tokio::test] async fn list_filters_and_orders_newest_first() { /* Active excludes resolved */ }
#[tokio::test] async fn active_keys_feed_resume() { /* after restart-like reopen of a file db: active_alert_keys returns the open key with its suppressed flag */ }
#[tokio::test] async fn diagnosis_lands_on_open_incidents_only() { /* one open, one resolved → Diagnosed added to the open one */ }
#[test] fn alert_json_shape() { /* {"id":1,"rule":"internet_down","key":"router_down","severity":"critical",...,"state":"open"} */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core --test alerts_store`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit:** `Store alerts and incidents with their timelines, suppression into the router incident, and acknowledgement.`

---

### Task 9: The monitor task

**Files:**
- Create: `crates/netlantern-core/src/monitor/{sampler.rs,runner.rs,notify.rs}`
- Modify:
  - `crates/netlantern-core/src/monitor/mod.rs`
  - `crates/netlantern-core/src/events.rs`
  - `crates/netlantern-core/src/lib.rs`
- Test: `crates/netlantern-core/tests/monitor_sim.rs` (paused clock, `TokioClock`, in-memory store)

**Interfaces:**
- Consumes: Tasks 4–8, `ScanManager::current_network`, `EventBus`, and Phase 1's `ScanManager` to run the scan that identifies the network in tests.
- Produces:
  - `trait Notifier: Send + Sync { fn notify(&self, title: &str, body: &str); }`. It must not block: an implementation hands work off to a thread. `struct NoopNotifier`.
  - `enum MonitorState { Monitoring, WaitingForScan, NotConnected }`, snake_case.
  - `struct MonitorTick { ts: i64, network_id: NetworkId, gateway_rtt_ms: Option<f64>, internet_rtt_ms: Option<f64>, router_lost: u32, router_window: u32, dns_ms: Option<f64>, wifi_rssi_dbm: Option<i32> }`
  - `struct DeviceChecks { device_id: DeviceId, outcomes: Vec<CheckOutcome> }`
  - `struct MonitorStatus { state: MonitorState, network_id: Option<NetworkId>, session_started_at: Option<i64>, last: Option<MonitorTick>, checks: Vec<DeviceChecks> }`
  - `struct Monitor`:
    - `pub fn new(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>, store: Store, bus: Arc<EventBus>, scans: Arc<ScanManager>, notifier: Arc<dyn Notifier>, clock: Arc<dyn Clock>, cfg: MonitorConfig) -> Self`
    - `pub fn start(&self)`: spawns the loop and the bus listener. Calling it twice is a no-op.
    - `pub fn status(&self) -> MonitorStatus`
    - `pub async fn stop(&self, reason: EndReason)`: stops the loop and ends any open session at `clock.now_ms()`.
  - **`EventKind` gains:**
    - `"monitor.tick"`: `MonitorTick { tick: MonitorTick }`
    - `"monitor.state"`: `MonitorStateChanged { state: MonitorState, network_id: Option<NetworkId>, session_started_at: Option<i64> }`
    - `"device.presence"`: `DevicePresence { device_id: DeviceId, online: bool, at: i64 }`
    - `"alert.changed"`: `AlertChanged { alert: Alert }`

**Loop:** `tokio::time::interval(cfg.tick)` with `MissedTickBehavior::Delay`; the first tick is immediate. Each tick, in order:
1. **Sleep check.** `now = clock.now_ms()`. If a session is open and `now - last_tick_wall > sleep_gap`, end it with `Sleep` at `last_tick_wall`, publish the state, and reset the engine's pending entries and the loss windows.
2. **Which network.** `scans.current_network()`:
   - `Err` or a not-connected interface → `NotConnected`;
   - `Ok(None)` → `WaitingForScan`.

   In both cases, end any open session with `NetworkLeft` and skip the rest of the tick.
3. **Network change.** If the network differs from the session's, end the session with `NetworkLeft`.
4. **New session.** With no session open: `open_session`, then `engine.resume(store.active_alert_keys(network))`, then reset the loss windows and set `gateway_has_answered_ping = false`. Publish the state.
5. **Probe.** Load the starred devices (`list_devices`, filtered) and `rule_settings`. Then `sampler::tick(...)` runs concurrently:
   - **Gateway:** a ping (`ping_timeout`); only on no reply, `arp_resolve` (`arp_timeout`).
   - **Internet:** a ping to `internet_target`; only on no reply, `tcp_probe(target, 443, tcp_fallback_timeout)`, timed.
   - **Each starred device with an IP:** a ping; on no reply, `arp_resolve`. Then its checks:
     - TCP: `tcp_probe`; ok = `Open`.
     - HTTP: `netio.http_get("{http|https}://{ip}:{port}{path}", true)`; ok = status 200.
   - **DNS:** when `dns_every` has passed, `dns_resolve(first system DNS, dns_name)`, timed.
   - **Wi-Fi:** `wifi_current()`; errors become `None`.
6. **Write.** Update the loss windows (the gateway only once it has answered a ping). Write `conditions::samples` with `insert_samples`, then `touch_session`.
7. **Presence of starred devices.** A device that answered ping or ARP gets `mark_seen`. Each device's online state is `presence::is_online(last_seen, session_started_at, now, grace(type))`. `set_presence` on change, and publish `device.presence` for each change.
8. **Presence refresh**, when `presence_every` has passed (and on the first tick of a session):
   - ping every non-starred device with an IP (`presence_ping_concurrency`), then `arp_resolve` the non-responders (`presence_arp_concurrency`), marking seen as above;
   - read `arp_table()`: each `Reachable` entry on the subnet goes to `record_presence_sighting`. Skip the gateway and this laptop, and skip zero MACs;
   - update presence for every device as in step 7;
   - a created device gives a `NewDevice` reading when `first_scan_finished_at(network)` is `Some(t)` with `t < device.first_seen` and `rules.new_device` is on.
9. **Readings and decisions.** Build the readings: router, internet, latency (the engine sees `consecutive`), packet loss for the gateway and for each starred device where `loss_applies`, `StarredOffline` for each starred device, and, for each active `NewDevice` key, a reading that fails while that device is still `is_new` (so naming or acknowledging it resolves the alert). Then:
   - `forget` keys of devices no longer starred;
   - `engine.evaluate`, then `store.apply_decisions`;
   - publish `alert.changed` for each returned alert;
   - call `notifier.notify(title, detail)` for each decision where `should_notify`.
10. **Housekeeping.** When `rollup_every` has passed, run `rollup(now)` and `prune(now, retention)`.
11. **Publish.** Send `monitor.tick`, and store the tick and check outcomes for `status()`.

**Bus listener** (a separate task, subscribed at `start`):
- **`ScanDevice`:** for a device with `is_new` and `first_seen` > `first_scan_finished_at(network)`, feed a `NewDevice` reading through the same engine and store path, behind a shared `tokio::sync::Mutex` held with the tick.
- **`DiagnoseDone` with a verdict:** `add_diagnosis_to_open_incidents(current network, headline, now)`, then publish `alert.changed` for each affected incident's root alert.

**Store or platform errors** inside a tick are logged once per kind and the tick continues; they never end the session or the task. A panic inside a tick is caught (`catch_unwind`, as in the scan) and the next tick runs.

- [ ] **Step 1: Write the failing tests** (each runs a quick scan first so the network is known, stars devices via `update_device`, then `advance`s the paused clock)

```rust
#[tokio::test(start_paused = true)] async fn printer_offline_alerts_within_two_and_a_half_minutes_and_resolves() {
    /* printer-offline; star .40; offline at 600 s → alert StarredOffline opened_at - 600_000 ≤ 150_000;
       incident timeline [Detected, Alerted] (+LastGood); notifier recorded exactly one call with title "Office printer is offline";
       back at 1200 s → Resolved, incident resolved, Recovered event */ }
#[tokio::test(start_paused = true)] async fn a_sleeping_phone_never_alerts_in_24_hours() {
    /* sleeping-phone; star .57; advance 24 h → list_alerts(All) is empty; phone online at the end */ }
#[tokio::test(start_paused = true)] async fn router_outage_suppresses_device_alerts_then_unsuppresses_the_one_still_down() {
    /* router-outage; star printer; at 600 s router down → RouterDown open; StarredOffline opens suppressed into the same incident, no notify;
       router back at 900 s → RouterDown resolved, printer alert unsuppressed with its own incident, one notify; printer back at 1500 → resolved */ }
#[tokio::test(start_paused = true)] async fn a_silent_router_raises_nothing() { /* router-silent for 1 h → no alerts, no gateway samples */ }
#[tokio::test(start_paused = true)] async fn blocked_icmp_is_not_internet_down() { /* icmp-blocked for 1 h → no alerts; internet samples present (TCP times) */ }
#[tokio::test(start_paused = true)] async fn a_device_joining_later_raises_one_new_device_alert() { /* new-device: no alerts after the first scan; .77 joins at 600 s → exactly one NewDevice alert by 900 s with the journey-B title shape */ }
#[tokio::test(start_paused = true)] async fn the_first_scan_raises_no_new_device_alerts() { /* home: scan, run 10 min → no NewDevice */ }
#[tokio::test(start_paused = true)] async fn a_wall_clock_jump_ends_the_session_as_sleep_without_offline_alerts() {
    /* printer-offline before 600 s; star printer; jump the TokioClock's epoch forward 8 h between ticks (TokioClock exposes jump(ms) for tests) →
       previous session end_reason Sleep; new session; no StarredOffline at the first ticks after the jump */ }
#[tokio::test(start_paused = true)] async fn leaving_the_network_ends_the_session() { /* stub platform switches to NotConnected → session NetworkLeft, state NotConnected event */ }
#[tokio::test(start_paused = true)] async fn stop_ends_the_session_as_quit() { /* … */ }
#[tokio::test(start_paused = true)] async fn open_alerts_resume_after_a_restart() { /* file db; printer offline alert open; drop monitor; new Monitor on same store → no duplicate; printer back → resolved */ }
#[tokio::test(start_paused = true)] async fn the_scheduler_never_scans_the_radio() { /* counting platform wrapper: wifi_scan calls == 0 after 1 h */ }
#[tokio::test(start_paused = true)] async fn a_diagnosis_lands_on_the_open_incident() { /* router-outage during the outage: DiagnoseManager on same bus runs → incident has Diagnosed */ }
#[tokio::test(start_paused = true)] async fn ticks_publish_samples_and_rollups_happen() { /* home 30 min → monitor.tick events ~60; metrics(Gateway, RttMs, 7 d range) has FiveMin points */ }
```

`TokioClock` gains `pub fn jump(&self, ms: i64)` (an `AtomicI64` offset). The wall clock moves without tokio time moving, which is what a laptop sleep looks like.

- [ ] **Step 2:** Run `cargo test -p netlantern-core --test monitor_sim`. Expected: FAIL.
- [ ] **Step 3:** Implement `sampler.rs` (I/O only, returns `TickResult`), `runner.rs` (the loop above), `notify.rs` and the events. Keep `runner.rs` under ~400 lines; split presence refresh into its own function.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit:** `Run monitoring on a schedule with sessions, presence, alerts, incidents and notifications, tested on simulated outages.`

---

### Task 10: Server: monitor, metrics, alerts, incidents, rules and device checks

**Files:**
- Modify:
  - `crates/netlantern-server/src/{state.rs,routes.rs,error.rs,lib.rs}`
  - `crates/netlantern-server/tests/api.rs`
- Test: `crates/netlantern-server/tests/{api.rs,monitor_api.rs}`

**Interfaces:**
- Consumes: Task 9's `Monitor` and its types, and Tasks 7–8's store methods.
- Produces:
  - `pub struct MonitorParts { pub config: MonitorConfig, pub notifier: Arc<dyn Notifier>, pub clock: Arc<dyn Clock> }`. Its `Default` is the default config, `NoopNotifier` and `SystemClock`.
  - `AppState` gains `pub monitor: Arc<Monitor>`.
    - `AppState::with_monitor(platform, netio, store, scan, diagnose, monitor: MonitorParts, policy) -> Self`.
    - `AppState::new` keeps its signature and calls `with_monitor` with `MonitorParts::default()`.
    - Neither starts the monitor; the host calls `state.monitor.start()`.
  - **New `ApiError` codes:**
    - `no_network` (409): message "Net Lantern doesn't know this network yet.", hint "Run a quick scan first."
    - `alert_resolved` (409): message "This alert has already been resolved."
  - **Routes** (all behind the guard):

    | Route | Answer |
    |---|---|
    | `GET /api/monitor` | `MonitorStatus` |
    | `GET /api/metrics?target=&metric=&range=` | `MetricSeries`. `range` is `1h`, `24h`, `7d` or `30d`, ending now. No known network → 200 with empty `points` and `sessions`. A bad `target`, `metric` or `range` → 400 naming the allowed values. |
    | `GET /api/alerts?state=&limit=` | `Vec<Alert>` for the current network. `state` is `all` (default), `active`, `open`, `acked` or `resolved`. `limit` defaults to 50, capped at 200. No network → `[]`. |
    | `POST /api/alerts/{id}/ack` | `Alert`; 409 `alert_resolved`; 404 |
    | `GET /api/incidents/{id}` | `Incident`; 404 |
    | `GET /api/rules`, `PUT /api/rules` | `RuleSettings`; no network → 409 `no_network`. PUT needs all six fields (`deny_unknown_fields`). |
    | `PATCH /api/devices/{id}` | Gains `checks`. Validation: at most `max_checks` (5); TCP and HTTP ports 1–65535; an HTTP `path` starting with `/`, at most 200 characters, with no whitespace. Errors → 400 with a message naming the rule. |

  - The 401 test covers every new route.

- [ ] **Step 1: Write the failing tests**

```rust
// monitor_api.rs (AppState::with_monitor + TokioClock + scaled config 60, SimNetwork printer-offline with_time_scale(60), real time with short waits)
#[tokio::test] async fn monitor_reports_waiting_for_scan_then_monitoring() { /* before scan: state waiting_for_scan; after POST /api/scans done: monitoring with session_started_at */ }
#[tokio::test] async fn starring_the_printer_leads_to_an_alert_an_incident_and_a_resolve() {
    /* PATCH starred; wait for alert.changed over the WebSocket; GET /api/alerts?state=active has "Office printer is offline";
       GET /api/incidents/{id} timeline kinds contain detected, alerted; ack → state acked; later resolved */ }
#[tokio::test] async fn metrics_validate_their_query() { /* range=2h → 400 mentioning "1h, 24h, 7d, 30d"; target=router → 400 */ }
#[tokio::test] async fn rules_need_a_known_network_and_round_trip() { /* before scan 409 no_network; after: PUT {latency_high:false,…} → GET equal */ }
#[tokio::test] async fn device_checks_are_validated() { /* 6 checks → 400; path "status" → 400; port 0 → 400; valid → Device.checks */ }
#[tokio::test] async fn acking_a_resolved_alert_is_a_conflict() { /* … 409 alert_resolved */ }
// api.rs
#[tokio::test] async fn every_new_route_needs_a_session() { /* extend the 401 table */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-server`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit:** `Serve monitoring status, metrics, alerts, incidents, rules and starred-device checks over the API.`

---

### Task 11: The exe: start and stop monitoring, demo time scale

**Files:**
- Modify:
  - `crates/netlantern/src/{main.rs,cli.rs}`
  - `crates/netlantern/tests/exe.rs`
  - `README.md` (outside traffic, `--time-scale`)

**Interfaces:**
- Consumes: Task 10's `AppState::with_monitor` and `MonitorParts`.
- Produces:
  - CLI `--time-scale <N>`:
    - `requires = "demo"`, range 1..=600, hidden from `--help`'s short summary but documented in the README;
    - it applies `MonitorConfig::scaled(N)` and `SimNetwork::with_time_scale(N)`.
  - In `main` after building state:
    - `state.monitor.start()`;
    - after `run(...)` returns, `state.monitor.stop(EndReason::Quit).await`.
  - The notifier is `NoopNotifier` until Task 12 swaps it.
  - A panic hook (`std::panic::set_hook`) logs the panic message and location through `tracing::error!`, then calls the previous hook. Nothing else: the next start closes the session as `crash` (Task 5).

- [ ] **Step 1: Write the failing tests**

```rust
// exe.rs
#[test] fn time_scale_requires_demo() { /* netlantern --time-scale 10 → exit 2, stderr mentions --demo */ }
#[test] fn the_demo_monitors_after_a_scan() {
    /* spawn --demo=scenarios/home.json --time-scale 60 --no-browser --port <free>; log in with the printed token (reuse the e2e/serve helper);
       POST /api/scans, wait done; within 5 s GET /api/monitor state == "monitoring" and last.gateway_rtt_ms is a number; kill */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern --test exe`. Expected: FAIL.
- [ ] **Step 3:** Implement. Update the README:
  - the scheduled outside traffic from Global Constraints;
  - a "Try the monitoring demo" line: `netlantern --demo=scenarios/printer-offline.json --time-scale 10`, then star "Office printer".
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit:** `Start monitoring with the server, end the session on quit, and add a demo time scale.`

---

### Task 12: Desktop notifications from the exe

**Files:**
- Create: `crates/netlantern/src/notify.rs`
- Modify: `crates/netlantern/src/{main.rs,lib.rs}`, `crates/netlantern/Cargo.toml`
- Test: `crates/netlantern/src/notify.rs` (unit)

**Interfaces:**
- Consumes: Task 2's outcome, and Task 9's `Notifier` trait.
- Produces: `fn desktop_notifier() -> Arc<dyn Notifier>`.
  - On Windows, per Task 2's outcome:
    - **A:** `WindowsToast { app_id: "NetLantern.NetLantern" }`. Calling `register_app_id()` first writes the HKCU key with `DisplayName` "Net Lantern", skipping the write when the value is already right. A failure is logged once and falls back to B's id.
    - **B:** `WindowsToast { app_id: Toast::POWERSHELL_APP_ID }`, titles prefixed `Net Lantern: `.
    - **C:** `NoopNotifier`, with a log line at startup.
  - Non-Windows: `NoopNotifier`.
  - `WindowsToast::notify` sends `(title, body)` to one dedicated thread over an `std::sync::mpsc` channel. Toasts are shown in order, and a failure is logged once per kind.
  - `--demo` and the real run both use `desktop_notifier()`. The exe tests pass `--no-notify`: a new hidden flag that selects `NoopNotifier`, so CI never shows toasts.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn titles_get_the_app_name_prefix_only_for_the_powershell_route() { /* format_title(Route::PowerShell, "X") == "Net Lantern: X"; Route::AppId → "X" */ }
#[test] fn the_toast_thread_survives_a_failing_show() { /* inject a show fn that errs; two notifies → both attempted, no panic */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern --lib`. Expected: FAIL.
- [ ] **Step 3:** Implement, wire it into `main`, and add `--no-notify` to the exe tests and to `ui/e2e/global-setup.ts`.
- [ ] **Step 4:** Manual check with the author: run `netlantern --demo=scenarios/printer-offline.json --time-scale 10`, scan, and star "Office printer". The author confirms a toast "Office printer is offline" appears about 6–7 s later (600 s / 10 + 2 min / 10 + one tick).
- [ ] **Step 5:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 6: Commit:** `Show desktop notifications for new alerts from the exe.`

---

### Task 13: UI: data layer, live events and the Dashboard

**Files:**
- Create:
  - `ui/src/api/monitor.ts`
  - `ui/src/api/alerts.ts`
  - `ui/src/components/{KpiTiles.tsx,LatencyChart.tsx,StarredDevices.tsx,RecentAlerts.tsx}`
  - `ui/src/lib/series.ts`
- Modify:
  - `ui/src/api/events.ts`
  - `ui/src/pages/Dashboard.tsx`
  - `ui/src/components/Shell.tsx` (alert badge)
  - `ui/package.json` (`uplot` 1.6.32)
- Test: `*.test.ts(x)` next to each new or changed file

**Interfaces:**
- Consumes: Task 10's JSON shapes.
- Produces:
  - **`monitor.ts`:**
    - types `MonitorStatus`, `MonitorTick`, `MetricSeries`, `MetricPoint`, `SessionSpan`, `Range = "1h" | "24h" | "7d" | "30d"`;
    - `useMonitor()`: query key `["monitor"]`;
    - `useMetrics(target, metric, range)`: key `["metrics", target, metric, range]`, `refetchInterval` 60 s.
  - **`alerts.ts`:**
    - types `Alert`, `Incident`, `TimelineEvent`, `RuleSettings`;
    - `useAlerts(state)`: key `["alerts", state]`;
    - `useIncident(id)`: key `["incident", id]`;
    - `useRules()`: key `["rules"]`;
    - `ackAlert(id)`, `saveRules(r)`, and `saveChecks(deviceId, checks)` (PATCH).
  - **`series.ts`:** `withGaps(points: MetricPoint[], sessions: SessionSpan[]): { ts: number[]; avg: (number | null)[] }`. It inserts a `null` at each session's `ended_at` (and at the start of a gap), so lines break where Net Lantern wasn't monitoring. Times are converted to seconds for uPlot.
  - **`events.ts` additions:**
    - `monitor.tick` → `setQueryData(["monitor"], old => ({ ...old, last: tick }))`, and invalidate `["metrics"]` at most once per 30 s;
    - `monitor.state` → invalidate `["monitor"]`;
    - `device.presence` → invalidate `["devices"]` and `["device"]`;
    - `alert.changed` → invalidate `["alerts"]`, `["incident"]` and `["monitor"]`;
    - `onResync` → also invalidate `["monitor"]`, `["alerts"]` and `["metrics"]`.

**Dashboard.** The new sections go after the scan card, which stays as it is:
- **Monitoring line.**
  - `monitoring`: "Monitoring since {time}" (local time, `toLocaleTimeString` with hour and minute).
  - `waiting_for_scan`: "Run a quick scan to start monitoring this network."
  - `not_connected`: "Not monitoring: not connected."
- **`KpiTiles`**: five tiles, each a label above a value, both text:

  | Tile | Value |
  |---|---|
  | Devices online | "{online} of {total}", from `useDevices` |
  | Router | "{n} ms" or "No reply" |
  | Internet | "{n} ms" or "No reply" |
  | Router loss (15 min) | "{lost} of {window}" |
  | Active alerts | count of `useAlerts("active")` |

  A tile with no data shows "—". The grid is 2 columns on phones and 5 at `lg`.
- **`LatencyChart`:**
  - range buttons 1h / 24h / 7d (`aria-pressed`);
  - one uPlot chart with two series, "Router" and "Internet" (gateway and internet `rtt_ms`), from `withGaps`;
  - colours from CSS tokens read with `getComputedStyle`;
  - it resizes with its container (`ResizeObserver`);
  - below it, a text summary for screen readers and everyone: "Last hour: router average {a} ms, internet average {b} ms." (or "No samples yet."), and the caption "Breaks in the lines are times Net Lantern wasn't monitoring."
- **`StarredDevices`:** starred devices with an online/offline text badge and "online for {formatAgo(presence_changed_at)}" (or "offline since …"), plus each check's outcome from `MonitorStatus.checks` ("Port 80: open", "HTTP /status: no answer"). Empty state: "Star a device on the Devices screen to watch it closely."
- **`RecentAlerts`:** the 5 newest active alerts, each showing severity as text ("Critical", "Warning", "Info"), the title and `formatAgo(opened_at)`, with a link "See all alerts" to `/alerts`. Empty: "No active alerts."
- **Shell:** the Alerts nav item shows the active count as text, e.g. "Alerts (2)", with `aria-label` "Alerts, 2 active", hidden when 0.

- [ ] **Step 1: Write the failing tests**

```ts
// series.test.ts
it("breaks the line between sessions", () => { /* points at 0,30,60 s in session A (ended 60 s), 600 s in session B → avg has a null between 60 and 600 */ });
// events.test.ts
it("puts a monitor tick into the monitor query without refetching", async () => { /* … */ });
it("refreshes alerts when one changes", async () => { /* invalidation of ["alerts"] */ });
// KpiTiles.test.tsx
it("shows text values and dashes when empty", async () => { /* "9 of 13", "3 ms", "No reply", "0 of 30", "2"; empty monitor → "—" */ });
// LatencyChart.test.tsx (uplot mocked via vi.mock: records constructor args)
it("draws router and internet series and a text summary", async () => { /* series labels; "Last hour: router average 3 ms, internet average 24 ms." */ });
it("switches range", async () => { /* click 24h → fetch with range=24h; aria-pressed */ });
// Dashboard.test.tsx
it("asks for a scan before monitoring", async () => { /* waiting_for_scan text */ });
// StarredDevices.test.tsx, RecentAlerts.test.tsx: empty states + one item each
// Shell.test.tsx
it("shows the active alert count in the nav", async () => { /* "Alerts (2)" */ });
```

- [ ] **Step 2:** Run `cd ui && npm test`. Expected: the new tests FAIL.
- [ ] **Step 3:** `npm install uplot@1.6.32`, then implement. Import `uplot/dist/uPlot.min.css`. In `src/test/setup.ts`, stub `matchMedia` and `ResizeObserver` if they are missing.
- [ ] **Step 4:** Run `npm test`, `npm run lint` and `npm run build`. Expected: PASS. At 375 px wide in the browser pane against `--demo --time-scale 60`, check `document.documentElement.scrollWidth === 375` and that the chart fits.
- [ ] **Step 5: Commit:** `Show live monitoring tiles, a router and internet latency graph, starred devices and recent alerts on the Dashboard.`

---

### Task 14: UI: the Alerts screen, rules and starred-device checks

**Files:**
- Create:
  - `ui/src/pages/Alerts.tsx`
  - `ui/src/components/{IncidentTimeline.tsx,RulesCard.tsx,NotificationPreview.tsx,ChecksEditor.tsx}`
- Modify:
  - `ui/src/App.tsx` (route `alerts` → `<Alerts />`)
  - `ui/src/components/DeviceDetails.tsx` (`ChecksEditor` for starred devices)
- Test: `*.test.tsx` next to each

**Interfaces:**
- Consumes: Task 13's `alerts.ts` and `monitor.ts`.
- Produces: the screens only.

**Alerts screen:**
- **Filter tabs**, buttons with `aria-pressed`: Active (default), Resolved, All.
- **List.** Each row shows:
  - severity text, the title, the detail;
  - "{opened ago}", plus "· acknowledged" or "· resolved {ago}";
  - "Held back while the router is down" when `suppressed`;
  - an **Acknowledge** button for `open` alerts (≥ 44 px). Pressing it calls `ackAlert`; on 409 it shows "Already resolved." inline.

  Clicking a row's title opens its incident.
- **Incident panel** (`IncidentTimeline`), under the list on phones and beside it at `lg`:
  - "Incident" heading, opened and resolved times;
  - an ordered list of timeline events: local time, kind label ("Last good check", "Detected", "Alert raised", "Affected", "Diagnosed", "Acknowledged", "Recovered") and text;
  - "Also affected" listing non-root alerts.
- **`RulesCard`:** six checkboxes, each with a one-line explanation:

  | Checkbox | Explanation |
  |---|---|
  | Starred device offline | 2 min, phones and tablets 10 min |
  | Internet down | router or internet unreachable for 2 checks |
  | Internet latency high | over 100 ms for 2 checks |
  | Packet loss | 3 of the last 30 checks to the router or a starred device; phones and tablets are skipped because they drop pings while asleep |
  | New device joined | — |
  | Send desktop notifications | — |

  Saving is immediate on change (`saveRules`), and an error shows inline. On 409 `no_network` the card shows the hint instead of the switches.
- **`NotificationPreview`:** a small card styled like a toast, with "Net Lantern", the newest active alert's title and its detail. With none, it shows the sample "Office printer is offline / Not seen for 2 min." It is captioned "What a desktop notification looks like". Below it: "If notifications don't appear, check Windows Settings → System → Notifications." The app doesn't change that setting.

**`ChecksEditor`** (in device details, only when `starred`):
- lists the checks, each with a Remove button;
- "Add check" opens inline fields: kind (TCP port / HTTP), port, and for HTTP a path (default `/`) and an HTTPS checkbox. Save calls `saveChecks` and shows the server's 400 message inline;
- explains: "Checks run every 30 s against this device's own address. Ping is always on for starred devices."

- [ ] **Step 1: Write the failing tests**

```tsx
// Alerts.test.tsx
it("lists active alerts with severity text and acknowledges one", async () => { /* POST /api/alerts/1/ack called; row shows "· acknowledged" after refetch */ });
it("shows held-back alerts as such", async () => { /* suppressed → text */ });
it("opens an incident timeline in order with labels", async () => { /* kinds → labels in order */ });
it("says already resolved on a 409", async () => { /* … */ });
// RulesCard.test.tsx
it("saves a rule change at once", async () => { /* uncheck latency → PUT body latency_high false */ });
it("shows the scan hint when the network is unknown", async () => { /* 409 no_network → "Run a quick scan first." */ });
// NotificationPreview.test.tsx
it("previews the newest active alert, else a sample", async () => { /* … */ });
// ChecksEditor.test.tsx
it("adds an HTTP check and shows a server validation message", async () => { /* PATCH body checks; 400 message shown */ });
it("is not shown for unstarred devices", async () => { /* … */ });
```

- [ ] **Step 2:** Run `cd ui && npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm test`, `npm run lint` and `npm run build`. Expected: PASS. Check 375 px width as in Task 13.
- [ ] **Step 5: Commit:** `Add the Alerts screen with incident timelines, rule switches, a notification preview and starred-device checks.`

---

### Task 15: End-to-end, docs, spec records and the checkpoint list

**Files:**
- Modify:
  - `ui/e2e/demo.spec.ts`, `ui/e2e/global-setup.ts`: the server runs `--demo --no-browser --no-notify --time-scale 60` on `home.json`.
  - `docs/superpowers/specs/2026-10-31-netlantern-design.md`
  - `README.md`
- Create: `docs/checkpoints/phase-3.md` (the manual checklist)

**Steps:**
- [ ] **Step 1: e2e.** Add to `demo.spec.ts` (after the existing scan test, so the network is known):
  - **"the dashboard starts monitoring after a scan":** the tiles show a Router value ending in " ms" within 10 s, and the chart summary text appears.
  - **"the alerts screen shows rules and no active alerts":** go to Alerts, the "Starred device offline" checkbox is checked, and the text "No active alerts." is shown.
  - **"starring a device shows it under starred devices":** star the NAS from Devices; the Dashboard lists it with "Online".

  The existing tests keep passing. `home.json` has no timeline, so its device count is unchanged.
- [ ] **Step 2: Spec records:**
  - **§6.5:** "Checks target only the device's own address (TCP port; HTTP GET on its IP, OK means status 200). Ping is implicit for starred devices. At most 5 checks."
  - **§6.8:** "Presence uses ping or an ARP resolve, not the age of an ARP table entry. Offline grace counts from the later of last seen and the session start, so waking from sleep doesn't mark everything offline. A wall-clock gap over 90 s between ticks ends the session as sleep. When 1.1.1.1 doesn't answer ping, a TCP connect to port 443 counts as a reply. A gateway that has answered no ping this session records no loss samples. A Reachable ARP entry with an unknown MAC creates a device during presence refresh."
  - **§6.9** (changed in phase 3):
    - router down is its own alert under the "internet down" switch;
    - the packet-loss rule skips phones and tablets;
    - new-device alerts start after the network's first finished scan;
    - when the router recovers, a suppressed alert is un-suppressed if it is still failing;
    - turning a rule off resolves its open alerts;
    - a diagnosis run during an open incident adds a "diagnosed" event; monitoring never starts a diagnosis itself, because diagnoses scan the radio.
  - **§6.12:** the new rows from Global Constraints, marked *(changed in phase 3)*.
  - **§7:** `monitor_sessions` gains `last_tick_at`. `alerts` holds `key`, `title` and `detail` in place of `subject`. `devices` gains `presence` and `presence_changed_at`. `samples.value` NULL means no reply.
  - **§8:** the new routes, with `/api/metrics` taking `range=1h|24h|7d|30d` and adding `/api/monitor`, plus the error codes `no_network` and `alert_resolved` and the event names `monitor.tick`, `monitor.state`, `device.presence` and `alert.changed`.
  - **§2.1:** the storage-budget figure from Task 5, as a dated note.
  - **§16:** confirm the toast and Tauri rows hold the spike outcomes.
- [ ] **Step 3: README:** the scheduled outside traffic, the monitoring demo line (Task 11), and that monitoring stops while the laptop sleeps.
- [ ] **Step 4: Checkpoint list.** Write `docs/checkpoints/phase-3.md` for the author to run on the home network:
  1. Scan, star a device you can unplug or switch off (a printer, a TV, a smart plug), and switch it off. Expect an alert and a toast within 2.5 min, and an incident timeline. Switch it back on; the alert resolves.
  2. Star your phone, leave it idle and locked for 24 h with Net Lantern running. Expect no alert. Count every alert over that day for the false-alert target (≤ 1).
  3. With the router's WAN cable out for 3 min: expect "The internet is down", then a resolve.
  4. With the router off for 3 min while a starred device is on: expect "Your router isn't answering", with the device alert held back.
  5. Sleep the laptop for 10 min. The graph shows a break and no alert fires on wake.
  6. Footprint after 1 h of monitoring: the CPU average and RAM of `netlantern.exe` (Task Manager, or `Get-Process netlantern | Select CPU,WS`), against < 1% and < 50 MB.
  7. **Still owed from Phase 1:** quick-scan time; coverage against the router's client list; vendor and type rates; the OS-ARP check (§6.3).

  Each item has a blank "Result:" line.
- [ ] **Step 5:** Run the full check: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, then in `ui/`: `npm run lint`, `npm test`, `npm run build`, `npm run e2e`. Expected: all PASS.
- [ ] **Step 6: Commit:** `Add monitoring end-to-end tests, the Phase 3 checkpoint list, and record the monitoring rules in the spec and README.`
