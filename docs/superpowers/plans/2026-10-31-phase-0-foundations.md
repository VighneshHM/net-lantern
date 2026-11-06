# Net Lantern Phase 0 — Foundations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `netlantern --demo` serves the dashboard with simulated devices from a real (simulated) quick scan streamed over a WebSocket, and a page on another origin can't open that WebSocket.

**Architecture:** Cargo workspace with four crates. `netlantern-platform` defines the `Platform`/`NetIo` traits and a simulator that loads JSON scenarios. `netlantern-core` holds the scan skeleton, the in-memory device store and the event bus. `netlantern-server` is an axum library with session/Host/Origin guards, REST, a WebSocket and the embedded UI. `netlantern` is the exe with single-instance and demo mode. React UI in `ui/`, built into the binary.

**Tech Stack:** Rust stable (edition 2021): tokio 1, axum 0.8 (`ws`), tower 0.5, async-trait 0.1, serde/serde_json 1, thiserror 2, rand 0.8, hex 0.4, rust-embed 8, mime_guess 2, clap 4 (derive), webbrowser 1, tracing 0.1 + tracing-subscriber 0.3, futures 0.3; tests: reqwest 0.12 (`json`, `cookies`), tokio-tungstenite 0.26. UI: React 19, TypeScript 5, Vite 6, Tailwind CSS 4 (`@tailwindcss/vite`), react-router 7, @tanstack/react-query 5, zustand 5, Vitest 3 + @testing-library/react 16, @playwright/test 1.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md` (Phase 0 row of §12; §5, §5.1, §6.2, §6.11, §8, §9, §11)

## Global Constraints

- Target Windows 11; CI runs on `windows-latest`. No admin rights needed for anything in this phase.
- Server binds **127.0.0.1 only**. Port: first free port in **47800–47809** unless `--port` is given.
- Every request passes a **Host** check (`127.0.0.1:<port>` or `localhost:<port>`) and an **Origin** check (absent, or `http://127.0.0.1:<port>` / `http://localhost:<port>` / a `--dev-origin`). WebSocket upgrades too.
- Every `/api/*` route except `POST /api/session` requires a valid session cookie `nl_session` (`HttpOnly; SameSite=Strict; Path=/`). Launch tokens are one-time.
- `API_VERSION = 1`: header `x-netlantern-api-version: 1` on every HTTP response; field `api_version: 1` on every WebSocket message.
- Error body is always `{"code": "...", "message": "...", "hint": "..."|null}` (plus `scan_id` on `scan_in_progress`).
- Quick-scan defaults (spec §6.12): ICMP concurrency **64**, timeout **800 ms**, retries **1**; largest automatic sweep **/22**, else the **/24 around the laptop**.
- Never parse OS tool text output. Never log tokens or session ids.
- UI: palette as CSS variables (default cyan; `data-palette="green-red"` alternative); status always shown with text, not color alone; touch targets ≥ 44 px; works at phone width.

## Review Focus

1. **Reloading or bookmarking the dashboard after the one-time token was used** → must keep working on the existing cookie, not show an error. Test in Task 9 (`ensureSession` token-already-used case) and Task 11 (reload e2e).
2. **Quick scan clicked while a scan is already running (double click, second tab)** → server answers 409 with the running `scan_id`; UI attaches to that scan instead of showing an error. Tests in Task 6 and Task 10.
3. **WebSocket drops (laptop sleeps, phone tab backgrounded) and misses events** → client sees the sequence gap or reconnect and reloads state over REST. Tests in Task 6 (`hello` carries the current seq) and Task 10 (`track` gap cases).
4. **Default port already taken by another program** → next port in the range, with the actual URL printed; all ten taken → a clear message, not a panic. Test in Task 7.
5. **Laptop not connected (no interface/gateway)** → `GET /api/network` and `POST /api/scans` return 503 `not_connected`; the dashboard shows "Not connected". Tests in Tasks 4, 6, 10 using `scenarios/offline.json`.

---

### Task 1: Workspace, repo and `MacAddr`

**Files:**
- Create: `Cargo.toml` (workspace: members `crates/*`, `resolver = "2"`, shared `[workspace.dependencies]` with the versions above), `rust-toolchain.toml` (`channel = "stable"`, components `rustfmt`, `clippy`), `.gitignore` (`/target`, `ui/node_modules`, `ui/dist`, `ui/test-results`, `ui/playwright-report`), `crates/netlantern-platform/{Cargo.toml,src/lib.rs,src/types.rs}`, and empty-lib skeletons `crates/netlantern-core`, `crates/netlantern-server`, `crates/netlantern` (bin, `fn main() {}`) so the workspace builds.
- Test: unit tests in `crates/netlantern-platform/src/types.rs`

**Interfaces:**
- Produces: `netlantern_platform::MacAddr(pub [u8; 6])` with `FromStr` (accepts `:` or `-`, any case), `Display` (`"50:C7:BF:3A:11:02"`, uppercase, colons), `is_locally_administered(&self) -> bool` (bit `0x02` of the first byte), serde as that string; `ParseMacError`.

- [ ] **Step 1:** `git init` in `C:\Users\vighn\projects\netlantern`, create the files above.
- [ ] **Step 2: Write the failing tests**

```rust
#[test] fn parses_colon_and_dash_forms() {
    let a: MacAddr = "50:c7:bf:3a:11:02".parse().unwrap();
    let b: MacAddr = "50-C7-BF-3A-11-02".parse().unwrap();
    assert_eq!(a, b);
    assert_eq!(a.to_string(), "50:C7:BF:3A:11:02");
}
#[test] fn rejects_short_or_bad_hex() {
    assert!("50:C7".parse::<MacAddr>().is_err());
    assert!("50:C7:BF:3A:11:ZZ".parse::<MacAddr>().is_err());
}
#[test] fn detects_private_mac() {
    assert!("DA:A1:19:5C:2E:01".parse::<MacAddr>().unwrap().is_locally_administered());
    assert!(!"50:C7:BF:3A:11:02".parse::<MacAddr>().unwrap().is_locally_administered());
}
#[test] fn serializes_as_string() {
    let m: MacAddr = "50:C7:BF:3A:11:02".parse().unwrap();
    assert_eq!(serde_json::to_string(&m).unwrap(), "\"50:C7:BF:3A:11:02\"");
    assert_eq!(serde_json::from_str::<MacAddr>("\"50:c7:bf:3a:11:02\"").unwrap(), m);
}
```

- [ ] **Step 3:** Run `cargo test -p netlantern-platform` → FAIL (`MacAddr` not found).
- [ ] **Step 4:** Implement `MacAddr` in `types.rs`, re-export from `lib.rs`.
- [ ] **Step 5:** Run `cargo test -p netlantern-platform` → PASS; `cargo clippy --workspace -- -D warnings` clean.
- [ ] **Step 6: Commit** (include the existing `docs/`):

```bash
git add -A
git commit -m "chore: workspace scaffold and MacAddr type"
```

---

### Task 2: `Platform` and `NetIo` traits + simulator

**Files:**
- Create: `crates/netlantern-platform/src/{platform.rs,netio.rs,sim/mod.rs,sim/scenario.rs}`, `scenarios/home.json`, `scenarios/offline.json`
- Test: `crates/netlantern-platform/tests/sim.rs`

**Interfaces:**
- Consumes: `MacAddr` (Task 1)
- Produces (all re-exported from `netlantern_platform`):
  - `Interface { ip: Ipv4Addr, prefix: u8, gateway: Ipv4Addr, dns_servers: Vec<Ipv4Addr>, kind: InterfaceKind, mac: MacAddr }`; `enum InterfaceKind { Wifi, Ethernet, Other }` (serde `snake_case`)
  - `PingReply { rtt: Duration, ttl: u8 }`
  - `ArpEntry { ip: Ipv4Addr, mac: MacAddr, state: NeighborState }`; `enum NeighborState { Reachable, Stale, Delay, Probe, Incomplete, Unreachable, Permanent }` with `fn is_present(&self) -> bool` (false only for `Incomplete`, `Unreachable`)
  - `enum TcpProbe { Open, Closed, Filtered }`
  - `enum PlatformError { NotConnected, PermissionNeeded { what: String }, Unavailable { reason: String }, Io { message: String } }` (thiserror)
  - `#[async_trait] trait Platform: Send + Sync` with `active_interface() -> Result<Interface, PlatformError>`, `ping(ip, timeout: Duration) -> Result<Option<PingReply>, PlatformError>`, `arp_resolve(ip, timeout) -> Result<Option<MacAddr>, PlatformError>`, `arp_table() -> Result<Vec<ArpEntry>, PlatformError>`, `tcp_probe(ip, port: u16, timeout) -> Result<TcpProbe, PlatformError>`. (Wi-Fi and firewall methods arrive in phases 2 and 6.)
  - `HttpResponse { status: u16, body_prefix: String }`; `#[async_trait] trait NetIo: Send + Sync` with `dns_resolve(server: Ipv4Addr, name: &str) -> Result<Vec<Ipv4Addr>, PlatformError>`, `http_get(url: &str, follow_redirects: bool) -> Result<HttpResponse, PlatformError>`. (mDNS/SSDP/NetBIOS arrive in phase 1.)
  - `sim::SimNetwork` implementing both traits; `SimNetwork::from_json(&str) -> Result<SimNetwork, ScenarioError>`, `SimNetwork::load(path) -> Result<..>`.

**Scenario format** (exact field names):
```json
{ "name": "home",
  "interface": { "ip": "192.168.1.23", "prefix": 24, "gateway": "192.168.1.1",
                 "dns": ["192.168.1.1"], "kind": "wifi", "mac": "F4:8E:38:9A:20:7C" },
  "devices": [ { "ip": "192.168.1.1", "mac": "50:C7:BF:3A:11:02", "ping_ms": 2, "ttl": 64, "open_ports": [53, 80, 443] } ],
  "dns": { "google.com": ["142.250.183.14"] },
  "http": { "http://www.msftconnecttest.com/connecttest.txt": { "status": 200, "body": "Microsoft Connect Test" } } }
```
`interface: null` means not connected. Device fields: `ping_ms` null → doesn't answer ping; `arp` defaults to `true`; `ttl` defaults to 64; `open_ports` defaults to `[]`.

**`scenarios/home.json` devices** (13 present; the printer `.40` and Xiaomi `.114` from the mockup are absent = offline):

| ip | mac | ping_ms | ttl | open_ports |
|---|---|---|---|---|
| .1 | 50:C7:BF:3A:11:02 | 2 | 64 | 53, 80, 443 |
| .23 (this laptop) | F4:8E:38:9A:20:7C | 0 | 128 | |
| .31 | 3C:28:6D:91:4E:B0 | 12 | 64 | |
| .44 | DA:A1:19:5C:2E:01 | 18 | 64 | |
| .52 | 8C:79:F5:0B:6A:31 | 8 | 64 | 8001, 8002 |
| .57 | F0:18:98:7C:3D:5E | 15 | 64 | |
| .61 | 44:65:0D:A2:9B:17 | 22 | 64 | |
| .70 | 00:11:32:AB:CD:70 | 4 | 64 | 22, 445, 5000 |
| .88 | 24:6F:28:3A:9F:21 | null | | 80 |
| .95 | 00:E4:21:6B:8C:40 | 6 | 64 | |
| .102 | 54:E0:19:2D:77:A8 | 41 | 64 | |
| .120 | 98:FA:9B:44:02:6D | 9 | 128 | |
| .130 | EC:B5:FA:12:88:0C | 5 | 64 | 80, 443 |

(all `192.168.1.x`). `scenarios/offline.json`: `{"name":"offline","interface":null,"devices":[],"dns":{},"http":{}}`.

**Sim behaviour:** `ping` sleeps `ping_ms` and returns the reply; a silent or absent address sleeps the full `timeout` and returns `Ok(None)`. `arp_table` returns every device with `arp: true` as `Reachable`. `arp_resolve` returns its MAC or `None`. `tcp_probe`: port in `open_ports` → `Open`; device exists → `Closed`; absent → `Filtered`. Any `Platform` call when `interface` is null → `Err(NotConnected)`. Unknown DNS name or URL → `Err(Io)`.

- [ ] **Step 1: Write the failing tests** (`#[tokio::test(start_paused = true)]`, load `../../scenarios/*.json` via `env!("CARGO_MANIFEST_DIR")`):

```rust
// home
assert_eq!(sim.arp_table().await.unwrap().len(), 13);
let r = sim.ping(ip("192.168.1.1"), ms(800)).await.unwrap().unwrap();
assert_eq!((r.rtt, r.ttl), (ms(2), 64));
assert!(sim.ping(ip("192.168.1.40"), ms(800)).await.unwrap().is_none());
assert!(sim.ping(ip("192.168.1.88"), ms(800)).await.unwrap().is_none());
assert_eq!(sim.arp_resolve(ip("192.168.1.88"), ms(800)).await.unwrap(), Some(mac("24:6F:28:3A:9F:21")));
assert_eq!(sim.tcp_probe(ip("192.168.1.70"), 5000, ms(800)).await.unwrap(), TcpProbe::Open);
assert_eq!(sim.tcp_probe(ip("192.168.1.70"), 9999, ms(800)).await.unwrap(), TcpProbe::Closed);
assert_eq!(sim.dns_resolve(ip("1.1.1.1"), "google.com").await.unwrap(), vec![ip("142.250.183.14")]);
assert!(sim.http_get("http://unknown.test/", false).await.is_err());
// offline
assert!(matches!(offline.active_interface().await, Err(PlatformError::NotConnected)));
```

- [ ] **Step 2:** `cargo test -p netlantern-platform --test sim` → FAIL (missing types).
- [ ] **Step 3:** Implement the types, traits and `SimNetwork` (serde structs for the scenario; `tokio::time::sleep` for delays).
- [ ] **Step 4:** `cargo test -p netlantern-platform` → PASS.
- [ ] **Step 5: Commit** — `feat(platform): Platform/NetIo traits and JSON scenario simulator`

---

### Task 3: Core model, sweep targets and event bus

**Files:**
- Create: `crates/netlantern-core/src/{model.rs,targets.rs,events.rs}`; unit tests inline.

**Interfaces:**
- Consumes: `MacAddr`, `Interface` (Tasks 1–2)
- Produces:
  - `pub const API_VERSION: u32 = 1;`
  - `ScanId(pub u64)` (serde as number); `enum ScanMode { Quick, Deep }`, `enum ScanState { Running, Done, Cancelled, Failed }`, `enum ScanStage { Sweep, Arp, Merge }` — serde `snake_case`
  - `DeviceSummary { ip: Ipv4Addr, mac: Option<MacAddr>, private_mac: bool, answers_ping: bool, rtt_ms: Option<u32>, is_self: bool, is_gateway: bool }` (Serialize, Deserialize, Clone, PartialEq)
  - `pub const MAX_SWEEP_PREFIX: u8 = 22;` `pub fn scan_targets(ip: Ipv4Addr, prefix: u8) -> Vec<Ipv4Addr>` — host addresses in ascending order, excluding network and broadcast; prefix < 22 → the /24 containing `ip`; prefix ≥ 31 → `vec![ip]`
  - `Event { seq: u64, api_version: u32, #[serde(flatten)] kind: EventKind }`; `#[serde(tag = "type", content = "data")] enum EventKind` with variants renamed `"scan.progress"` `{scan_id, done: u32, total: u32, stage: ScanStage}`, `"scan.device"` `{scan_id, device: DeviceSummary}`, `"scan.done"` `{scan_id, state: ScanState, devices_found: u32}`
  - `EventBus::new(capacity: usize)`, `publish(&self, kind: EventKind) -> Event` (seq starts at 1, +1 per publish), `subscribe(&self) -> broadcast::Receiver<Event>`, `last_seq(&self) -> u64`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn slash24() { let t = scan_targets(ip("192.168.1.23"), 24);
    assert_eq!((t.len(), t[0], *t.last().unwrap()), (254, ip("192.168.1.1"), ip("192.168.1.254"))); }
#[test] fn slash22() { let t = scan_targets(ip("10.0.5.9"), 22);
    assert_eq!((t.len(), t[0], *t.last().unwrap()), (1022, ip("10.0.4.1"), ip("10.0.7.254"))); }
#[test] fn larger_than_22_scans_own_24() { let t = scan_targets(ip("10.0.5.9"), 16);
    assert_eq!((t.len(), t[0], *t.last().unwrap()), (254, ip("10.0.5.1"), ip("10.0.5.254"))); }
#[test] fn point_to_point() { assert_eq!(scan_targets(ip("10.0.0.1"), 32), vec![ip("10.0.0.1")]); }
#[test] fn seq_increments_from_one() { let bus = EventBus::new(16);
    let a = bus.publish(done(1)); let b = bus.publish(done(1));
    assert_eq!((a.seq, b.seq, bus.last_seq()), (1, 2, 2)); }
#[test] fn event_json_shape() { let bus = EventBus::new(16);
    let v = serde_json::to_value(bus.publish(done(7))).unwrap();
    assert_eq!(v, serde_json::json!({"seq":1,"api_version":1,"type":"scan.done",
        "data":{"scan_id":7,"state":"done","devices_found":13}})); }
```
(`done(id)` builds `EventKind::ScanDone { scan_id: ScanId(id), state: ScanState::Done, devices_found: 13 }`.)

- [ ] **Step 2:** `cargo test -p netlantern-core` → FAIL.
- [ ] **Step 3:** Implement `model.rs`, `targets.rs`, `events.rs` (`AtomicU64` for seq; publish ignores "no receivers").
- [ ] **Step 4:** `cargo test -p netlantern-core` → PASS.
- [ ] **Step 5: Commit** — `feat(core): model types, sweep targets, event bus with sequence numbers`

---

### Task 4: Quick-scan skeleton and device store

Phase-0 scope of spec §6.2: steps 1 (ICMP sweep) and 3 (ARP table read) and 5–6 (merge, stream). TCP probes, `arp_resolve` for entry-less addresses, discovery protocols and identification come in phase 1.

**Files:**
- Create: `crates/netlantern-core/src/{devices.rs,scan.rs,error.rs}`
- Test: `crates/netlantern-core/tests/scan_sim.rs` (dev-dependency on `netlantern-platform`)

**Interfaces:**
- Consumes: `Platform`, `SimNetwork`, `EventBus`, `scan_targets`, model types (Tasks 2–3)
- Produces:
  - `DeviceStore` (`Default`): `upsert(&self, d: DeviceSummary)` keyed by MAC when present, else IP; `list(&self) -> Vec<DeviceSummary>` sorted by IP
  - `ScanConfig { icmp_concurrency: usize, icmp_timeout: Duration, icmp_retries: u32 }` with `Default` = 64 / 800 ms / 1
  - `enum CoreError { NotConnected, Unsupported(&'static str), Platform(PlatformError) }` — `PlatformError::NotConnected` maps to `CoreError::NotConnected`
  - `enum StartOutcome { Started(ScanId), AlreadyRunning(ScanId) }`
  - `ScanSnapshot { id: ScanId, mode: ScanMode, state: ScanState, stage: ScanStage, done: u32, total: u32, devices_found: u32 }` (Serialize)
  - `ScanManager::new(platform: Arc<dyn Platform>, bus: Arc<EventBus>, devices: Arc<DeviceStore>, config: ScanConfig)`; `async fn start(&self, mode: ScanMode) -> Result<StartOutcome, CoreError>` (Deep → `Unsupported("deep scan arrives in phase 4")`; interface read before spawning so `NotConnected` is returned synchronously); `fn snapshot(&self, id: ScanId) -> Option<ScanSnapshot>`

**Algorithm:** ping every target with `buffer_unordered(icmp_concurrency)` (one retry on `None`); publish `scan.progress` when `done % 16 == 0` or `done == total`; stage `Arp`: read `arp_table`; stage `Merge`: a target is present if it replied or has an entry with `is_present()`; `mac` from ARP (the laptop's own from `Interface.mac`), `private_mac = mac.is_locally_administered()`, `is_self = ip == interface.ip`, `is_gateway = ip == interface.gateway`; upsert, publish `scan.device` per device, then `scan.done`. Scan ids start at 1.

- [ ] **Step 1: Write the failing tests** (`#[tokio::test(start_paused = true)]`, home scenario unless stated):

```rust
// completes with all 13 present devices
let id = started(mgr.start(ScanMode::Quick).await.unwrap());
let events = collect_until_done(&mut rx).await;
assert!(events.windows(2).all(|w| w[1].seq == w[0].seq + 1));
assert!(matches!(events.last().unwrap().kind, EventKind::ScanDone { devices_found: 13, state: ScanState::Done, .. }));
assert_eq!(mgr.snapshot(id).unwrap().state, ScanState::Done);
let list = store.list();
let quiet = list.iter().find(|d| d.ip == ip("192.168.1.88")).unwrap();
assert!(!quiet.answers_ping && quiet.mac.is_some());
assert!(list.iter().find(|d| d.ip == ip("192.168.1.23")).unwrap().is_self);
assert!(list.iter().find(|d| d.ip == ip("192.168.1.1")).unwrap().is_gateway);
assert!(list.iter().find(|d| d.ip == ip("192.168.1.44")).unwrap().private_mac);
// second start while running returns the same id
assert_eq!(mgr.start(ScanMode::Quick).await.unwrap(), StartOutcome::AlreadyRunning(id));
// offline scenario
assert!(matches!(offline_mgr.start(ScanMode::Quick).await, Err(CoreError::NotConnected)));
// deep
assert!(matches!(mgr.start(ScanMode::Deep).await, Err(CoreError::Unsupported(_))));
```

- [ ] **Step 2:** `cargo test -p netlantern-core --test scan_sim` → FAIL.
- [ ] **Step 3:** Implement `devices.rs`, `error.rs`, `scan.rs` (running scan guarded by a `Mutex<Option<ScanId>>`; snapshots in a `Mutex<HashMap<ScanId, ScanSnapshot>>`).
- [ ] **Step 4:** `cargo test -p netlantern-core` → PASS.
- [ ] **Step 5: Commit** — `feat(core): quick-scan skeleton (ICMP sweep + ARP read) and device store`

---

### Task 5: Server auth, guard and structured errors

**Files:**
- Create: `crates/netlantern-server/src/{auth.rs,guard.rs,error.rs}`
- Test: `crates/netlantern-server/tests/guard.rs`

**Interfaces:**
- Produces:
  - `ApiError { status: StatusCode, code: &'static str, message: String, hint: Option<String>, scan_id: Option<u64> }` implementing `IntoResponse` (JSON body; `scan_id` omitted when `None`) with constructors `unauthorized()` 401, `forbidden_host()` 403, `forbidden_origin()` 403, `not_connected()` 503 (hint: "Connect to a Wi-Fi or Ethernet network, then try again."), `scan_in_progress(id)` 409, `unsupported(msg)` 501, `not_found()` 404, `bad_request(msg)` 400
  - `Auth::new()`, `issue_launch_token(&self) -> String` (64 hex chars from 32 random bytes), `redeem(&self, token: &str) -> Option<String>` (one-time; returns a new 64-hex session id), `is_session(&self, id: &str) -> bool`
  - `AccessPolicy { allowed_hosts: Vec<String>, allowed_origins: Vec<String> }` with `AccessPolicy::for_port(port: u16, dev_origins: &[String]) -> Self`
  - `pub async fn guard(State<GuardState>, Request, Next) -> Response` where `GuardState { auth: Arc<Auth>, policy: Arc<AccessPolicy> }`; order: Host → Origin → session (for `/api/*` except `POST /api/session`); always adds `x-netlantern-api-version: 1`

- [ ] **Step 1: Write the failing tests** (a test router with `GET /api/ping` and `GET /` behind `guard`, driven by `tower::ServiceExt::oneshot`; port 47800):

```rust
assert_eq!(auth.redeem(&t).is_some(), true); assert!(auth.redeem(&t).is_none());   // one-time
assert!(auth.redeem("nope").is_none());
// foreign Host (DNS rebinding)
assert_status_code(send(get("/").host("evil.example:47800")), 403, "forbidden_host");
// foreign Origin
assert_status_code(send(get("/").host("127.0.0.1:47800").origin("http://evil.example")), 403, "forbidden_origin");
// API without session
assert_status_code(send(get("/api/ping").host("127.0.0.1:47800")), 401, "unauthorized");
// API with session, localhost host, same origin
let r = send(get("/api/ping").host("localhost:47800").origin("http://localhost:47800").cookie(&sid));
assert_eq!(r.status(), 200); assert_eq!(r.headers()["x-netlantern-api-version"], "1");
// static path needs no session
assert_eq!(send(get("/").host("127.0.0.1:47800")).status(), 200);
// dev origin allowed only when configured
assert_eq!(send_with_policy(dev_policy, get("/api/ping").host("127.0.0.1:47800").origin("http://localhost:5173").cookie(&sid)).status(), 200);
```

- [ ] **Step 2:** `cargo test -p netlantern-server --test guard` → FAIL.
- [ ] **Step 3:** Implement `error.rs`, `auth.rs` (`Mutex<HashSet<String>>` for tokens and sessions), `guard.rs`.
- [ ] **Step 4:** `cargo test -p netlantern-server` → PASS.
- [ ] **Step 5: Commit** — `feat(server): session tokens, Host/Origin guard, structured errors`

---

### Task 6: REST routes and WebSocket events

**Files:**
- Create: `crates/netlantern-server/src/{state.rs,routes.rs,ws.rs}`; modify `lib.rs`
- Test: `crates/netlantern-server/tests/api.rs` (real listener on `127.0.0.1:0`, `reqwest` with cookie store, `tokio-tungstenite`; scan config with `icmp_timeout = 50 ms` so tests stay fast)

**Interfaces:**
- Consumes: `ScanManager`, `DeviceStore`, `EventBus`, `Platform`, `Auth`, `AccessPolicy`, `guard`, `ApiError`
- Produces:
  - `AppState { platform: Arc<dyn Platform>, bus: Arc<EventBus>, scans: Arc<ScanManager>, devices: Arc<DeviceStore>, auth: Arc<Auth>, policy: Arc<AccessPolicy> }` and `AppState::new(platform: Arc<dyn Platform>, scan: ScanConfig, policy: AccessPolicy) -> Self`
  - `pub fn router(state: AppState) -> Router` with:
    - `POST /api/session {token}` → 204 + `Set-Cookie: nl_session=<id>; HttpOnly; SameSite=Strict; Path=/`; bad/used token → 401 `unauthorized`
    - `GET /api/network` → `{ip, prefix, gateway, dns_servers, kind, mac}` or 503 `not_connected`
    - `POST /api/scans {mode: "quick"|"deep"}` → 202 `{scan_id}`; running → 409 `scan_in_progress` with `scan_id`; deep → 501 `unsupported`; offline → 503 `not_connected`
    - `GET /api/scans/:id` → `ScanSnapshot` or 404
    - `GET /api/devices` → `[DeviceSummary]`
    - `GET /api/events` (WebSocket) → first message `{"type":"hello","seq":<bus.last_seq()>,"api_version":1}`, then every `Event` as JSON text. On `RecvError::Lagged`, keep going (the client detects the gap).

- [ ] **Step 1: Write the failing tests**

```rust
// session
assert_eq!(post_session(&c, &token).await.status(), 204);
assert_eq!(post_session(&c2, &token).await.status(), 401);              // reused token
// scan lifecycle
let id = post_scan(&c, "quick").await.json()["scan_id"].as_u64().unwrap();
let again = post_scan(&c, "quick").await;
assert_eq!(again.status(), 409); assert_eq!(again.json()["scan_id"], id);
wait_until(|| snapshot(&c, id).state == "done").await;
assert_eq!(snapshot(&c, id).devices_found, 13);
assert_eq!(get_devices(&c).await.len(), 13);
// websocket: hello then strictly increasing events ending in scan.done
let mut ws = connect_ws(port, Some(&cookie), Some(&format!("http://127.0.0.1:{port}"))).await.unwrap();
let hello = next_json(&mut ws).await; assert_eq!(hello["type"], "hello"); assert_eq!(hello["api_version"], 1);
// (start a scan, then:) seqs == hello.seq+1, +2, ...; last type == "scan.done"
// cross-origin and no-session upgrades refused
assert_http_status(connect_ws(port, Some(&cookie), Some("http://evil.example")).await, 403);
assert_http_status(connect_ws(port, None, Some(&format!("http://127.0.0.1:{port}"))).await, 401);
// offline scenario
assert_status_code(get_network(&offline_client).await, 503, "not_connected");
assert_status_code(post_scan(&offline_client, "quick").await, 503, "not_connected");
assert_status_code(post_scan(&c, "deep").await, 501, "unsupported");
```

- [ ] **Step 2:** `cargo test -p netlantern-server --test api` → FAIL.
- [ ] **Step 3:** Implement `state.rs`, `routes.rs`, `ws.rs` (axum `WebSocketUpgrade`; guard runs before the upgrade).
- [ ] **Step 4:** `cargo test -p netlantern-server` → PASS.
- [ ] **Step 5: Commit** — `feat(server): REST API, WebSocket event stream with hello/seq`

---

### Task 7: Embedded UI and port binding

**Files:**
- Create: `crates/netlantern-server/src/{assets.rs,serve.rs}`
- Test: `crates/netlantern-server/tests/serve.rs`

**Interfaces:**
- Produces:
  - `#[derive(RustEmbed)] #[folder = "../../ui/dist"] #[allow_missing = true] struct UiAssets` and a fallback handler added by `router()`: known file → bytes with `mime_guess` type; any other non-`/api` path → `index.html`; if `index.html` isn't embedded → 200 `text/html` page saying "The Net Lantern UI isn't built yet. Run `npm run build` in `ui/`, then rebuild."
  - `pub const DEFAULT_PORTS: RangeInclusive<u16> = 47800..=47809;`
  - `pub async fn bind(ports: RangeInclusive<u16>) -> Result<(TcpListener, u16), ServeError>` — first free port on 127.0.0.1
  - `ServeError::NoFreePort { first: u16, last: u16 }` with Display: `"Ports {first}–{last} are all in use. Close the program using them or start Net Lantern with --port <number>."`
  - `pub async fn run(listener: TcpListener, state: AppState, shutdown: impl Future<Output = ()>)`

- [ ] **Step 1: Write the failing tests**

```rust
// next free port is used
let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let p = held.local_addr().unwrap().port();
let (_l, got) = bind(p..=p + 1).await.unwrap(); assert_eq!(got, p + 1);
// all taken
let err = bind(p..=p).await.unwrap_err(); assert!(err.to_string().contains("are all in use"));
// SPA fallback serves HTML for a client route (works whether or not ui/dist exists)
let r = get_text(&format!("http://127.0.0.1:{port}/devices")).await;
assert!(r.content_type.starts_with("text/html"));
```

- [ ] **Step 2:** `cargo test -p netlantern-server --test serve` → FAIL.
- [ ] **Step 3:** Implement `assets.rs` and `serve.rs`; wire the fallback into `router()`.
- [ ] **Step 4:** `cargo test -p netlantern-server` → PASS.
- [ ] **Step 5: Commit** — `feat(server): embedded UI with SPA fallback, port range binding`

---

### Task 8: `netlantern` executable: demo mode and single instance

**Files:**
- Create: `crates/netlantern/src/{main.rs,cli.rs,instance.rs}`
- Test: unit tests in `cli.rs`; `crates/netlantern/tests/instance.rs` (`#[cfg(windows)]`)

**Interfaces:**
- Consumes: `netlantern_server::{AppState, router, bind, run, DEFAULT_PORTS}`, `SimNetwork`, `ScanConfig::default()`
- Produces:
  - CLI (clap derive) `Cli { demo: Option<Option<PathBuf>>, port: Option<u16>, no_browser: bool, dev_origin: Vec<String> }`: `--demo` (embedded `scenarios/home.json` via `include_str!`) or `--demo=<file>`, `--port <n>` (binds exactly that port), `--no-browser`, repeatable `--dev-origin <url>`
  - `enum Instance { Primary(NamedPipeServer), Secondary }`; `pub fn acquire(pipe: &str) -> io::Result<Instance>` (`ServerOptions::new().first_pipe_instance(true)`; access-denied/busy → `Secondary`); `pub async fn notify_open(pipe: &str) -> io::Result<()>` (writes `open\n`); `pub async fn serve_open_requests(first: NamedPipeServer, pipe: String, on_open: impl Fn() + Send + 'static)`
  - Pipe name: `\\.\pipe\netlantern-<USERNAME env var>` (tests use a unique name)

**Behaviour:** no `--demo` → print `Real network scanning arrives in phase 1. For now run: netlantern --demo` and exit code 2. Secondary → `notify_open`, print `Net Lantern is already running; opened its dashboard.`, exit 0. Primary → build state, bind, issue a launch token, print exactly `Net Lantern running at http://127.0.0.1:<port>/?t=<token>` on stdout (the e2e harness reads this line), open the browser unless `--no-browser`; each pipe `open` request issues a fresh token and opens the browser; Ctrl+C shuts down cleanly. Tokens never go to `tracing` logs.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn demo_flag_forms() {
    assert_eq!(Cli::parse_from(["netlantern", "--demo"]).demo, Some(None));
    assert_eq!(Cli::parse_from(["netlantern", "--demo=x.json"]).demo, Some(Some("x.json".into())));
    let c = Cli::parse_from(["netlantern", "--port", "48000", "--dev-origin", "http://localhost:5173", "--dev-origin", "http://127.0.0.1:5173"]);
    assert_eq!((c.port, c.dev_origin.len()), (Some(48000), 2));
}
// tests/instance.rs
let name = format!(r"\\.\pipe\netlantern-test-{}", std::process::id());
let Instance::Primary(server) = acquire(&name).unwrap() else { panic!("first must be primary") };
assert!(matches!(acquire(&name).unwrap(), Instance::Secondary));
let (tx, rx) = oneshot::channel(); /* serve_open_requests(server, name.clone(), move || tx.send(())) */
notify_open(&name).await.unwrap(); timeout(secs(2), rx).await.unwrap().unwrap();
```

- [ ] **Step 2:** `cargo test -p netlantern` → FAIL.
- [ ] **Step 3:** Implement `cli.rs`, `instance.rs`, `main.rs` (tracing-subscriber to stderr at `info`).
- [ ] **Step 4:** `cargo test -p netlantern` → PASS. Manual: `cargo run -p netlantern -- --demo --no-browser` prints the `Net Lantern running at` line; running it again from a second terminal prints the "already running" line and exits.
- [ ] **Step 5: Commit** — `feat: netlantern exe with demo mode and single-instance pipe`

---

### Task 9: UI scaffold, shell and session bootstrap

**Files:**
- Create: `ui/package.json` (scripts `dev`, `build` = `tsc -b && vite build`, `lint` = `eslint .`, `test` = `vitest run`, `e2e` = `playwright test`), `ui/vite.config.ts`, `ui/tsconfig.json`, `ui/eslint.config.js`, `ui/index.html` (IBM Plex Sans + Mono self-hosted via @fontsource, imported in `main.tsx` — no third-party font request), `ui/src/main.tsx`, `ui/src/App.tsx`, `ui/src/styles/{tokens.css,app.css}`, `ui/src/components/Shell.tsx`, `ui/src/pages/{Dashboard.tsx,Devices.tsx,Placeholder.tsx,NeedsLaunch.tsx}`, `ui/src/api/{client.ts,session.ts}`
- Test: `ui/src/api/session.test.ts`, `ui/src/components/Shell.test.tsx`

**Interfaces:**
- Produces:
  - `api/client.ts`: `apiFetch<T>(path: string, init?: RequestInit): Promise<ApiResult<T>>` where `type ApiResult<T> = { ok: true; status: number; data: T } | { ok: false; status: number; error: { code: string; message: string; hint: string | null; scan_id?: number } }` (`credentials: "same-origin"`)
  - `api/session.ts`: `ensureSession(loc: Location = window.location, hist: History = window.history): Promise<"ok" | "needs-launch">` — if `?t=` present, `POST /api/session {token}`, then **always** remove `t` with `hist.replaceState`; then `GET /api/network`: 200 or 503 → `"ok"`, 401 → `"needs-launch"`
  - Routes: `/` Dashboard, `/devices` Devices, `/map` (Placeholder "Arrives in phase 5"), `/diagnose` ("phase 2"), `/alerts` ("phase 3"), `/settings` ("phase 6")
  - `tokens.css`: `:root` = `--bg #0A0F14; --surface #111820; --surface-2 #16202A; --border #223040; --border-strong #2C3E50; --text #E6EDF3; --text-muted #93A4B3; --accent #46C2E8; --accent-ink #04141B; --ok #56D364; --warn #E3B341; --bad #FF7B72`; `[data-palette="green-red"]` = `--bg #050A07; --surface #0A130D; --surface-2 #112016; --border #1D3324; --border-strong #2A4A33; --text-muted #96AE9C; --accent #2BFF88; --accent-ink #02140A; --accent-2 #FF3355`. Tailwind theme colors map to these variables.
  - `vite.config.ts`: dev proxy `/api` → `http://127.0.0.1:47800` with `ws: true`; Vitest `environment: "jsdom"`. (Dev loop: `netlantern --demo --port 47800 --dev-origin http://localhost:5173` + `npm run dev`.)
  - `NeedsLaunch` page copy: "This dashboard link has expired. Start Net Lantern again and it will open a fresh one."

- [ ] **Step 1:** Scaffold the Vite React-TS app and install the dependencies from the Tech Stack.
- [ ] **Step 2: Write the failing tests**

```ts
it("redeems the token and cleans the URL", async () => {
  mockFetch({ "POST /api/session": 204, "GET /api/network": 200 });
  const hist = fakeHistory();
  expect(await ensureSession(loc("/?t=abc"), hist)).toBe("ok");
  expect(hist.replaced).toBe("/");
});
it("still works when the token was already used but the cookie is valid", async () => {
  mockFetch({ "POST /api/session": 401, "GET /api/network": 200 });
  expect(await ensureSession(loc("/devices?t=old"), fakeHistory())).toBe("ok");
});
it("asks for a fresh launch when there is no session", async () => {
  mockFetch({ "GET /api/network": 401 });
  expect(await ensureSession(loc("/"), fakeHistory())).toBe("needs-launch");
});
it("treats not_connected as a valid session", async () => {
  mockFetch({ "GET /api/network": 503 });
  expect(await ensureSession(loc("/"), fakeHistory())).toBe("ok");
});
it("shell shows six nav links and marks the active one", () => {
  renderAt("/devices");
  expect(screen.getAllByRole("link")).toHaveLength(6);
  expect(screen.getByRole("link", { name: "Devices" })).toHaveAttribute("aria-current", "page");
});
```

- [ ] **Step 3:** `npm test` in `ui/` → FAIL.
- [ ] **Step 4:** Implement `client.ts`, `session.ts`, `Shell.tsx` (sidebar as in the mockup; it stacks above the content below 760 px), the pages and tokens. `App` calls `ensureSession` before rendering routes.
- [ ] **Step 5:** `npm test` → PASS; `npm run lint` and `npm run build` succeed.
- [ ] **Step 6: Commit** — `feat(ui): app shell, design tokens, session bootstrap`

---

### Task 10: Live events, Dashboard scan and Devices table

**Files:**
- Create: `ui/src/api/{events.ts,queries.ts}`, `ui/src/state/live.ts`; modify `ui/src/pages/{Dashboard.tsx,Devices.tsx}`
- Test: `ui/src/api/events.test.ts`, `ui/src/pages/Dashboard.test.tsx`, `ui/src/pages/Devices.test.tsx`

**Interfaces:**
- Consumes: `apiFetch`, server routes (Task 6)
- Produces:
  - `type ServerEvent = { seq: number; api_version: number; type: string; data?: unknown }`
  - `track(lastSeq: number | null, ev: ServerEvent): { lastSeq: number; gap: boolean }` — `hello` sets the baseline (`gap: false`); otherwise `gap = lastSeq === null || ev.seq !== lastSeq + 1`
  - `connectEvents(handlers: { onEvent(ev: ServerEvent): void; onResync(): void; onVersionMismatch(): void }): () => void` — WebSocket to `/api/events` (same host, `ws:`), reconnect with backoff 1 s doubling to 10 s; calls `onResync` after every (re)connect `hello` and on any gap; `api_version !== 1` → `onVersionMismatch`
  - `queries.ts`: `useNetwork()`, `useDevices()` (TanStack Query keys `["network"]`, `["devices"]`), `startScan(mode): Promise<number>` (on 409 returns the error's `scan_id`)
  - `state/live.ts` (zustand): `{ scan: { id: number; state: string; stage: string; done: number; total: number; devicesFound: number } | null; outdated: boolean; applyEvent(ev: ServerEvent): void; attach(scanId: number): void }`
  - Dashboard: network card (`192.168.1.23 · 192.168.1.0/24 · gateway 192.168.1.1`); "Not connected" card on 503; **Quick scan** button (shows "Scanning…" and is disabled while running); **Deep scan** button disabled with visible text "Arrives in phase 4"; `role="progressbar"` with `aria-valuenow` = percent; text "`N` devices found" when done; "This dashboard is out of date — reload" banner when `outdated`
  - Devices: table rows sorted by IP; columns Name (IP for now), IP, MAC (shows "Private address" when `private_mac`), Latency (`rtt_ms` + " ms" or "—"), Status text "Online" or "Online · doesn't answer ping"; badges "This device" and "Router"

- [ ] **Step 1: Write the failing tests**

```ts
it("hello sets the baseline; consecutive is fine; a skip is a gap", () => {
  let s = track(null, ev("hello", 5)); expect(s).toEqual({ lastSeq: 5, gap: false });
  s = track(s.lastSeq, ev("scan.progress", 6)); expect(s.gap).toBe(false);
  s = track(s.lastSeq, ev("scan.progress", 8)); expect(s.gap).toBe(true);
});
it("an event before any hello counts as a gap", () => {
  expect(track(null, ev("scan.done", 3)).gap).toBe(true);
});
it("Quick scan attaches to the running scan on 409", async () => {
  mockFetch({ "POST /api/scans": [409, { code: "scan_in_progress", message: "", hint: null, scan_id: 7 }] });
  renderDashboard(); await user.click(screen.getByRole("button", { name: "Quick scan" }));
  expect(useLive.getState().scan?.id).toBe(7);
  expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();
});
it("shows Not connected on 503", async () => {
  mockFetch({ "GET /api/network": [503, { code: "not_connected", message: "", hint: "Connect…" }] });
  renderDashboard(); expect(await screen.findByText("Not connected")).toBeInTheDocument();
});
it("lists devices with text status", async () => {
  mockFetch({ "GET /api/devices": [200, [dev("192.168.1.1", { is_gateway: true, rtt_ms: 2 }), dev("192.168.1.88", { answers_ping: false })]] });
  renderDevices();
  expect(await screen.findByText("Online · doesn't answer ping")).toBeInTheDocument();
  expect(screen.getByText("Router")).toBeInTheDocument();
});
```

- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implement `events.ts`, `queries.ts`, `live.ts`, update the pages; `onResync` invalidates `["network"]` and `["devices"]` and, if a scan is attached, reloads `GET /api/scans/:id`; `scan.device` and `scan.done` events invalidate `["devices"]`.
- [ ] **Step 4:** `npm test` → PASS; `npm run lint && npm run build` succeed.
- [ ] **Step 5: Commit** — `feat(ui): live scan progress over WebSocket, devices table`

---

### Task 11: End-to-end demo test, CI and README

**Files:**
- Create: `ui/playwright.config.ts`, `ui/e2e/global-setup.ts`, `ui/e2e/demo.spec.ts`, `.github/workflows/ci.yml`, `README.md`

**Interfaces:**
- Consumes: the `Net Lantern running at <url>` stdout line (Task 8)
- Produces: `global-setup.ts` spawns `../target/debug/netlantern.exe --demo --no-browser --port 47850`, waits for that line (timeout 30 s), saves the URL to `process.env.NETLANTERN_URL`, and kills the process in teardown

- [ ] **Step 1: Write the failing e2e test** (`demo.spec.ts`, Chromium):

```ts
test("demo: scan finds the simulated network", async ({ page }) => {
  await page.goto(process.env.NETLANTERN_URL!);
  await expect(page.getByText("192.168.1.0/24")).toBeVisible();
  await page.getByRole("button", { name: "Quick scan" }).click();
  await expect(page.getByText("13 devices found")).toBeVisible({ timeout: 30_000 });
  await page.getByRole("link", { name: "Devices" }).click();
  await expect(page.getByRole("row", { name: /192\.168\.1\.88/ })).toContainText("doesn't answer ping");
});
test("reload after the one-time token is used keeps working", async ({ page }) => {
  await page.goto(process.env.NETLANTERN_URL!);
  await expect(page).not.toHaveURL(/\?t=/);
  await page.reload();
  await expect(page.getByRole("button", { name: "Quick scan" })).toBeVisible();
});
```

- [ ] **Step 2:** `cargo build -p netlantern` then `npm run e2e` in `ui/` → PASS once Tasks 1–10 are in. (If it fails, fix the cause in the owning task's code, not in the test.)
- [ ] **Step 3:** Write `ci.yml` (`windows-latest`, on push and pull request): checkout; Node 22 with npm cache; `npm ci`, `npm run lint`, `npm test`, `npm run build` in `ui/`; Rust stable with rustfmt and clippy; `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo build -p netlantern`; `npx playwright install --with-deps chromium`; `npm run e2e`.
- [ ] **Step 4:** Write `README.md`: one-line pitch (spec §1.1); status "Phase 0: demo mode only"; quick start (`npm ci && npm run build` in `ui/`, then `cargo run -p netlantern -- --demo`); dev loop (two commands from Task 9); repo layout (spec §5); link to the spec.
- [ ] **Step 5:** Run the full local check: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`, then in `ui/`: `npm run lint && npm test && npm run build && npm run e2e` → all PASS.
- [ ] **Step 6: Commit** — `test: e2e demo journey; ci: windows workflow; docs: README`

---

## Phase 0 review checkpoint (spec §13)

- [ ] **Demo:** run `netlantern --demo` on the author's machine. Show the scan, the devices table, a second launch opening the same dashboard, and a reload after the token was used.
- [ ] **Done-when (spec §12):** the dashboard shows the simulated devices. The cross-origin WebSocket test in Task 6 passes (a page on another origin can't open the WebSocket).
- [ ] **Summary** for the author: what each crate does and why the traits exist.
- [ ] **Learning check:** ask the author to explain two or three of the phase 0 goals in their own words: the traits as a seam for real vs simulated code; tokio tasks and the broadcast channel; why a localhost server needs a token plus Host/Origin checks (DNS rebinding, cross-site WebSocket hijacking). Walk through any gaps in the code.
- [ ] **Next:** write the Phase 1 plan (Windows platform, discovery, identification, SQLite storage).
