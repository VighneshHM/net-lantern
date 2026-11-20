# Phase 6: Phone Access and Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a phone open the dashboard over Wi-Fi through a QR code and token. Detect a Public network and a blocked Windows Firewall rule, and explain both. Add the Settings screen, a first-run card, the "new router on a known network" prompt, and a responsive pass over every screen.

**Architecture:**
- **Platform:** gains two read-only capabilities: `network_category()`, which reads Windows' Public/Private/Domain mark, and `firewall_status()`, which reads the firewall rules for this exe. Both have real Windows (COM) and simulated versions.
- **Phone listener:** a `PhoneAccess` manager in the server runs a second listener on the laptop's LAN address, serving the same router. A reusable phone token is exchanged for a phone session. Turning phone access off rotates the token, ends phone sessions and closes their WebSockets.
- **Guard:** the Host/Origin guard learns the phone address only while the listener is up.
- **Network identity:** gains the SSID label and a "replaced router" decision that blocks identification until the user answers.
- **UI:** gains the Settings screen, the first-run card and the router prompt.

**Tech Stack:**
- Rust: tokio, axum, rusqlite, `windows` 0.62 COM: `INetworkListManager` and `INetFwPolicy2`.
- UI: React 19 + TypeScript, TanStack Query, Tailwind 4, the `qrcode` npm package (new), Vitest and Playwright.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`. The relevant sections are:
- §5.1: `firewall_status`.
- §6.1: identity and the replaced router.
- §6.11: access and security.
- §6.12: Server defaults.
- §8: API and errors.
- §9: Settings, first-run and look.
- §9.1 C: café journey.
- §10: degraded states.
- §11: security tests.
- §12: the phase 6 row.
- §13: the checkpoint and learning goals.

**Branch:** `phase-6-phone-settings`, created from `main` at `a3cbd22`, after Phases 4 and 5 merged.

## Decisions this plan makes (for the author's review)

1. **What Settings holds in Phase 6:**
   - phone access;
   - this network (Windows category and deep-scan permission, which can now be withdrawn; Phase 4 had no UI for that);
   - a link to the alert rules, where desktop notifications already live (Phase 3);
   - retention shown read-only.

   Spec §9 also lists **update check**, **copy debug info** and **data reset**. These move to Phase 7: the first two are in the Phase 7 row, and data reset belongs with the installer's data-removal choice. **Scan schedule** is dropped from v1, because there is no scheduled scanning; monitoring's presence refresh covers it. Start at login is also Phase 7, since it needs the Tauri app.
2. **The phone token is reusable while phone access is on**, so a second phone can scan the same QR. It is regenerated at every start and on every "off". Sessions are in memory, so a restart also ends phone sessions. The "on" setting itself is saved, and phone access comes back after a restart, with a new QR.
3. **Phone sessions** have their own cap of 32, so phone sign-ins can never push out the laptop's session.
4. **A firewall block is a state, not a request failure.** The firewall can only be judged after the user answers the Windows prompt. So `POST /api/phone-access` succeeds, and `firewall` in the status reads `blocked`. Spec §8 lists `firewall_blocked` as an error code; this plan reports it as a status value instead and records that in the spec.
5. **Public network.** Turning phone access on while Windows marks the network Public fails with `409 public_network`. If a network that is already on becomes Public, or the connection drops, phone access **pauses**. The listener closes and phone sessions end. It resumes on its own when the laptop is on a Private (or Domain) network again. The Domain category counts as Private.
6. **Demo mode** binds the phone listener to `127.0.0.1` on a free port. The URL then works on the laptop only, and the page says so. This lets the whole flow run in `--demo` and in Playwright.
7. **Replaced router.** Identification never silently splits a known network. A gateway MAC can be new while a known network has the same SSID and subnet. Until the user answers, scans and deep-scan consent fail:
   - the scan start returns `409 network_decision_needed`;
   - a scan already running fails with the text "This looks like a network you know with a new router."

   "Keep" moves the old network to the new gateway MAC; "New network" creates a separate record. If the SSID is unknown (location permission off), no prompt is possible and a new network is created, as today.

## Global Constraints

- Spec §6.11:
  - "Bind: 127.0.0.1 only by default."
  - "Host check: requests whose `Host` header isn't the server's own address (`127.0.0.1:<port>`, `localhost:<port>`, or the LAN address while phone access is on) are refused."
  - "Origin check: requests and WebSocket upgrades that carry an `Origin` other than the server's own are refused."
  - "Phone access: Settings → 'Allow phone access' binds to the LAN, generates a random access token, and shows a QR code with the URL + token, exchanged for a session cookie the same way. Turning it off rotates the token and ends every phone session."
  - "Plain HTTP on the LAN in v1 (the token is the protection)."
  - "Turning it on the first time triggers the Windows Firewall prompt, and Net Lantern explains the prompt beforehand. If the prompt is dismissed, Windows silently adds a blocking rule and never asks again; Net Lantern reads the rules (`firewall_status`) after enabling and, if blocked, explains how to allow it."
  - "On networks Windows marks **Public**, phone access stays off; Net Lantern says why and how to mark the network Private."
- Spec §12, done when: "Phone opens the dashboard via QR; refused without token; dismissed firewall prompt is detected and explained."
- Spec §10: "No location permission / firewall blocked / Public network → feature-level degraded state with an explanation and the one action that fixes it; never a hard failure of the whole scan."
- Tokens and session ids are never logged, formatted into errors, or printed. The exception is the launch URL the exe already prints, and the phone URL shown to the signed-in user.
- The firewall and network-list reads are **read-only**. Net Lantern never adds, changes or removes firewall rules or network categories (the author's rule: never change Windows settings).
- Errors are structured `{code, message, hint}`. New codes: `public_network` (409), `network_decision_needed` (409), `phone_access_unavailable` (503, for a bind failure or no network), `no_decision_pending` (404).
- UI: design tokens only, no raw colours. Touch targets are at least 44 px. Status always uses text, not colour alone. Every new string is plain English.
- Commits are one plain sentence, with no prefix, no body, and no Co-Authored-By or Claude attribution (the author's rule).
- Build with `CARGO_TARGET_DIR=C:/Users/vighn/AppData/Local/Temp/nl-target`. Never kill a `netlantern.exe` you did not start. Use `NETLANTERN_PIPE_NAME=<unique>` for any demo you run. Cargo may need `export PATH="$HOME/.cargo/bin:$PATH"`.
- Per-task verification:
  - Rust tasks: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
  - UI tasks: `npm run lint`, `npx tsc -b` and `npm test` from `ui/`.

## Review Focus

1. **Turning phone access off with a phone still connected.** Its open WebSocket and keep-alive HTTP connection must stop working at once: the socket closes, and the next request gets 401. The listener must not hang on graceful shutdown. Tests: Task 5, `revoking phone sessions closes their websockets`, and Task 6, `disable ends phone sessions and stops within the grace`.
2. **The laptop moves networks or its IP changes while phone access is on.** The listener must follow the new address or pause, and never keep serving on a dead or wrong address. The guard must stop accepting the old LAN host. Test: Task 6, `rebinds when the address changes and pauses on a public network`.
3. **Guessing or reusing tokens across listeners.**
   - A launch token or the phone token redeemed twice behaves as specified (the launch token once; the phone token many times while on).
   - The old phone token fails after off/on.
   - A LAN request with a loopback `Host`, or the reverse, is refused.

   Tests: Task 5 (auth unit tests) and Task 6 (`old phone token is refused after a restart of phone access`, `lan listener refuses a foreign host`).
4. **A firewall rule that blocks on one profile and allows on another**, or a disabled firewall. The verdict must use the profile Windows applies to the current network, and a block rule wins over an allow rule. Test: Task 3, `firewall verdict uses the current profile and block wins`.
5. **The replaced-router prompt with SSID unknown, two candidate networks, or a scan already running.** Expected:
   - no prompt without an SSID;
   - the most recently seen candidate is used;
   - a running scan fails with the fixed text and writes nothing.

   Test: Task 2, `replaced router: no ssid no prompt; newest candidate; running scan fails without writing`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/netlantern-core/src/store/settings.rs` (new) | Global key/value settings (scope `"global"`). |
| `crates/netlantern-core/src/store/networks.rs` (modify) | SSID label, replaced-router candidate lookup, moving a network to a new gateway MAC. |
| `crates/netlantern-core/src/scan/stages.rs`, `scan.rs`, `error.rs` (modify) | The replaced-router check in identification; passive `replaced_router()`; `decide_replaced_router()`; `CoreError::NetworkDecisionNeeded`. |
| `crates/netlantern-platform/src/platform.rs`, `types.rs` (modify) | `NetworkCategory`, `FirewallProfile`, `FirewallRuleState`, `FirewallStatus`; trait methods. |
| `crates/netlantern-platform/src/sim/{mod,scenario}.rs` (modify) | Scenario fields `category` and `firewall`. |
| `crates/netlantern-platform/src/win/netlist.rs` (new) | Network category via `INetworkListManager`. |
| `crates/netlantern-platform/src/win/firewall.rs` (new) | Firewall rules via `INetFwPolicy2` (read-only). |
| `crates/netlantern-server/src/auth.rs` (modify) | Session kinds, the phone token, revocation. |
| `crates/netlantern-server/src/guard.rs` (modify) | The dynamic phone host and origin. |
| `crates/netlantern-server/src/phone.rs` (new) | `PhoneAccess`: binder, listener lifecycle, watcher, status. |
| `crates/netlantern-server/src/routes.rs`, `error.rs`, `ws.rs`, `state.rs`, `lib.rs` (modify) | Routes, error codes, WebSocket close on revoke, wiring. |
| `crates/netlantern/src/{main,cli}.rs` (modify) | The real or demo binder, restoring phone access at start, `--phone-check`. |
| `ui/src/api/settings.ts` (new) | Phone access, category and replaced-router API hooks. |
| `ui/src/pages/Settings.tsx` (new) | The Settings screen. |
| `ui/src/components/PhoneAccess.tsx` (new) | The phone access card: explanation, QR, firewall and public states. |
| `ui/src/components/FirstRun.tsx` (new), `RouterPrompt.tsx` (new) | The first-run card and the replaced-router prompt. |
| `ui/src/pages/Dashboard.tsx`, `ui/src/App.tsx` (modify) | Wiring. |
| `ui/e2e/demo.spec.ts` | End-to-end tests for responsiveness and the phone flow. |
| `docs/checkpoints/phase-6.md` (new), spec (modify) | Checkpoint and notes. |

---

### Task 1: Global settings and network SSID storage

**Files:**
- Create: `crates/netlantern-core/src/store/settings.rs`
- Modify: `crates/netlantern-core/src/store/mod.rs`, `crates/netlantern-core/src/store/networks.rs`
- Test: unit tests inside both files (the existing in-file test style of `store/rules.rs`)

**Interfaces:**
- Produces:

```rust
impl Store {
    pub async fn global_setting(&self, key: &str) -> Result<Option<String>, StoreError>;
    pub async fn set_global_setting(&self, key: &str, value: &str) -> Result<(), StoreError>;
    pub async fn set_ssid_label(&self, network: NetworkId, ssid: &str) -> Result<(), StoreError>;
    /// The most recently seen network with this SSID and subnet whose gateway MAC differs from `gateway_mac`.
    pub async fn replaced_router_candidate(&self, gateway_mac: MacAddr, subnet: &str, ssid: &str)
        -> Result<Option<(NetworkId, String)>, StoreError>; // (id, ssid label)
    /// Points `network` at a new gateway MAC (the "keep my devices" answer).
    pub async fn move_network_to_gateway(&self, network: NetworkId, gateway_mac: MacAddr) -> Result<(), StoreError>;
}
pub const PHONE_ACCESS_KEY: &str = "phone_access"; // values "on" / "off"
```

- [ ] **Step 1: Write the failing tests.**
  - `global_settings_round_trip_and_missing_is_none`.
  - `global_scope_does_not_collide_with_network_rules`: setting `notifications` globally leaves `rule_settings(net)` unchanged.
  - `ssid_label_is_stored_and_overwritten`.
  - `replaced_router_candidate_matches_ssid_and_subnet_only`: same SSID and subnet with another MAC → Some. The same MAC → None. Another subnet → None. Another SSID → None.
  - `replaced_router_candidate_picks_most_recently_seen`.
  - `move_network_to_gateway_makes_find_network_match_the_new_mac`: after the move, `find_network(new, subnet)` is Some(old id) and `find_network(old_mac, subnet)` is None.
  - `move_network_to_gateway_refuses_when_the_new_mac_already_has_a_network`: it returns an error and changes nothing. This relies on the `UNIQUE(gateway_mac, subnet)` constraint the existing `ON CONFLICT` upsert implies. Verify the constraint exists.
- [ ] **Step 2:** Run `cargo test -p netlantern-core store::` and see the tests fail.
- [ ] **Step 3:** Implement. The scope string is `"global"`; rules use the network id, so the two cannot collide. `ssid_label` already exists in the `networks` table, so no migration is needed.
- [ ] **Step 4:** Run the tests, then the full workspace verification.
- [ ] **Step 5:** Commit: "Store global settings, each network's SSID label, and find or move a network for a replaced router."

---

### Task 2: Replaced-router decision in network identification

**Files:**
- Modify: `crates/netlantern-core/src/error.rs`, `crates/netlantern-core/src/scan.rs`, `crates/netlantern-core/src/scan/stages.rs`
- Test: `crates/netlantern-core/tests/scan_sim.rs`

**Interfaces:**
- Consumes: Task 1's store methods.
- Produces:

```rust
// error.rs
CoreError::NetworkDecisionNeeded { candidate: NetworkId, label: String }
pub const NEW_ROUTER_TEXT: &str = "This looks like a network you know with a new router.";

// scan.rs
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReplacedRouter { pub network_id: NetworkId, pub label: String }
impl ScanManager {
    /// Passive: ARP table only, no probes. None when not connected, the gateway is not in the table, or no decision is pending.
    pub async fn replaced_router(&self) -> Result<Option<ReplacedRouter>, CoreError>;
    /// keep = true: move the candidate to the current gateway MAC; false: create the new network. Returns the network now in use.
    pub async fn decide_replaced_router(&self, keep: bool) -> Result<NetworkId, CoreError>;
}
```

**Decisions:**
- **The check:** `identify_network` reads the SSID with `platform.wifi_current()`, best effort. `Ok(Some(c))` gives `c.ssid`; anything else means no SSID.
  - When `find_network(mac, subnet)` is None and an SSID is known, it calls `replaced_router_candidate`. Some → `Err(Failure(NEW_ROUTER_TEXT))`, and the network is not created.
  - Otherwise it does as today (`network_for`). Then, when an SSID is known, it calls `set_ssid_label`.
- **Errors:** `start(mode)` runs the same passive check as the consent check, through `replaced_router()`, and returns `CoreError::NetworkDecisionNeeded` before a scan row is created. `set_scan_consent` goes through `identify_network`, so it maps that `Failure` text to `NetworkDecisionNeeded` as well, not `Unidentified`.
- **The answer:** `decide_replaced_router` identifies actively (ARP resolve if needed). It reads the candidate again and errors with `Unidentified` if none is pending. Then:
  - `keep` → `move_network_to_gateway(candidate, mac)`;
  - otherwise `network_for(mac, subnet)`.

  It then remembers the result as `KnownNetwork`, the same as `set_scan_consent`.

- [ ] **Step 1: Write the failing tests** in `scan_sim.rs`. Seed the store with a network `(old_mac, "192.168.1.0/24")` whose `ssid_label` is the scenario's SSID ("Lantern Home" in `home.json`; read the real value), then:
  - `replaced_router_is_offered_and_blocks_scans`: `replaced_router()` returns that network's id and label, and `start(Quick)` returns `NetworkDecisionNeeded`.
  - `keeping_moves_the_old_network_and_its_devices_to_the_new_router`: a seeded device on the old network is in the device list after `decide_replaced_router(true)` and a quick scan, under the same id and name.
  - `declining_creates_a_new_network`: the old network keeps its MAC. The scan's devices go to a new network id, and `replaced_router()` is None afterwards.
  - `replaced router: no ssid no prompt; newest candidate; running scan fails without writing`:
    - wifi permission denied → no prompt, and a new network is created;
    - two candidates → the newer one is offered;
    - a candidate inserted after `start` but before identification → the scan ends `failed` with `NEW_ROUTER_TEXT`, and no network row is created for the new MAC.
  - `identification_records_the_ssid_label`.
- [ ] **Step 2:** Run `cargo test -p netlantern-core --test scan_sim replaced` and see the tests fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests, then the full workspace verification.
- [ ] **Step 5:** Commit: "Ask before treating a known network with a new router as new, and record each network's SSID."

---

### Task 3: Network category and firewall status in the platform seam (types and simulator)

**Files:**
- Modify: `crates/netlantern-platform/src/platform.rs`, `crates/netlantern-platform/src/lib.rs`, `crates/netlantern-platform/src/sim/mod.rs`, `crates/netlantern-platform/src/sim/scenario.rs`, `crates/netlantern-platform/src/win/platform.rs` (stub only: `Unavailable { reason: "not implemented yet" }`), `scenarios/cafe.json`
- Test: unit tests in `platform.rs` and the sim module

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkCategory { Public, Private, Domain }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirewallRuleState { Allowed, Blocked, NoRule }

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FirewallProfileStatus { pub profile: NetworkCategory, pub firewall_on: bool, pub rule: FirewallRuleState }

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FirewallStatus { pub profiles: Vec<FirewallProfileStatus> }

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FirewallVerdict { Allowed, Blocked, NotDecided, Off }

impl FirewallStatus {
    /// The verdict for the profile `category` maps to: firewall off → Off; Blocked → Blocked; Allowed → Allowed; NoRule → NotDecided. Missing profile → NotDecided.
    pub fn verdict_for(&self, category: NetworkCategory) -> FirewallVerdict;
}

// Platform trait (no default bodies):
async fn network_category(&self) -> Result<NetworkCategory, PlatformError>;
/// Inbound TCP rules whose program is `exe` (case-insensitive path compare). Block wins over allow within a profile.
async fn firewall_status(&self, exe: &std::path::Path, port: u16) -> Result<FirewallStatus, PlatformError>;
```

**The simulator:**
- Scenario fields:
  - `"category": "public" | "private" | "domain"`, default `"private"`;
  - `"firewall": { "on": bool, "rule": "allowed" | "blocked" | "none" }`, default `{ on: true, rule: "allowed" }`, applied to all three profiles.
- When not connected, `network_category` returns `NotConnected`.
- `scenarios/cafe.json` gets `"category": "public"`.
- Each existing scenario-test helper that builds `Scenario` literals must still compile, because the new fields have defaults.

- [ ] **Step 1: Write the failing tests.**
  - `firewall verdict uses the current profile and block wins`: profiles Private Allowed, Public Blocked and Domain NoRule give `verdict_for(Private) == Allowed`, `verdict_for(Public) == Blocked` and `verdict_for(Domain) == NotDecided`. A profile with `firewall_on: false` and rule Blocked gives `Off`. A missing profile gives `NotDecided`.
  - `category_and_firewall_serialize_snake_case`.
  - `sim_reads_category_and_firewall_from_the_scenario_with_defaults`: `cafe.json` gives Public, and `home.json` gives Private with all profiles Allowed.
  - `sim_category_is_not_connected_when_offline`: use `offline.json`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement. Add the Windows stub so the workspace builds; Task 4 replaces it.
- [ ] **Step 4:** Run the tests, then the full workspace verification.
- [ ] **Step 5:** Commit: "Add network category and firewall status to the platform seam, with simulator support and a Public café scenario."

---

### Task 4: Windows network category and firewall reader

**Files:**
- Create: `crates/netlantern-platform/src/win/netlist.rs`, `crates/netlantern-platform/src/win/firewall.rs`
- Modify: `crates/netlantern-platform/src/win/{mod,platform,adapters}.rs`, `crates/netlantern-platform/Cargo.toml` (windows features), `crates/netlantern/src/{cli,main}.rs` (`--phone-check`)
- Test: pure helpers unit-tested; a live check `#[ignore]` test; `--phone-check` for the author

**Interfaces:**
- Consumes: Task 3 types.
- Produces:
  - Real `network_category()` and `firewall_status()` on the Windows platform.
  - A CLI `--phone-check`, conflicting with the other `*-once` flags. It prints, on Windows only:
    - `Network category: Private`;
    - `Firewall: Private on, rule allowed · Public on, no rule · Domain on, no rule`;
    - `Phone access would be: allowed` (or `blocked`, `waiting for the Windows prompt`, or `firewall off`, using the current category).

**Decisions:**
- **Threading:** COM runs on `spawn_blocking`, with `CoInitializeEx(None, COINIT_MULTITHREADED)` per call and `CoUninitialize` on the way out. A `RPC_E_CHANGED_MODE` from `CoInitializeEx` means COM is already initialized; proceed without uninitializing.
- **Category:**
  - `CoCreateInstance(NetworkListManager)` → `INetworkListManager::GetNetworkConnections()` → for each `INetworkConnection`, `GetAdapterId()` (a GUID).
  - Match the GUID against the active adapter's `AdapterName` GUID. `adapters.rs` already selects the active adapter; expose its GUID through a crate-private helper and choose the same adapter as `active_interface`.
  - On a match, `GetNetwork()?.GetCategory()`: `NLM_NETWORK_CATEGORY_PUBLIC` → Public, `PRIVATE` → Private, `DOMAIN_AUTHENTICATED` → Domain.
  - No match → the most restrictive category among connected networks: Public if any is Public, else Private.
  - No active adapter → `NotConnected`.
- **Firewall:**
  - `CoCreateInstance(NetFwPolicy2)`. For each profile (`NET_FW_PROFILE2_DOMAIN`, `_PRIVATE`, `_PUBLIC`), `get_FirewallEnabled(profile)`.
  - `get_Rules()` → `_NewEnum` → `IEnumVARIANT`, then cast each to `INetFwRule`.
  - Keep rules where all of these hold: `Enabled`, `Direction == NET_FW_RULE_DIR_IN`, `ApplicationName` equal to the exe path ignoring ASCII case, `Protocol` TCP (6) or any (256), and `LocalPorts` either `"*"` or containing the port as a plain number in its comma list. Ranges are not parsed; a rule with a range counts only if the range text is `"*"`.
  - Per profile, a rule applies when its `Profiles` bitmask includes the profile. Any applying Block gives Blocked; otherwise any Allow gives Allowed; otherwise NoRule.
- **Helpers:** pure functions for the matching and folding, so they unit-test without COM:

```rust
pub(crate) struct RuleFacts { pub enabled: bool, pub inbound: bool, pub app: Option<String>, pub protocol: i32, pub local_ports: String, pub profiles: i32, pub allow: bool }
pub(crate) fn fold_rules(rules: &[RuleFacts], exe: &Path, port: u16, profile_bit: i32) -> FirewallRuleState;
pub(crate) fn category_from_nlm(raw: i32) -> NetworkCategory;
```

- **Windows features:** add `Win32_System_Com`, `Win32_System_Ole`, `Win32_System_Variant`, `Win32_NetworkManagement_WindowsFirewall` and `Win32_Networking_NetworkListManager`. Check the exact feature names against windows 0.62 and report any rename.
- **Read-only:** do not call any `put_*` on firewall objects, `Add`, `Remove`, or `SetCategory`.

- [ ] **Step 1: Write the failing tests** for `fold_rules`:
  - `block_wins_over_allow`;
  - `other_programs_ignored_case_insensitive_match`;
  - `outbound_and_disabled_ignored`;
  - `udp_only_rule_ignored`;
  - `local_ports_star_or_listed_port_only`;
  - `profile_bit_filters`.

  For `category_from_nlm`: one assert per category value. Add `#[ignore] live_category_and_firewall_read_without_error`, which calls both on the real platform and asserts `Ok`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement the readers and `--phone-check`.
- [ ] **Step 4:** Run the tests and `cargo test -p netlantern-platform -- --ignored live_category` on this machine; report the output. Run the debug exe with `--phone-check` and put its output in the report. Then the full workspace verification.
- [ ] **Step 5:** Commit: "Read Windows' network category and this exe's firewall rules, read-only, and add --phone-check to print them."

---

### Task 5: Session kinds, the phone token, and revocation

**Files:**
- Modify: `crates/netlantern-server/src/auth.rs`, `crates/netlantern-server/src/routes.rs` (`create_session`), `crates/netlantern-server/src/ws.rs`, `crates/netlantern-server/src/guard.rs` (where the session is looked up)
- Test: unit tests in `auth.rs`; `crates/netlantern-server/tests/api.rs` for the WebSocket close

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKind { Local, Phone }
pub struct AuthLimits { pub token_ttl: Duration, pub max_tokens: usize, pub max_sessions: usize, pub max_phone_sessions: usize } // default max_phone_sessions 32
impl Auth {
    /// A launch token (one-time, TTL) gives a Local session; the current phone token (reusable) gives a Phone session.
    pub fn redeem(&self, token: &str) -> Option<(String, SessionKind)>;
    /// New phone token, replacing any old one. Never logged.
    pub fn rotate_phone_token(&self) -> String;
    /// No phone token: nothing redeems as Phone.
    pub fn clear_phone_token(&self);
    pub fn phone_token(&self) -> Option<String>;
    /// Drops every phone session and bumps the revocation generation.
    pub fn end_phone_sessions(&self);
    pub fn session_kind(&self, id: &str) -> Option<SessionKind>;
    pub fn is_session(&self, id: &str) -> bool; // unchanged meaning: either kind
    /// Fires (value changes) whenever sessions are revoked.
    pub fn revocations(&self) -> tokio::sync::watch::Receiver<u64>;
}
```

**Decisions:**
- **Comparison:** phone-token comparison is constant-time. Add `subtle` (`ConstantTimeEq`), or compare fixed-length byte arrays with a fold; the implementer chooses and says which.
- **Storage:** phone sessions live in their own deque with their own cap.
- **`Debug`:** still prints counts only, plus `phone_token: bool`.
- **WebSocket:** the handler `ws::events` remembers the session id it was opened with. In its send loop, it `select!`s on `revocations().changed()`. When its session is no longer valid, it sends a close frame and ends.
- **Cookie:** `create_session` sets the same cookie for both kinds.

- [ ] **Step 1: Write the failing tests** in `auth.rs`:
  - `phone_token_redeems_many_times_as_phone`;
  - `launch_token_still_redeems_once_as_local`;
  - `rotated_phone_token_refuses_the_old_one`;
  - `cleared_phone_token_redeems_nothing`;
  - `end_phone_sessions_keeps_local_sessions`;
  - `phone_sessions_never_evict_local_sessions`: after 40 phone redeems, a local session made before them is still valid;
  - `revocations_receiver_sees_a_change_on_end_phone_sessions`;
  - `debug_never_contains_tokens_or_session_ids`.

  In `api.rs`, add `revoking phone sessions closes their websockets`:
  1. Redeem a phone token through `POST /api/session`.
  2. Open `/api/events` with that cookie.
  3. Call `auth.end_phone_sessions()`.
  4. The socket gets a close frame within 1 s, and `GET /api/network` with that cookie is 401.
  5. A local session's socket stays open.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests, then the full workspace verification.
- [ ] **Step 5:** Commit: "Give sessions a kind, add a reusable phone token, and close a phone's WebSocket when its session is revoked."

---

### Task 6: Phone access manager, LAN listener and API

**Files:**
- Create: `crates/netlantern-server/src/phone.rs`
- Modify: `crates/netlantern-server/src/{lib,state,guard,routes,error,serve}.rs`, `crates/netlantern/src/main.rs`
- Test: `crates/netlantern-server/tests/phone.rs` (new), unit tests in `phone.rs`

**Interfaces:**
- Consumes: Tasks 1 and 3 to 5 (`PHONE_ACCESS_KEY`, `network_category`, `firewall_status`, and `Auth`'s phone methods).
- Produces:

```rust
#[async_trait]
pub trait LanBinder: Send + Sync {
    /// Bind a listener for phones. Returns the listener and the address to put in the URL.
    async fn bind(&self, lan_ip: Ipv4Addr, preferred_port: u16) -> std::io::Result<(TcpListener, SocketAddr)>;
}
pub struct RealLanBinder;      // binds (lan_ip, preferred_port), else the first free port in DEFAULT_PORTS
pub struct LoopbackLanBinder;  // demo/tests: binds 127.0.0.1:0, URL host 127.0.0.1

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneState { Off, On, Paused }

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PhoneStatus {
    pub state: PhoneState,
    /// Why it is paused or could not start: "public_network" | "not_connected" | "bind_failed"; None when Off/On.
    pub reason: Option<String>,
    /// Full URL with token, only when On. Shown only to signed-in users.
    pub url: Option<String>,
    pub firewall: Option<FirewallVerdict>,  // None when Off or when it could not be read
    pub category: Option<NetworkCategory>,
    pub demo: bool,                          // true with LoopbackLanBinder: "this address works on this laptop only"
}

pub struct PhoneAccess { /* … */ }
impl PhoneAccess {
    pub fn new(platform: Arc<dyn Platform>, store: Store, auth: Arc<Auth>, bus: Arc<EventBus>,
               binder: Arc<dyn LanBinder>, exe: PathBuf, main_port: u16, demo: bool) -> Arc<Self>;
    /// The router the phone listener serves. AppState holds the PhoneAccess, and the router needs the
    /// AppState, so the host calls this once after building both; enable() before it is bind_failed.
    pub fn attach_router(&self, router: axum::Router);
    /// Restore the saved setting at start (On if saved "on" and allowed).
    pub async fn restore(self: &Arc<Self>);
    pub async fn enable(self: &Arc<Self>) -> Result<PhoneStatus, ApiError>; // 409 public_network, 503 phone_access_unavailable
    pub async fn disable(&self) -> PhoneStatus;
    pub async fn status(&self) -> PhoneStatus;
    /// Host and Origin values the guard must accept now (empty when not On).
    pub fn allowed_host(&self) -> Option<String>;   // e.g. "192.168.1.23:47800"
    pub fn allowed_origin(&self) -> Option<String>; // e.g. "http://192.168.1.23:47800"
    pub async fn stop(&self); // on quit: close the listener within the grace
}
```

**Decisions:**
- **Enable:**
  1. Read the interface; `NotConnected` → 503 `phone_access_unavailable`, with the message "Phone access needs a network connection."
  2. Read the category; Public → 409 `public_network`, with the hint "Open Settings → Network & internet → Wi-Fi → your network, and set Network profile type to Private." The setting is still saved "on", so it resumes on a Private network.
  3. Bind through the binder; failure → 503 `phone_access_unavailable`, reason `bind_failed`.
  4. `rotate_phone_token()`.
  5. Spawn `axum::serve(listener, router(app_state.clone()))` with a graceful-shutdown signal. Keep its `JoinHandle`.
  6. Save `"on"`.
  7. Return the status, with the firewall verdict read now (it is usually `not_decided` right after the first bind).
- **Disable:**
  1. Save `"off"`.
  2. `clear_phone_token()` and `end_phone_sessions()`.
  3. Signal shutdown, wait up to 2 s (the same grace as quit, spec §6.12), then abort the task.
- **Watcher:** while the setting is "on", check every 5 s. This adds one §6.12 row, "Phone access network check: every 5 s".
  - Interface IP changed → rebind on the new IP and keep the token. Phone sessions end, because their cookies are bound to the old host.
  - The category became Public or the connection dropped → Paused with a reason: close the listener and end phone sessions.
  - Paused and now Private, Domain or connected → resume, with a new token.
- **Guard:** `GuardState` gains `phone: Arc<PhoneAccess>`, or a smaller `Arc<dyn PhoneHosts>` view to avoid a dependency cycle (implementer's choice). The Host check accepts `allowed_host()` and the Origin check accepts `allowed_origin()`, but only while On.
- **URL:** `http://{lan_ip}:{port}/?t={token}`. This is the format the UI's `ensureSession` already handles.
- **Routes:**
  - `GET /api/phone-access` → `PhoneStatus`.
  - `POST /api/phone-access {enabled: bool}` → `PhoneStatus` or an error.
  - Events: publish `phone.changed` on every state change, so the Settings page refreshes. Add the event name to the events list in spec §8.
- **Wiring in `main.rs`:**
  - The real run uses `RealLanBinder` with `exe = std::env::current_exe()`, `main_port = port`, `demo = false`.
  - `--demo` uses `LoopbackLanBinder` with `demo = true`.
  - Call `restore()` after the server starts, and `stop()` on quit, before `monitor.stop`.

- [ ] **Step 1: Write the failing tests** in `tests/phone.rs`, using the sim platform, `LoopbackLanBinder` and an in-memory store:
  - `enable_serves_the_dashboard_on_the_phone_address_with_the_token`: GET `/` on the phone address is 200. POST `/api/session` with the phone token gives a cookie, and `GET /api/network` with it is 200.
  - `refused_without_token`: `GET /api/network` on the phone address without a cookie is 401, and a wrong token is 401.
  - `lan listener refuses a foreign host`: a request to the phone listener with `Host: evil.example` is 403, and one with the phone host but a foreign `Origin` is 403.
  - `old phone token is refused after a restart of phone access`: enable, disable, then enable again; the first token is 401.
  - `disable ends phone sessions and stops within the grace`:
    - with an open phone WebSocket and an idle keep-alive client, `disable()` returns within 2.5 s;
    - the socket is closed and the old session is 401 on the main listener too;
    - a new connection to the phone port is refused.
  - `public network refuses enable with 409 public_network`: use `cafe.json`, and check the error body's code and hint.
  - `rebinds when the address changes and pauses on a public network`: drive the sim. Use the scenario's events if they can change the interface IP or category; otherwise add a test-only setter on the sim (`SimNetwork::set_category`, `set_interface_ip`) and say so. Expected:
    - the state follows On → On on the new address, with the old host refused by the guard;
    - then → Paused with "public_network";
    - then back to Private → On with a new token.

    Use tokio's paused clock to step the 5 s watcher.
  - `restore_turns_phone_access_back_on_from_the_saved_setting`.
  - `status_reports_firewall_verdict_and_demo_flag`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests and the full workspace verification. Then run `--demo` with a unique pipe name, enable via curl with the launch cookie, and open the phone URL: report status codes, not tokens.
- [ ] **Step 5:** Commit: "Serve the dashboard to phones on a LAN listener that follows the network, pauses on Public networks and ends phone sessions when turned off."

---

### Task 7: Network API: category, SSID and the replaced-router answer

**Files:**
- Modify: `crates/netlantern-server/src/routes.rs`, `crates/netlantern-server/src/error.rs`
- Test: `crates/netlantern-server/tests/api.rs`

**Interfaces:**
- Consumes: Task 2 (`replaced_router`, `decide_replaced_router`, `NetworkDecisionNeeded`) and Task 3 (`network_category`).
- Produces:
  - `GET /api/network` adds three fields:
    - `category: "public" | "private" | "domain" | null`, null when it can't be read, never failing the route;
    - `ssid: string | null`, from `wifi_current`, best effort;
    - `replaced_router: { network_id, label } | null`.
  - `POST /api/network/replaced-router {keep: bool}` → `{ network_id }`. It returns 404 `no_decision_pending` when nothing is pending.
  - `POST /api/scans` and `PUT /api/network/consent` map `NetworkDecisionNeeded` to 409 `network_decision_needed`, with the message `NEW_ROUTER_TEXT` and the hint "Answer the question on the Dashboard: keep this network's devices and names, or start a new network."

- [ ] **Step 1: Write the failing tests:**
  - `network_reports_category_ssid_and_no_pending_router` (home: private, "Lantern Home", null);
  - `network_reports_public_on_the_cafe_network`;
  - `pending_replaced_router_blocks_scans_with_409`;
  - `answering_keep_clears_the_prompt_and_scans_start`;
  - `answering_when_nothing_is_pending_is_404_no_decision_pending`.

  For a pending router, seed the store as in Task 2.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests, then the full workspace verification.
- [ ] **Step 5:** Commit: "Report the network's Windows category, SSID and any pending new-router question, and accept the answer over the API."

---

### Task 8: Settings screen with the phone access card

**Files:**
- Create: `ui/src/api/settings.ts`, `ui/src/pages/Settings.tsx`, `ui/src/pages/Settings.test.tsx`, `ui/src/components/PhoneAccess.tsx`, `ui/src/components/PhoneAccess.test.tsx`
- Modify: `ui/src/App.tsx` (the `settings` route, lazy like Map; remove the last `Placeholder` use and delete `pages/Placeholder.tsx` along with its test reference in `Shell.test.tsx`), `ui/src/api/queries.ts` (`Network` gains `category`, `ssid`, `replaced_router`), `ui/src/api/events.ts` (`phone.changed` invalidates `["phone"]`), `ui/package.json` (`qrcode` plus `@types/qrcode` if needed)

**Interfaces:**
- Consumes: Task 6 and 7 APIs.
- Produces:

```ts
export type PhoneState = "off" | "on" | "paused";
export type FirewallVerdict = "allowed" | "blocked" | "not_decided" | "off";
export interface PhoneStatus { state: PhoneState; reason: string | null; url: string | null; firewall: FirewallVerdict | null; category: "public" | "private" | "domain" | null; demo: boolean }
export function usePhoneAccess(): UseQueryResult<PhoneStatus>;
export function useSetPhoneAccess(): (enabled: boolean) => Promise<PhoneStatus>; // throws ApiRequestError
export function useAnswerReplacedRouter(): (keep: boolean) => Promise<void>;
```

**Settings page:** an `<h1>` "Settings", then four cards, each a `<section aria-label=…>`, in this order.

1. **Phone access** (`PhoneAccess` component). The copy is exact.
   - **Off:**
     - Text: "Open this dashboard on your phone over Wi-Fi. Your phone and this laptop must be on the same network."
     - Button: "Allow phone access".
     - Clicking shows the explanation before anything is sent: "Windows may ask whether Net Lantern can use the network. Choose **Allow** for **Private networks**. This needs an administrator's approval once. Anyone with the QR code can open the dashboard until you turn phone access off."
     - Buttons: "Continue" (POST enabled: true) and "Cancel".
   - **On:**
     - A QR code of `url`, rendered as an SVG with `qrcode.toString(url, { type: "svg", margin: 1 })` and inserted via a sanitized `<img src="data:image/svg+xml;…">`. Wrap it in a white quiet-zone box, so phone cameras can read it on the dark theme. Add a `--qr-bg: #FFFFFF` token to `tokens.css` and use it, so no raw colour appears in components.
     - The URL as text, with a "Copy link" button.
     - Text: "Scan this with your phone's camera."
     - When `demo`: "Demo mode: this link works on this laptop only."
     - Firewall line from `firewall`:
       - `allowed` or `off`: "Windows Firewall allows phones to connect."
       - `not_decided`: "Waiting for your answer to the Windows Firewall prompt." The card polls `GET /api/phone-access` every 2 s while `not_decided`, for at most 60 s.
       - `blocked`: "Windows Firewall is blocking phones. Open **Windows Security → Firewall & network protection → Allow an app through firewall**, find **netlantern.exe**, and tick **Private**. Then turn phone access off and on." Use the `role="alert"` style the app uses for errors.
     - Button: "Turn off phone access".
   - **Paused:**
     - `public_network`: "Phone access is paused because Windows marks this network as Public. On a network you trust, open **Settings → Network & internet → Wi-Fi → your network** and set it to **Private**."
     - `not_connected`: "Phone access is paused until this laptop is connected to a network."
     - Button: "Turn off phone access".
   - **Enable failed with `public_network`:** the same Public text, with the setting still saved, shown as Paused.
2. **This network:**
   - SSID (or "Unknown name") and the interface IP.
   - "Windows marks it **Public** / **Private** / **Domain**", or "Windows' network type couldn't be read."
   - Deep-scan permission: "Deep scans allowed on this network" with "Withdraw permission" (`useSetScanConsent(false)`), or "Deep scans not allowed yet; you'll be asked before the first one."
3. **Alerts and notifications:** "Alert rules and desktop notifications are on the Alerts screen.", with a link to `/alerts`.
4. **Data kept:** a read-only list:
   - raw samples 24 hours;
   - 5-minute averages 30 days;
   - online/offline history 90 days;
   - logs 7 days.

   Values come from spec §6.12; put them in one `const`.

- [ ] **Step 1: Write the failing tests** with `mockFetch`:
  - `off_shows_the_explanation_before_enabling`: clicking "Allow phone access" sends nothing yet. "Continue" POSTs `{enabled:true}`.
  - `on_shows_qr_url_and_firewall_line_for_each_verdict` (4 cases).
  - `not_decided_polls_until_a_verdict`: use fake timers.
  - `demo_note_shown_only_in_demo`.
  - `paused_public_and_not_connected_texts`.
  - `enable_409_public_network_shows_the_public_text`.
  - `turn_off_posts_false`.
  - `network_card_shows_category_and_withdraws_consent`.
  - `settings_route_has_no_placeholder`: no "Arrives in phase" text anywhere in `AppRoutes`.
  - The QR test asserts an `img` with alt "QR code for the phone link".
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests, then `npm run lint`, `npx tsc -b`, `npm test` and `npm run build`. The Settings page and `qrcode` must be in a lazy chunk; record its size.
- [ ] **Step 5:** Commit: "Add the Settings screen with phone access, its QR code and firewall and Public-network explanations, this network's details, and data retention."

---

### Task 9: First-run card and replaced-router prompt

**Files:**
- Create: `ui/src/components/FirstRun.tsx`, `ui/src/components/RouterPrompt.tsx`, with tests
- Modify: `ui/src/pages/Dashboard.tsx` (and its test)

**Interfaces:**
- Consumes: `Network.category`, `ssid`, `replaced_router`; `useAnswerReplacedRouter`; existing scan-start and consent code in `Dashboard.tsx`.

**Decisions:**
- **First-run card:** shown on the Dashboard when `network.network_id === null`, or when there is no latest scan for this network (`fetchLatestScan()` returned null) and no scan is running. Content:
  - "Network detected" heading.
  - SSID (or the IP and subnet) and "Windows marks this network **Public**/**Private**".
  - When Public: "Some features, like phone access, stay off on Public networks."
  - "Run your first scan to see the devices on this network.", with a "Run quick scan" button using the existing start path.
  - A muted note: "A deep scan asks for your permission first."

  It disappears as soon as a scan starts. The existing scan card stays as is, with no duplicated buttons visible at the same time: hide the scan card's buttons while the first-run card shows.
- **Router prompt (`RouterPrompt`):**
  - Shown at the top of the Dashboard when `network.replaced_router` is not null, or when a scan start or consent call fails with `network_decision_needed`, or a scan fails with `NEW_ROUTER_TEXT`. This follows the `NO_CONSENT_TEXT` pattern already in `Dashboard.tsx`.
  - Text: "This looks like **{label}** with a new router. Keep your devices and names?"
  - Buttons: "Keep my devices" (keep: true) and "It's a different network" (keep: false).
  - After answering, invalidate `["network"]`. If the prompt came from a blocked scan start, restart that scan in the same mode.
  - Focus handling is the same as `ConsentPrompt`. Read it and reuse its pattern.

- [ ] **Step 1: Write the failing tests:**
  - `first_run_shows_for_a_network_with_no_scans_and_starts_a_quick_scan`;
  - `first_run_mentions_public_networks_only_when_public`;
  - `first_run_hides_once_a_scan_runs`;
  - `router_prompt_from_network_field_keep_and_decline_post_the_answer`;
  - `blocked_scan_start_shows_the_prompt_and_retries_the_same_mode_after_keep`;
  - `scan_failed_with_new_router_text_shows_the_prompt_once`.
- [ ] **Step 2:** Run the tests and see them fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm run lint`, `npx tsc -b` and `npm test`.
- [ ] **Step 5:** Commit: "Show a first-run card on a new network and ask whether a known network with a new router should keep its devices."

---

### Task 10: Responsive polish across every screen

**Files:**
- Modify: whichever page and component files the checks below find wanting. The known one is `ui/src/pages/Devices.tsx`, whose table scrolls sideways at about 800 px.
- Test: `ui/e2e/demo.spec.ts` (a new `describe` at the end)

**Decisions:**
- **Widths:** 375×812 (phone) and 800×900 (small laptop or tablet).
- **What counts as passing on every route (`/`, `/devices`, `/map`, `/diagnose`, `/alerts`, `/settings`):**
  - `document.documentElement.scrollWidth <= innerWidth`;
  - every visible `button`, `a` and `input[type=checkbox|search|text]` in `main` and the nav has a bounding box of at least 44×44 px. Inline text links inside a paragraph are exempt; mark them with `data-inline-link` and skip them.
- **Devices table:** below 900 px, hide the MAC and Latency columns as well as IP. The IP still shows under the name, as it does on phones today.
- **Fixes** stay within layout and Tailwind classes; no copy changes.

- [ ] **Step 1: Write the failing e2e tests:**
  - `every screen fits a phone`;
  - `every screen fits a small laptop`;
  - `touch targets are at least 44px`.

  Loop over the routes, after a quick scan so lists have content. Reset the viewport in `finally`.
- [ ] **Step 2:** Build the UI and the exe (temp target), then run `npm run e2e` with `NETLANTERN_EXE`. Record which routes fail and why.
- [ ] **Step 3:** Fix each failure.
- [ ] **Step 4:** Re-run e2e until green, plus `npm run lint`, `npx tsc -b` and `npm test`.
- [ ] **Step 5:** Commit: "Make every screen fit phone and small-laptop widths with 44px touch targets, and keep the Devices table from scrolling sideways."

---

### Task 11: Phone flow end to end, checkpoint and spec notes

**Files:**
- Create: `docs/checkpoints/phase-6.md`
- Modify: `ui/e2e/demo.spec.ts`
- Modify: `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§6.11, §6.12, §8, §9, §12 notes)

- [ ] **Step 1: Write the e2e test** `phone access works from a second browser and stops when turned off`. Add it at the end of `demo.spec.ts`'s serial `describe`, reusing its signed-in `page`; the launch token is single-use, so a separate spec file could not sign in.
  1. On the signed-in laptop page (a quick scan has already run in the earlier tests), go to Settings, click "Allow phone access", then "Continue".
  3. Read the link text, then open it in a **new browser context**, which acts as the phone with no cookies. Expect the Dashboard heading.
  4. In that phone context, request `/api/network` without the token, in a third fresh context: 401.
  5. Back on the laptop page, click "Turn off phone access".
  6. In the phone context, reload. Expect the "needs launch" text from `NeedsLaunch.tsx`, or a 401 on `/api/network`.
- [ ] **Step 2:** Run all e2e tests and make sure they pass.
- [ ] **Step 3: Write the checkpoint** `docs/checkpoints/phase-6.md`. Use the same shape as `phase-5.md`, with the build block. Items:
  1. `--phone-check` output on the home Wi-Fi, before turning phone access on.
  2. Turn on phone access. Note the Windows Firewall prompt (or none), what you chose, and the firewall line Settings shows afterwards.
  3. Scan the QR with the phone. The dashboard opens, and the phone can browse Devices, Map and Alerts.
  4. The phone swipe check left over from Phase 5, item 6: on the phone, a one-finger swipe over the map scrolls the page.
  5. Open the URL without `?t=…` on the phone in a private tab. It must show the sign-in-needed page.
  6. Turn phone access off. The phone's open page stops updating and a reload shows the sign-in-needed page.
  7. A dismissed prompt:
     1. **Only if you are willing:** open Windows Security → Allow an app through firewall, and untick Private for `netlantern.exe`.
     2. Turn phone access off and on, and confirm Settings explains the block.
     3. Tick it again.

     Or skip this item and write "skipped".
  8. A Public network: if you can, on a hotspot or café network Windows marks Public, phone access says it is paused and why. Or skip.
  9. Restart Net Lantern with phone access on. It comes back on, and the old QR no longer works (a new one shows).
  10. Responsive check on the phone: every screen fits with no sideways scroll.
  11. The learning check (spec §13, phase 6): explain why the phone token is reusable but the launch token is one-time; what `HttpOnly` and `SameSite=Strict` protect against; what Windows does when you dismiss the firewall prompt and why Net Lantern can only read, never fix, the rule.

  End with a "What to look at" note: the phone listener's port, and the extra 5 s network check.
- [ ] **Step 4: Update the spec.**
  - §6.11: append a phase-6 note on the reusable phone token, pausing on Public networks, the demo loopback binder, read-only firewall and category reads, and phone sessions closing their WebSockets.
  - §6.12: add Server rows, each marked *(changed in phase 6)*:
    - phone listener port: the same port as the main listener, else the first free port in 47800–47809;
    - phone access network check: every 5 s;
    - phone sessions cap: 32;
    - phone listener stop grace: 2 s.
  - §8:
    - `GET /api/phone-access`;
    - the `POST /api/phone-access` shape;
    - `POST /api/network/replaced-router`;
    - the new `GET /api/network` fields;
    - the `phone.changed` event;
    - error codes `public_network`, `network_decision_needed`, `phone_access_unavailable` and `no_decision_pending`;
    - the note that `firewall_blocked` is a status value, not an error.
  - §9: a note that Settings in Phase 6 holds phone access, this network, the alerts link and retention. Update check, copy debug info, data reset and start at login move to Phase 7, and scan schedule is dropped from v1, with the reason.
- [ ] **Step 5: Final verification.**
  - Rust, from the repo root: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
  - UI, from `ui/`: `npm run lint`, `npx tsc -b`, `npm test`, `npm run build` and `npm run e2e`.

  All must be clean.
- [ ] **Step 6:** Commit: "Add the phone access end-to-end test, the Phase 6 checkpoint, and the phone access, settings and API notes in the spec."
