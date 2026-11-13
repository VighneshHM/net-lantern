# Net Lantern Phase 4: Deep Scan Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user who owns the network can run a deep scan without admin rights. It asks permission once per network, checks 100 common TCP ports on every device, reads what the open ports say (HTTP page titles, service greetings), and shows an OS guess labelled as a guess. The scan can be stopped and it stops by itself when the network changes. Every scan, quick or deep, also asks each device directly over mDNS for its name, which helps with phones that announce nothing. Three items parked from Phase 3 are fixed.

**Architecture:**
- **`netlantern-platform`:** three new `NetIo` methods, each with a default body so existing test fakes keep compiling:
  - `mdns_query`: a unicast mDNS question sent to one device (its reverse name and its service list);
  - `tcp_banner`: read the greeting a service sends on connect;
  - `lan_http_get`: HTTP(S) GET limited to LAN addresses that accepts self-signed certificates.

  The real implementations live in `net/`. The simulator gains per-device banners, unicast-only mDNS and a reverse name.
- **`netlantern-core`:**
  - Pure parts: `identify/ports.rs` (the deep port list, service names, banner and title cleaning) and `identify/os.rs` (the OS guess).
  - Schema v3 adds `devices.deep_scanned_at`. The store records banners and services per port, replaces the scanned ports' rows after a deep scan, and reads and writes `networks.scan_consent`.
  - `ScanManager` runs `ScanMode::Deep` as the quick-scan stages with two changes: the port stage uses the 100-port list, and a banner stage is added. It checks consent before a deep scan, can be cancelled, and cancels itself when the interface changes.
  - Discovery adds the unicast mDNS query to both modes.
- **Server:** `GET /api/network` gains `network_id` and `scan_consent`. New routes: `PUT /api/network/consent` and `POST /api/scans/{id}/cancel`. New error codes: `consent_needed`, `network_unidentified`, `scan_not_running`.
- **Exe:** `--scan-once --deep`.
- **UI:**
  - The Dashboard enables Deep scan with a consent prompt and gets a Stop button.
  - Device details show ports with services and banners, the OS guess, and when the device was last deep scanned.

**Tech Stack:** Phase 3's stack. New dev-dependencies only, in `netlantern-platform`: `rcgen` 0.13 and `tokio-rustls` 0.26 (a self-signed HTTPS server for one test).

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`. Relevant sections:
- the Phase 4 row of §12;
- §6.1 (the consent question), §6.3 (deep scan), §6.4 (identification), §6.12 (the Deep scan row);
- §7 (`networks.scan_consent`, `device_ports`), §8 (routes, errors), §9 and journey C in §9.1;
- §10 (network change mid-scan, degraded states), the Phase 4 learning goals in §13, §15 (the port list is the builder's choice), §16 (the OS-ARP risk).

Earlier plans: `docs/superpowers/plans/2026-11-11-phase-3-monitor-alerts.md` and the three before it.

**Branch:** `phase-4-deep-scan`, cut from `main` at 587a9b2 (Phases 0–3 merged).

## Global Constraints

- **Windows 11, no admin rights for anything.** CI runs on `windows-latest`. Never parse the text output of OS tools. No raw sockets, no Npcap, no SYN scan: the deep scan is a TCP **connect** scan.
- **IPv4 only.**
- **Only the LAN.** A deep scan contacts nothing outside the LAN.
  - `lan_http_get` refuses any URL whose host is not an RFC 1918 or loopback IPv4 literal, and follows redirects only to the same host and port.
  - No new traffic leaves the LAN, so the README list of outside endpoints is unchanged.
- **Consent.** A deep scan never starts on a network whose `scan_consent` is 0. A quick scan never asks (spec §6.1).
- **Deep scan defaults.** Recorded in spec §6.12 in Task 11:

  | Setting | Value |
  |---|---|
  | Port list | the 100 ports in Task 4 (`DEEP_PORTS`), a superset of the quick-scan five |
  | TCP connect timeout / total concurrency | 800 ms / 64 (as the quick scan) |
  | Connects in flight per device | 8 |
  | Banner and title read: timeout / concurrency | 2 s / 16 |
  | LAN HTTP(S) fetch: total timeout / body kept | 3 s / 16 KB |
  | Unicast mDNS query per device (both modes) | 1.5 s, concurrency 32 |
  | Network-change check during any scan | every 2 s |
  | Monitor stop grace (Phase 3 parked item) | 2 s |

- **Text from the network** (banners, titles, `Server` headers, mDNS names): control characters are removed, it is cut to 100 characters, and the UI renders it as text only. Never log tokens or session ids.
- **API:**
  - `API_VERSION` stays **1**. New routes, fields, enum values and events are additions.
  - Every error body is `{"code","message","hint"}`.
  - Timestamps are milliseconds since the Unix epoch.
  - JSON enums are `snake_case`.
- **UI:** status is always shown as text, touch targets are ≥ 44 px, it works at phone width with no horizontal page scroll, and colours come only from the tokens.
- **Never change Windows settings for the author.**
- **Commit messages are one plain sentence.** No prefix, no body, no `Co-Authored-By` and no Claude or Anthropic attribution.
- **Building while the author's own `netlantern.exe` runs:** it locks `target\debug`. Build with `CARGO_TARGET_DIR=C:/Users/vighn/AppData/Local/Temp/nl-target`. Never kill that process.

## Review Focus

1. **A small device that falls over under many connects** (cheap cameras, smart plugs). Expected: never more than 8 connects in flight to one address, whatever the total concurrency. Tested in Task 6 (`per_device_connects_stay_at_eight`).
2. **Deep scan clicked on a network never scanned, or never consented.** Expected:
   - the UI asks the §6.1 question first;
   - the server answers `403 consent_needed` and creates no scan row;
   - answering yes identifies the network (creating it if new) and then the scan runs.

   Tested in Task 6, Task 8 and Task 10.
3. **Hostile or messy banner text:** a `<script>` page title, control characters, non-UTF-8 bytes, a 5 KB greeting. Expected: cleaned, cut to 100 characters, shown as literal text. Tested in Task 4 (cleaning) and Task 10 (rendering).
4. **A device that answers nothing during a deep scan** (a phone asleep). Expected: the ports found before are kept, and `deep_scanned_at` is not set for that device. Tested in Task 5.
5. **The user stops the scan, or the network changes, mid deep scan.** Expected:
   - the scan ends `cancelled`; a network change says why;
   - devices recorded so far are kept;
   - a new scan can start straight away;
   - monitoring still raises "new device" alerts for devices that scan recorded.

   Tested in Task 7 and Task 1.

## Not in this phase

- **The raw-ARP / Npcap helper.** The Phase 1 coverage check could not be measured on the author's home network (its devices change too often), so the helper does not return here. Packet capture, DHCP listening and optional nmap are discussed after Phase 4 (agreed 2026-11-13).
- **A "best guess: phone" rule for private MACs, and "Private address" in the `--scan-once` table.** Decided after seeing this phase's results.
- **UDP scanning**, and scanning ports outside the 100-port list.
- **Scheduled deep scans, and revoking consent in the UI.** Phase 6 (Settings). The API already accepts `granted: false`.
- **The network map.** Phase 5.

---

### Task 1: Real-network records and two Phase 3 parked fixes

**Files:**
- Modify:
  - `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§2.1 note, §6.3 note, §16 Tauri outcome)
  - `crates/netlantern-core/src/store/devices.rs` (new query)
  - `crates/netlantern-core/src/monitor/listener.rs` (reconcile new devices)
  - `crates/netlantern-core/src/monitor/runner.rs` (`Inner::run`, stop grace)
  - `crates/netlantern-core/src/monitor/config.rs` (`stop_grace`)
- Test:
  - `crates/netlantern-core/tests/store.rs`
  - `crates/netlantern-core/tests/monitor_sim.rs`

**Interfaces:**
- Produces:
  - `Store::devices_first_seen_in_scan(&self, network: NetworkId, scan: ScanId) -> Result<Vec<Device>, StoreError>`. Returns the network's devices whose `last_scan_id = scan` and whose `first_seen >= ` that scan's `started_at`, sorted by id.
  - `MonitorConfig.stop_grace: Duration`, default 2 s.

- [ ] **Step 1: Record the real-network results in the spec**

  These are the author's results from 2026-11-13. Keep the wording plain.
  - **§2.1**, a new note under the Phase 2 note: "Phase 1, author's home Wi-Fi (2026-11-13): `--scan-once` found 4 devices in 7.8 s (≤ 20 s: met). Both non-private MACs showed a vendor (met). 2 of 4 devices got a type (50%, target ≥ 70%: not met); the two unknown devices are phones with private MACs that announce nothing. Coverage against the router's client list could not be measured: the devices on the author's home network change too often to compare."
  - **§6.3**, in the italic note, replace "OS-ARP check against the router's client list still to do." with: "2026-11-13: the check could not be measured on the author's home network (see 2.1). The Npcap helper does not return in phase 4; packet capture is discussed again after phase 4."
  - **§16**, a paragraph after the toast outcome: "Tauri row, phase 3 spike outcome (2026-11-13, author's machine): the spike window loaded the dashboard signed in; a scan and the demo worked in it, and it closed cleanly. Main process working set 30.9 MB (the WebView2 helper processes are separate and were not measured). Surprise: the spike builds `AppState::new` and does not start the monitor; phase 7 must start it."

- [ ] **Step 2: Write the failing store test**

```rust
#[tokio::test]
async fn devices_first_seen_in_a_scan_are_only_the_ones_it_found_first() {
    // Scan 1 records A. Scan 2 (started later) records A again and B for the first time.
    let (store, net) = store_with_network().await;
    let s1 = store.insert_scan(ScanMode::Quick, 1_000).await.unwrap();
    store.set_scan_network(s1, net).await.unwrap();
    let a = store.record_sighting(net, s1, sighting("10.0.0.2", "02:00:00:00:00:02"), 1_100).await.unwrap();
    let s2 = store.insert_scan(ScanMode::Quick, 2_000).await.unwrap();
    store.set_scan_network(s2, net).await.unwrap();
    store.record_sighting(net, s2, sighting("10.0.0.2", "02:00:00:00:00:02"), 2_100).await.unwrap();
    let b = store.record_sighting(net, s2, sighting("10.0.0.3", "50:C7:BF:00:00:03"), 2_200).await.unwrap();

    let ids: Vec<_> = store.devices_first_seen_in_scan(net, s2).await.unwrap().iter().map(|d| d.id).collect();
    assert_eq!(ids, vec![b.id]);
    let ids: Vec<_> = store.devices_first_seen_in_scan(net, s1).await.unwrap().iter().map(|d| d.id).collect();
    assert!(ids.is_empty(), "A's last scan is now scan 2: {ids:?} (a = {:?})", a.id);
}
```

  The names `store_with_network` and `sighting` stand for the file's existing `fx()` fixture and `sighting(ip, mac)` builder (finished with `.build()` or however the builder ends). Use those; don't add new helpers.

- [ ] **Step 3: Run it to make sure it fails**

  Run: `cargo test -p netlantern-core --test store devices_first_seen`
  Expected: FAIL, `no method named devices_first_seen_in_scan`.

- [ ] **Step 4: Implement `devices_first_seen_in_scan`** in `store/devices.rs`

  One SQL query joins `scans` for `started_at`. Each row is read with the existing `load`.

- [ ] **Step 5: Write the failing monitor tests** in `monitor_sim.rs`

  Each test is built the same way:
  - Run the first scan of `scenarios/new-device.json` and start the monitor on an `EventBus::new(8)`. Advance the clock until the session opens.
  - Advance past the device's join time. Run a second scan on a **separate** `ScanManager` that has its own bus but shares the store, so the monitor's bus never carries that scan's `scan.device` events.

  The tests:
  - `new_device_alert_survives_a_lagged_listener`: publish 20 `ScanProgress` events for `ScanId(999)` on the monitor's bus without awaiting in between, then advance 1 s. Assert exactly one open alert with key `new_device:<id of the joined device>`.
  - `new_device_alert_follows_scan_done`: publish only `ScanDone { scan_id: <second scan>, state: Done, .. }` on the monitor's bus, then advance 1 s. Assert one such alert.
  - `scan_device_then_scan_done_opens_one_alert`: publish the joined device's `ScanDevice`, then the `ScanDone`. Assert exactly one open alert for it, and exactly one `alert.changed` event with `state: open` for that key.
  - `stop_returns_within_the_grace_while_a_tick_hangs`: use a `Platform` wrapper whose `ping` sleeps 60 s (paused clock). Advance into the first tick, call `monitor.stop(EndReason::Quit)`, and assert it returns before 2.5 s of virtual time. Assert the session row ended `quit`.

- [ ] **Step 6: Run them to make sure they fail**

  Run: `cargo test -p netlantern-core --test monitor_sim new_device_alert stop_returns scan_device_then`
  Expected:
  - the lagged and scan-done tests fail with no alert;
  - the stop test times out at 60 s of virtual time;
  - `scan_device_then_scan_done_opens_one_alert` already passes. Keep it: it guards the next step.

- [ ] **Step 7: Implement reconciliation in `listener.rs`**

  `Inner::reconcile_new_devices(&self, scan: Option<ScanId>)`:
  - Take the state lock and the session's network. Do nothing without a session.
  - Use `scan`, or else the newest scan of that network (`latest_scan_for`). Read `devices_first_seen_in_scan`.
  - For each device that is still `is_new` and whose `first_seen` is after `first_scan_finished_at`, build `new_device_reading`. Evaluate them all in one `engine.evaluate` call and `apply` the decisions.

  The engine already opens a key only once. Call this method:
  - on `ScanDone` whose state is `Done` or `Cancelled`, with `Some(scan_id)`;
  - on `RecvError::Lagged`, with `None`.

- [ ] **Step 8: Implement the stop grace in `runner.rs`**

  In `Inner::run`, pin the tick future and `select!` it against `stopped.changed()`. When the stop wins, give the tick up to `cfg.stop_grace` to finish (`tokio::time::timeout`), then break. `finish_tick` runs only after a tick that completed.

- [ ] **Step 9: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-core`
  Expected: PASS, including the whole `monitor_sim` file.

- [ ] **Step 10: Commit**

```bash
git add docs/superpowers/specs/2026-10-31-netlantern-design.md crates/netlantern-core
git commit -m "Record the real-network Phase 1 and Tauri results, raise new-device alerts that a lagged listener missed, and stop monitoring within two seconds of quitting."
```

---

### Task 2: Platform seams for banners, LAN HTTP(S) and unicast mDNS

**Files:**
- Modify:
  - `crates/netlantern-platform/src/netio.rs`
  - `crates/netlantern-platform/src/net/mod.rs`
  - `crates/netlantern-platform/src/net/http.rs`
  - `crates/netlantern-platform/src/net/mdns.rs`
  - `crates/netlantern-platform/src/sim/mod.rs`
  - `crates/netlantern-platform/src/sim/scenario.rs`
  - `crates/netlantern-platform/src/lib.rs` (exports)
  - `crates/netlantern-platform/Cargo.toml` (dev-dependencies)
  - `crates/netlantern-core/src/monitor/sampler.rs` (parked item: HTTPS checks on self-signed LAN certificates)
- Create: `crates/netlantern-platform/src/net/banner.rs`
- Test:
  - `crates/netlantern-platform/tests/net.rs`
  - `crates/netlantern-platform/tests/sim.rs`
  - `crates/netlantern-platform/src/net/mdns.rs` (unit)
  - `crates/netlantern-core/tests/monitor_sim.rs`

**Interfaces:**
- Produces, in `netio.rs`:

```rust
pub struct HttpResponse {
    pub status: u16,
    pub location: Option<String>,
    /// The `Server` header.
    pub server: Option<String>,
    pub body_prefix: String,
}

/// What one device answered to a unicast mDNS query.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdnsHost {
    pub ip: Ipv4Addr,
    /// The target of the reverse (`in-addr.arpa`) PTR record, e.g. `Johns-iPhone.local`.
    pub hostname: Option<String>,
    pub services: Vec<MdnsService>,
}

/// True for `http://` or `https://` URLs whose host is an RFC 1918 or
/// loopback IPv4 literal, with an optional numeric port and no userinfo.
pub fn lan_url_allowed(url: &str) -> bool;

// New NetIo methods, each with the default body shown:
async fn mdns_query(&self, ip: Ipv4Addr, timeout: Duration)
    -> Result<Option<MdnsHost>, PlatformError> { Ok(None) }
async fn tcp_banner(&self, ip: Ipv4Addr, port: u16, timeout: Duration)
    -> Result<Option<String>, PlatformError> { Ok(None) }
async fn lan_http_get(&self, url: &str, follow_redirects: bool)
    -> Result<HttpResponse, PlatformError> {
    Err(PlatformError::Unavailable { reason: "LAN HTTP is not available".into() })
}
```

- Sim scenario additions:
  - on each device: `mdns_reverse: Option<String>`, `mdns_multicast: bool` (default `true`), `banners: BTreeMap<u16, String>`;
  - on `ScenarioHttp`: `server: Option<String>`.

- [ ] **Step 1: Write the failing platform tests**

  In `tests/net.rs` (loopback servers on ephemeral ports):

```rust
#[test]
fn lan_urls_are_private_or_loopback_literals_only() {
    for ok in ["http://192.168.1.1/", "https://10.0.0.5:8443/x", "http://172.16.0.9", "http://127.0.0.1:8080/"] {
        assert!(lan_url_allowed(ok), "{ok}");
    }
    for bad in ["http://8.8.8.8/", "http://router.lan/", "ftp://192.168.1.1/", "http://user@192.168.1.1/",
                "http://192.168.1.1:99999/", "http://172.32.0.1/", "https://[::1]/"] {
        assert!(!lan_url_allowed(bad), "{bad}");
    }
}

#[tokio::test]
async fn tcp_banner_reads_the_greeting_line() { /* listener writes "SSH-2.0-OpenSSH_9.6\r\nmore" in one write;
   expect exactly Some("SSH-2.0-OpenSSH_9.6\r\n"): text after the first \n is dropped even when it arrives in the same read */ }

#[tokio::test]
async fn tcp_banner_is_none_when_the_service_stays_silent() { /* listener accepts and sends nothing; timeout 300 ms; expect Ok(None) */ }

#[tokio::test]
async fn lan_http_get_accepts_a_self_signed_certificate() {
    /* rcgen self-signed cert for "localhost", tokio-rustls acceptor on 127.0.0.1;
       respond "HTTP/1.1 200 OK\r\nServer: TestBox\r\nContent-Length: 34\r\n\r\n<html><title>Hi</title></html>   ";
       expect status 200, server Some("TestBox"), body contains "<title>Hi</title>" */
}

#[tokio::test]
async fn lan_http_get_refuses_outside_hosts_without_connecting() {
    let err = RealNetIo::new().lan_http_get("http://8.8.8.8/", false).await.unwrap_err();
    assert!(matches!(err, PlatformError::Io { .. }));
}
```

  In `tests/sim.rs`:
  - `sim_mdns_query_answers_with_reverse_name_and_services`. A device with `mdns_reverse: "Johns-iPhone.local"`, `mdns_multicast: false` and one `_companion-link._tcp` service:
    - `mdns_query` returns that hostname and the service;
    - `mdns_browse` leaves the device out;
    - a missing address returns `Ok(None)` after the timeout.
  - `sim_tcp_banner_and_lan_http`:
    - `tcp_banner` returns the scenario banner for an open port, and `None` for a closed one;
    - `lan_http_get` returns a canned URL with its `server`, and ignores `tls_error`;
    - `lan_http_get("http://8.8.8.8/")` is an error.

  Unit test in `mdns.rs`: `reverse_name_from_messages_reads_the_ptr_target`. Build a `Message` holding a PTR answer `23.1.168.192.in-addr.arpa. → Johns-iPhone.local.` from `192.168.1.23`. Expect `Some("Johns-iPhone.local")`. The same message from another source address gives `None`.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern-platform`
  Expected: FAIL to compile (`lan_url_allowed`, `mdns_query`, `tcp_banner`, `lan_http_get` and `MdnsHost` are undefined).

- [ ] **Step 3: Add the dev-dependencies, the types and `lan_url_allowed`**

  Add `rcgen = "0.13"` and `tokio-rustls = "0.26"` under `[dev-dependencies]` of `netlantern-platform`.

  Add `server` to every `HttpResponse` literal:
  - `http.rs` reads the header;
  - the sim takes `ScenarioHttp.server`;
  - the fakes in `crates/netlantern-core/src/diagnose/judge.rs` (tests) and `crates/netlantern-core/tests/discover.rs` use `None`.

- [ ] **Step 4: Implement the real methods**
  - **`net/banner.rs`, `tcp_banner`:**
    - connect within `timeout`;
    - read until the first `\n`, 256 bytes, or `timeout`, whichever comes first;
    - return the bytes as lossy UTF-8, untrimmed. Cleaning happens in core.
    - Nothing read, or a failed connect, is `Ok(None)`.
  - **`net/http.rs`, `lan_http_get`:**
    - refuse unless `lan_url_allowed`, with `PlatformError::Io { message: "only LAN addresses are allowed" }`;
    - client: `danger_accept_invalid_certs(true)`, `no_proxy()`, the existing user agent, a 3 s total timeout, and the existing same-origin redirect policy when `follow_redirects` is set;
    - the body reading is shared with `http_get`;
    - `http_get` itself is unchanged.
  - **`net/mdns.rs`, `mdns_query`:**
    - one ephemeral UDP socket, packets sent to `ip:5353`, only datagrams from `ip` read;
    - round 1, up to `timeout / 2`: PTR for the reverse name plus PTR `_services._dns-sd._udp.local.`;
    - round 2, only if round 1 listed service types, up to the remaining time: PTR for each listed type, at most 10;
    - `hostname` comes from the new pure `reverse_name_from_messages(msgs, ip) -> Option<String>` (trailing dot dropped); services from the existing `services_from_messages`, filtered to `ip`;
    - `Ok(None)` when nothing came back.

  Wire all three into `RealNetIo`.

- [ ] **Step 5: Implement the sim side**
  - `mdns_browse` skips devices with `mdns_multicast: false`.
  - `mdns_query`: a present device that has `mdns_reverse` or any `mdns` entry returns `MdnsHost`; anything else sleeps `timeout` and returns `Ok(None)`.
  - `tcp_banner`: a present device whose port is open and listed in `banners` returns that text; otherwise `Ok(None)`.
  - `lan_http_get`: refuse unless `lan_url_allowed`; otherwise look up the existing `http` map by URL and ignore `tls_error`. A URL missing from the map is `Io { "connection refused" }`.

- [ ] **Step 6: Switch starred-device HTTP checks to `lan_http_get`**

  This is the parked Phase 3 item. In `sampler.rs`, `Check::Http` calls `netio.lan_http_get(&url, true)`.

  Add the test `https_check_passes_on_a_self_signed_device` to `monitor_sim.rs`. Use a scenario `http` entry for `https://<starred ip>:443/` with `status: 200` and `tls_error: "UnknownIssuer"`. The check reports OK with the text `HTTP 200`.

- [ ] **Step 7: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-platform` and then `cargo test -p netlantern-core`
  Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/netlantern-platform crates/netlantern-core
git commit -m "Add unicast mDNS queries, service banner reads and a LAN-only HTTP fetch that accepts self-signed certificates, and use it for starred-device HTTPS checks."
```

---

### Task 3: Ask every device for its name over unicast mDNS

**Files:**
- Modify:
  - `crates/netlantern-core/src/identify/classify.rs` (`Signals.mdns_host`)
  - `crates/netlantern-core/src/identify/names.rs` (`announced_name`)
  - `crates/netlantern-core/src/discover.rs`
  - `crates/netlantern-core/src/store/devices.rs` (`merge_evidence`)
- Create: `scenarios/private-phones.json`
- Test:
  - `crates/netlantern-core/src/identify/names.rs` (unit)
  - `crates/netlantern-core/tests/discover.rs`
  - `crates/netlantern-core/tests/scan_sim.rs`

**Interfaces:**
- Consumes: `NetIo::mdns_query`, `MdnsHost` (Task 2).
- Produces:
  - `Signals.mdns_host: Option<String>` (`#[serde(default)]`), the raw reverse name;
  - `DiscoverConfig.mdns_query_timeout: Duration`, default 1500 ms.

- [ ] **Step 1: Write the failing tests**
  - **`names.rs`, `mdns_reverse_name_is_used_after_srv_hostnames`:**
    - `Signals { mdns_host: Some("Johns-iPhone.local".into()), ..Default::default() }` gives `Some("Johns-iPhone")`;
    - with an mDNS service whose `hostname` is `Office-Mac.local`, that SRV name wins;
    - a generated reverse name (`3c1e9a7b1f22.local`) is ignored.
  - **`tests/discover.rs`, `unicast_mdns_fills_host_and_services`:**
    - a `RecordingNetIo` answers `mdns_query` for `.23` with hostname `Johns-iPhone.local` and one `_companion-link._tcp` service;
    - `discover` gives `.23` the `mdns_host` and that service, with no duplicate when multicast browse returned the same service;
    - `mdns_query` is never called for the laptop's own address.
  - **`tests/discover.rs`, `a_hanging_mdns_query_costs_at_most_its_timeout`:** with `HangingNetIo` and a paused clock, `discover` returns within `window + 1 s`, as it already does for the other methods.
  - **`tests/scan_sim.rs`, `private_mac_phones_get_named_and_typed_by_unicast_mdns`:** a quick scan of `scenarios/private-phones.json`. The phone with `mdns_reverse` gets display name `Johns-iPhone` and type `phone`. The phone without it stays `unknown`.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern-core mdns`
  Expected: FAIL (`mdns_host` is undefined).

- [ ] **Step 3: Create `scenarios/private-phones.json`**

  Copy the router and laptop from `home.json`, then add two phones:
  - `192.168.1.23`: private MAC `DA:A1:19:5C:2E:23`, ping 104 ms, `mdns_multicast: false`, `mdns_reverse: "Johns-iPhone.local"`, one `_companion-link._tcp` service;
  - `192.168.1.24`: private MAC `D6:11:22:33:44:24`, ping 106 ms, nothing announced.

  This mirrors the author's two unknown phones.

- [ ] **Step 4: Implement**
  - `discover` runs a fifth concurrent branch. It calls `mdns_query` for every present address except `iface.ip`, with concurrency `cfg.concurrency` and each call bounded by `mdns_query_timeout + REQUEST_GRACE`. Failures are logged once at debug level.
  - The fold sets `mdns_host`, and appends services whose `(service_type, instance)` is not already present.
  - `announced_name` tries `mdns_host` through `clean_name` and `usable`, right after the SRV-hostname branch.
  - `merge_evidence`: `m.mdns_host = now.mdns_host.or(m.mdns_host)`.

- [ ] **Step 5: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-core`
  Expected: PASS. The `classifies_home_devices` table is unchanged.

- [ ] **Step 6: Commit**

```bash
git add scenarios/private-phones.json crates/netlantern-core
git commit -m "Ask every present device for its name and services over unicast mDNS during discovery, so phones that ignore multicast browsing still get a name and a type."
```

---

### Task 4: The deep port catalogue, banner cleaning and the OS guess

**Files:**
- Create:
  - `crates/netlantern-core/src/identify/ports.rs`
  - `crates/netlantern-core/src/identify/os.rs`
- Modify: `crates/netlantern-core/src/identify/mod.rs`
- Test: unit tests in the two new files

**Interfaces:**
- Produces, in `identify/ports.rs`:

```rust
/// The deep scan's ports: 100, ascending, a superset of the quick scan's.
pub const DEEP_PORTS: [u16; 100] = [
    21, 22, 23, 25, 53, 80, 81, 88, 110, 111, 135, 139, 143, 389, 427, 443, 445, 465, 515, 548,
    554, 587, 631, 873, 990, 993, 995, 1025, 1080, 1400, 1433, 1723, 1883, 2000, 2049, 2222, 2869,
    3000, 3001, 3128, 3306, 3389, 3689, 4070, 4443, 5000, 5001, 5060, 5222, 5357, 5432, 5555, 5800,
    5900, 5985, 6000, 6379, 6466, 6467, 6668, 7000, 7547, 8000, 8001, 8002, 8008, 8009, 8060, 8080,
    8081, 8086, 8088, 8090, 8123, 8181, 8200, 8291, 8443, 8554, 8880, 8883, 8888, 9000, 9080, 9090,
    9091, 9100, 9197, 9295, 9443, 9999, 10000, 32400, 34567, 37777, 49152, 49153, 50000, 55443, 62078,
];

/// What the banner stage does with an open port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe { Http, Https, Greeting, Nothing }

pub fn probe_kind(port: u16) -> Probe;
pub fn service_name(port: u16) -> Option<&'static str>;
/// First non-empty line, control characters removed, trimmed, cut to 100 chars.
pub fn clean_banner(raw: &str) -> Option<String>;
/// The `<title>` (case-insensitive), with &amp; &lt; &gt; &quot; &#39; decoded,
/// whitespace collapsed, then `clean_banner`.
pub fn html_title(body: &str) -> Option<String>;
/// The page title, else `"Server: <header>"`, else None.
pub fn http_banner(resp: &netlantern_platform::HttpResponse) -> Option<String>;
```

- `probe_kind`:
  - **Http:** 80, 81, 1400, 2869, 3000, 3001, 5000, 5800, 7547, 8000, 8001, 8002, 8008, 8060, 8080, 8081, 8086, 8088, 8090, 8123, 8181, 8200, 8880, 8888, 9000, 9080, 9090, 9091, 9197, 10000, 32400, 49152, 49153, 50000.
  - **Https:** 443, 4443, 5001, 8443, 9443.
  - **Greeting:** 21, 22, 23, 25, 110, 143, 587, 2222, 3306, 5900.
  - **Nothing:** everything else.
- `service_name`: the IANA service name, or the product commonly behind the port, for every entry of `DEEP_PORTS`. Pinned names: 22 `ssh`, 80 `http`, 443 `https`, 445 `smb`, 631 `ipp`, 1400 `sonos`, 3389 `rdp`, 8009 `chromecast`, 8123 `home-assistant`, 9100 `printer`, 32400 `plex`, 62078 `iphone-sync`. Returns `None` off the list.

- Produces, in `identify/os.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OsGuess {
    /// e.g. "Windows", "iOS or iPadOS", "Linux, Android or embedded".
    pub name: String,
    /// What it rests on, e.g. "TTL 128 and ports 135, 445".
    pub basis: String,
}
pub fn os_guess(s: &Signals) -> Option<OsGuess>;
```

- `os_guess`. "Starting TTL" is 64 when the observed TTL is at most 64, 128 when at most 128, and 255 otherwise. Rules, first match wins:
  1. `is_self` → "Windows", basis "this laptop".
  2. Starting TTL 128 → "Windows", basis "TTL 128", plus " and ports …" listing any of 135, 139, 445, 3389, 5985 that are open.
  3. Starting TTL 64, and port 62078 open or `_apple-mobdev2._tcp` announced → "iOS or iPadOS", basis "TTL 64 and port 62078" or "TTL 64 and _apple-mobdev2".
  4. Starting TTL 64 and other Apple evidence → "macOS or iOS", basis "TTL 64 and <evidence>". Apple evidence: port 548, an mDNS `_companion-link._tcp`, `_airplay._tcp` or `_raop._tcp`, or vendor "Apple".
  5. Starting TTL 64 and port 5555 open → "Android", basis "TTL 64 and port 5555".
  6. Starting TTL 64 → "Linux, Android or embedded", basis "TTL 64".
  7. Starting TTL 255 → "Network equipment or embedded", basis "TTL 255".
  8. No TTL, port 62078 open → "iOS or iPadOS", basis "port 62078".
  9. No TTL, two or more of the Windows ports open → "Windows", basis "ports …".
  10. Otherwise `None`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn deep_ports_are_100_sorted_unique_and_cover_the_quick_list() {
    assert_eq!(DEEP_PORTS.len(), 100);
    assert!(DEEP_PORTS.windows(2).all(|w| w[0] < w[1]));
    for p in [80, 443, 445, 22, 62078] { assert!(DEEP_PORTS.contains(&p), "{p}"); }
    for p in DEEP_PORTS { assert!(service_name(p).is_some(), "no name for {p}"); }
    assert_eq!(service_name(12345), None);
}

#[test]
fn probe_kinds() {
    assert_eq!(probe_kind(8080), Probe::Http);
    assert_eq!(probe_kind(8443), Probe::Https);
    assert_eq!(probe_kind(22), Probe::Greeting);
    assert_eq!(probe_kind(445), Probe::Nothing);
}

#[test]
fn banners_are_cleaned_and_cut() {
    assert_eq!(clean_banner("\r\n  SSH-2.0-OpenSSH_9.6\r\nrest").as_deref(), Some("SSH-2.0-OpenSSH_9.6"));
    assert_eq!(clean_banner("220 ftp\u{0007}\u{001b}[31m ready").as_deref(), Some("220 ftp[31m ready"));
    assert_eq!(clean_banner(&"x".repeat(5000)).map(|s| s.chars().count()), Some(100));
    assert_eq!(clean_banner(" \r\n\t"), None);
    assert_eq!(clean_banner("caf\u{FFFD} box").as_deref(), Some("caf\u{FFFD} box"));
}

#[test]
fn titles_are_decoded_and_never_markup() {
    assert_eq!(html_title("<HTML><TiTlE>\n  Router &amp; Modem\n</title>").as_deref(), Some("Router & Modem"));
    assert_eq!(html_title("<title><script>alert(1)</script></title>").as_deref(), Some("<script>alert(1)</script>"));
    assert_eq!(html_title("<title></title>"), None);
    assert_eq!(html_title("no title here"), None);
}

#[test]
fn http_banner_prefers_the_title_then_the_server_header() { /* title wins; no title + server "lighttpd" → "Server: lighttpd"; neither → None */ }
```

  The `<script>` title comes back as plain text. It is safe because the UI renders text only (Task 10).

  In `os.rs`, one test per rule. The exact strings are pinned:
  - TTL 128 with ports [135, 445] → `("Windows", "TTL 128 and ports 135, 445")`;
  - TTL 57 with [62078] → `("iOS or iPadOS", "TTL 64 and port 62078")`;
  - TTL 64, vendor Apple → `("macOS or iOS", "TTL 64 and vendor Apple")`;
  - TTL 64 with [5555] → `("Android", "TTL 64 and port 5555")`;
  - TTL 63 → `("Linux, Android or embedded", "TTL 64")`;
  - TTL 250 → `("Network equipment or embedded", "TTL 255")`;
  - no TTL with [62078] → `("iOS or iPadOS", "port 62078")`;
  - no TTL with [445, 3389] → `("Windows", "ports 445, 3389")`;
  - no TTL with [445] → `None`;
  - `is_self` → `("Windows", "this laptop")`.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern-core identify::`
  Expected: FAIL (the modules don't exist).

- [ ] **Step 3: Implement both modules**

  Text-only and allocation-light. No regex crate: a case-insensitive `find` on a lower-cased copy is enough for `<title>`.

- [ ] **Step 4: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-core identify::`
  Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/netlantern-core/src/identify
git commit -m "Add the 100-port deep scan list with service names, banner and page-title cleaning, and a TTL and open-port OS guess."
```

---

### Task 5: Schema v3, port details and consent in the store

**Files:**
- Modify:
  - `crates/netlantern-core/src/store/migrations.rs` (`SCHEMA_V3`)
  - `crates/netlantern-core/src/store/devices.rs`
  - `crates/netlantern-core/src/store/networks.rs`
  - `crates/netlantern-core/src/model.rs`
  - `crates/netlantern-core/src/scan/merge.rs` (`deep: None`)
- Test: `crates/netlantern-core/tests/store.rs`

**Interfaces:**
- Consumes: `service_name`, `os_guess`, `OsGuess` (Task 4).
- Produces, in `model.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortInfo { pub port: u16, pub service: Option<String>, pub banner: Option<String> }

/// What a deep scan learned about one device's ports.
#[derive(Clone, Debug)]
pub struct DeepFindings {
    /// Every port that was probed on this device.
    pub scanned: Vec<u16>,
    /// The open ones, with what they said.
    pub ports: Vec<PortInfo>,
}

// Sighting gains:   pub deep: Option<DeepFindings>,
// Device gains:     #[serde(default)] pub ports: Vec<PortInfo>,
//                   #[serde(default)] pub os_guess: Option<OsGuess>,
//                   #[serde(default)] pub deep_scanned_at: Option<i64>,
```

- Produces, in `networks.rs`:
  - `Store::scan_consent(&self, network: NetworkId) -> Result<bool, StoreError>`;
  - `Store::set_scan_consent(&self, network: NetworkId, granted: bool) -> Result<(), StoreError>`.

- `SCHEMA_V3`: `ALTER TABLE devices ADD COLUMN deep_scanned_at INTEGER;`

- **`record_sighting` rules:**
  - Every port written gets `service = service_name(port)`. This applies to quick scans too.
  - When `deep` is `Some` and the device responded (`answers_ping` or at least one open port):
    - delete the `device_ports` rows for ports in `scanned` that are not open now, and drop the same ports from the merged evidence's `open_ports`;
    - upsert each `PortInfo`. The banner is overwritten only by a new `Some`: `banner = COALESCE(excluded.banner, banner)`;
    - set `deep_scanned_at = now`.
  - When `deep` is `Some` but the device did not respond, port rows and `deep_scanned_at` are left as they were.
  - Port rows for ports outside `scanned` are never deleted.
- **`load`:**
  - fills `ports` from `device_ports` (port order);
  - `open_ports` stays the list of port numbers;
  - `os_guess = os_guess(&evidence)` from the stored evidence JSON;
  - reads `deep_scanned_at`.

- [ ] **Step 1: Write the failing tests** in `tests/store.rs`

  - **`deep_sighting_replaces_scanned_ports_and_keeps_others`:**
    - A quick sighting records ports [22, 80, 9999]. Port 9999 is not in `scanned` in this test: use `scanned: vec![22, 80, 443]`.
    - A deep sighting with `answers_ping: true` finds port 443 open, banner `"Router login"`.
    - The device's `ports` are now `[(443, Some("https"), Some("Router login")), (9999, Some(..), None)]`, `open_ports == [443, 9999]`, and `deep_scanned_at == Some(now)`.
  - **`a_silent_device_keeps_its_ports_after_a_deep_scan`:** a quick sighting records [22]; then a deep sighting with `answers_ping: false` and no open ports. The device keeps [22], and `deep_scanned_at` stays `None`.
  - **`a_missing_banner_keeps_the_old_one`:** deep sighting 1 gives 80 with banner `"NAS"`; deep sighting 2 gives 80 open with banner `None`. The banner is still `"NAS"`.
  - **`os_guess_comes_from_the_evidence`:** a sighting with TTL 128 and ports [445, 3389] gives `os_guess == Some(OsGuess { name: "Windows", basis: "TTL 128 and ports 445, 3389" })`.
  - **`scan_consent_defaults_off_and_persists`:** `false` for a new network; `true` after `set_scan_consent(net, true)`, and still `true` after reopening the file-backed store; `false` again after `set_scan_consent(net, false)`.
  - **`v2_database_upgrades_to_v3`:** create a v2 file with `MIGRATIONS[..2]` and one device row, reopen it with the store, and assert `PRAGMA user_version == 3`, the device loads with `deep_scanned_at: None`, and a `.bak` file exists.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern-core --test store`
  Expected: FAIL to compile (`DeepFindings`, `ports` and `scan_consent` are undefined).

- [ ] **Step 3: Implement**

  The migration, the model fields (`Sighting { deep: None, .. }` everywhere a sighting is built today), the store rules above, and the consent functions.

- [ ] **Step 4: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-core`
  Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/netlantern-core
git commit -m "Store each open port's service and banner, let a deep scan replace the ports it checked, compute the OS guess on load, and keep scan consent per network in schema v3."
```

---

### Task 6: The deep scan and its consent check

**Files:**
- Modify:
  - `crates/netlantern-core/src/model.rs` (`ScanStage`)
  - `crates/netlantern-core/src/error.rs`
  - `crates/netlantern-core/src/scan.rs`
  - `crates/netlantern-core/src/scan/stages.rs`
  - `scenarios/home.json` (banners and pages for the demo and tests)
- Test: `crates/netlantern-core/tests/scan_sim.rs`

**Interfaces:**
- Consumes:
  - `DEEP_PORTS`, `probe_kind`, `service_name`, `clean_banner`, `http_banner` (Task 4);
  - `DeepFindings`, `PortInfo`, `scan_consent`, `set_scan_consent` (Task 5);
  - `tcp_banner`, `lan_http_get` (Task 2).
- Produces:
  - `ScanStage::PortScan` (`"port_scan"`) and `ScanStage::Banners` (`"banners"`), used only by deep scans. Their `done`/`total` count probes, not addresses. The snapshot's `total` follows the progress events.
  - `CoreError::ConsentNeeded` (message "deep scan needs permission for this network") and `CoreError::Unidentified(String)` (the user-facing reason).
  - `ScanConfig` additions: `deep_ports: Vec<u16>` (default `DEEP_PORTS.to_vec()`), `deep_per_host: usize` (8), `banner_timeout: Duration` (2 s), `banner_concurrency: usize` (16).
  - `ScanManager::scan_consent(&self) -> Result<Option<(NetworkId, bool)>, CoreError>`: passive. `None` when the laptop is on a network not yet identified.
  - `ScanManager::set_scan_consent(&self, granted: bool) -> Result<NetworkId, CoreError>`: identifies the network actively, as a scan does (creating it if new), stores the flag, and remembers the network for `current_network`. A router that doesn't answer is `Unidentified(<the existing "Your router at … didn't answer" text>)`.
  - `ScanManager::start(ScanMode::Deep)`:
    - a running scan is still reported first;
    - then `current_network()` plus `scan_consent`; a missing or false consent is `Err(ConsentNeeded)`, and no scan row is created.

- **Deep pipeline** (`Inner::quick_scan` becomes `Inner::scan(&self, id, mode, iface, targets)`):
  1. network, sweep and ARP as today;
  2. **`port_scan`** replaces `ports`:
     - every present address except the laptop, against `deep_ports`;
     - probes ordered port-major (all addresses for port 1, then port 2, and so on);
     - total concurrency `tcp_concurrency`, and at most `deep_per_host` in flight per address (one `tokio::sync::Semaphore` per address);
     - `ScanProgress { stage: PortScan, done, total }` every 64 probes and at the end;
  3. **`banners`**:
     - every open `(ip, port)` whose `probe_kind` is not `Nothing`, concurrency `banner_concurrency`;
     - `Http` uses `lan_http_get("http://{ip}:{port}/", false)` then `http_banner`; `Https` does the same with `https://`; `Greeting` uses `tcp_banner(ip, port, banner_timeout)` then `clean_banner`;
     - each call is bounded by `banner_timeout + 500 ms`, and an error or timeout means no banner (logged once at debug level);
     - progress as for `port_scan`;
  4. discovery and merge as today. Each non-self sighting carries `deep: Some(DeepFindings { scanned: deep_ports.clone(), ports })`, with `ports` from steps 2 and 3, each port's `service` set by `service_name`.

  Quick scans are unchanged apart from the rename.

- [ ] **Step 1: Extend `scenarios/home.json`**

  This only adds fields; existing devices keep everything they have.
  - Router `.1`: open ports 53, 80, 443. Add `http` entries: `http://192.168.1.1/` 200 with body `<title>Archer AX55</title>`, and `https://192.168.1.1/` 200, `tls_error: "UnknownIssuer"`, `server: "lighttpd"`.
  - NAS `.70`: `banners: { "22": "SSH-2.0-OpenSSH_9.6\r\n" }`, open port 5000, and `http://192.168.1.70:5000/` with title `DiskStation`.
  - Hue `.130`: `http://192.168.1.130/` with title `hue personal wireless lighting`.

- [ ] **Step 2: Write the failing tests** in `scan_sim.rs`

  - **`deep_scan_needs_consent_and_creates_no_scan`:** a quick scan of `home`, then `start(Deep)`: `Err(CoreError::ConsentNeeded)`, and `latest_scan` is still the quick scan.
  - **`consent_on_a_new_network_identifies_it`:** on a fresh store, `scan_consent()` is `Ok(None)`. Then `set_scan_consent(true)` returns a `NetworkId`, `scan_consent()` is `Ok(Some((id, true)))`, and `start(Deep)` returns `Started`.
  - **`consent_needs_the_router`:** with `scenarios/router-down.json` at t=0 (gateway absent), `set_scan_consent(true)` is `Err(CoreError::Unidentified(msg))` with `msg` starting "Your router at 192.168.1.1".
  - **`deep_scan_finds_ports_banners_and_titles`:** consent, then a deep scan of `home` to `scan.done` (paused clock):
    - `.1` has ports `[(53, "dns", None), (80, "http", "Archer AX55"), (443, "https", "Server: lighttpd")]`;
    - `.70` port 22 has banner `"SSH-2.0-OpenSSH_9.6"` and port 5000 has banner `"DiskStation"`;
    - every non-self device has `deep_scanned_at` set;
    - the laptop's `deep_scanned_at` is `None`.
  - **`deep_scan_progress_counts_probes`:** the events include `ScanProgress { stage: PortScan, total: 100 × <present devices other than the laptop>, .. }` ending with `done == total`, then `Banners` events, then `ScanDone { state: Done }`.
  - **`per_device_connects_stay_at_eight`:** a `CountingPlatform` wrapper (the file has one; extend it) tracks connects in flight per address, each `tcp_probe` sleeping 100 ms. Over a deep scan of `isolated.json` (only the gateway present), the peak for the gateway is exactly 8 and never more.
  - **`a_hanging_banner_read_costs_its_timeout`:** a `NetIo` wrapper whose `tcp_banner` never returns. A deep scan of `home` still finishes `Done`, and port 22 on `.70` has no banner.

- [ ] **Step 3: Run them to make sure they fail**

  Run: `cargo test -p netlantern-core --test scan_sim deep consent per_device hanging_banner`
  Expected: FAIL (`start(Deep)` returns `Unsupported`; the new methods don't exist).

- [ ] **Step 4: Implement**

  The stages, the config fields, the consent methods and the errors. Remove `CoreError::Unsupported("deep scan arrives in phase 4")` from `start`.
  - Keep `Unsupported` in the enum, but update its unit test to use a neutral message (`"not yet"`).
  - `identify_network` returns `Result<_, Failure>`; `set_scan_consent` maps that `Failure(msg)` to `Unidentified(msg)`.

- [ ] **Step 5: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-core`
  Expected: PASS. Every existing quick-scan test is unchanged.

- [ ] **Step 6: Commit**

```bash
git add scenarios/home.json crates/netlantern-core
git commit -m "Run deep scans over 100 ports with at most eight connects per device, read banners and page titles from open ports, and refuse a deep scan until the network has consent."
```

---

### Task 7: Stopping a scan, and stopping when the network changes

**Files:**
- Modify:
  - `crates/netlantern-core/src/scan.rs`
  - `crates/netlantern-core/src/error.rs`
- Test: `crates/netlantern-core/tests/scan_sim.rs`

**Interfaces:**
- Produces:
  - `ScanManager::cancel(&self, id: ScanId) -> bool`: true when `id` is the running scan and it has now been told to stop. False for a finished or unknown id.
  - `ScanConfig.network_watch: Duration`, default 2 s.
  - Cancelled outcomes, published in `scan.done` and stored:
    - a user stop is `state: Cancelled, error: None`;
    - a network change is `state: Cancelled, error: Some("The network changed during the scan.")`;
    - a lost connection is `state: Cancelled, error: Some("The connection dropped during the scan.")`.

  Devices recorded before the stop are kept, `devices_found` counts them, and `isolated` is false.

- **Mechanism:**
  - `start` creates a `tokio::sync::watch::channel::<Option<Option<String>>>(None)`. The outer `Some` means stop; the inner value is the reason. The sender is stored next to `running`.
  - `run_scan` `select!`s three futures, `biased` in this order:
    1. the stop signal;
    2. the scan;
    3. a network watcher. Every `network_watch` it reads `active_interface`: `NotConnected` sends the connection reason; an `InterfaceKey` different from the scan's sends the network-change reason; other errors are ignored.
  - Dropping the scan future abandons in-flight probes. Store calls that already started still finish on the database thread.
  - The finish critical section is unchanged, so `scan.done` still goes out exactly once and frees the guard.

- [ ] **Step 1: Write the failing tests**

  - **`a_stopped_deep_scan_ends_cancelled_and_keeps_its_devices`:**
    - `set_scan_consent(true)`, then a deep scan of `home` with each `tcp_probe` slowed to 1 s (wrapper);
    - after the first `ScanProgress { stage: PortScan }`, call `cancel(id)` and assert it returns `true`;
    - expect `ScanDone { state: Cancelled, error: None }`, the stored scan `cancelled`, `list_devices` non-empty, and `cancel(id)` now `false`;
    - an immediate `start(Quick)` returns `Started`.
  - **`a_network_change_cancels_the_scan_with_a_reason`:** the file's `Switchable` platform switches the gateway mid-sweep. Expect `ScanDone { state: Cancelled, error: Some("The network changed during the scan.") }`.
  - **`losing_the_connection_cancels_the_scan_with_a_reason`:** `Switchable` returns `NotConnected` mid-sweep. Expect the connection reason.
  - **`cancel_of_an_unknown_scan_is_false`:** `cancel(ScanId(42))` with nothing running is `false`.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern-core --test scan_sim cancel network_change losing_the_connection`
  Expected: FAIL (no `cancel` method; a gateway change goes unnoticed).

- [ ] **Step 3: Implement as described**

- [ ] **Step 4: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-core`
  Expected: PASS. Task 1's `new_device_alert_follows_scan_done` also passes for a `Cancelled` scan: add the case `state: Cancelled` to that test as a second assertion block.

- [ ] **Step 5: Commit**

```bash
git add crates/netlantern-core
git commit -m "Let a running scan be stopped, and stop it with a reason when the network changes or the connection drops."
```

---

### Task 8: Server routes for consent, cancel and deep scans

**Files:**
- Modify:
  - `crates/netlantern-server/src/routes.rs`
  - `crates/netlantern-server/src/error.rs`
- Test: `crates/netlantern-server/tests/api.rs`

**Interfaces:**
- Consumes: `ScanManager::scan_consent`, `set_scan_consent`, `cancel`, `start(Deep)`; `CoreError::ConsentNeeded` and `Unidentified`.
- Produces:
  - **`GET /api/network`:** the `Interface` fields flattened, plus `network_id: number | null` and `scan_consent: bool`. Consent is false when the network is not identified.
  - **`PUT /api/network/consent`** with body `{"granted": bool}`:
    - `200 {"network_id": n, "scan_consent": bool}`;
    - `503 not_connected`;
    - `503 network_unidentified`, with the core message and hint "Check that you're connected to your router, then try again.";
    - `400 bad_request` for a malformed body.
  - **`POST /api/scans {mode: "deep"}`** without consent: `403 consent_needed`, message "Deep scan needs your permission for this network.", hint "Confirm that you own this network or have permission to scan it."
  - **`POST /api/scans/{id}/cancel`:**
    - `202` with the scan's current snapshot;
    - `409 scan_not_running`, message "That scan has already finished.", when the scan exists but is not running;
    - `404 not_found` for an unknown id.

- [ ] **Step 1: Write the failing tests** in `tests/api.rs` (`home` scenario, signed-in client)

  - `network_reports_consent_and_id`:
    - before any scan: `network_id: null, scan_consent: false`;
    - after a quick scan: a number and `false`;
    - after `PUT /api/network/consent {granted: true}`: `true`.
  - `deep_scan_without_consent_is_403_consent_needed`: the body code is `consent_needed` and the hint matches.
  - `consent_then_deep_scan_runs_to_done`: `PUT` consent, then `POST /api/scans {mode:"deep"}` → 202. Poll `GET /api/scans/{id}` until `done`. `GET /api/devices` shows `.1` with a `ports` entry `{port: 80, service: "http", banner: "Archer AX55"}`, an `os_guess` object, and a numeric `deep_scanned_at`.
  - `cancel_routes`:
    - cancelling a running deep scan → 202, and the scan soon reads `cancelled`;
    - cancelling it again → 409 `scan_not_running`;
    - `POST /api/scans/9999/cancel` → 404.
  - `consent_with_a_dead_router_is_503_network_unidentified`: uses `router-down.json`.
  - Replace the existing `ApiError::from(CoreError::Unsupported("deep scan arrives in phase 4"))` test case with the neutral message from Task 6.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern-server --test api consent deep cancel network_reports`
  Expected: FAIL (404 for the new routes, 501 for deep scans).

- [ ] **Step 3: Implement the routes and errors**

- [ ] **Step 4: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern-server`
  Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/netlantern-server
git commit -m "Serve scan consent with the network, add routes to grant consent and stop a scan, and answer deep scans without consent with consent_needed."
```

---

### Task 9: `--scan-once --deep`

**Files:**
- Modify:
  - `crates/netlantern/src/cli.rs`
  - `crates/netlantern/src/main.rs` (`scan_once`)
  - `crates/netlantern/src/report.rs`
- Test:
  - `crates/netlantern/src/cli.rs` (unit)
  - `crates/netlantern/src/report.rs` (unit)
  - `crates/netlantern/tests/exe.rs`

**Interfaces:**
- Produces:
  - `Cli.deep: bool` (`--deep`, `requires = "scan_once"`);
  - `report::deep_lines(devices: &[Device]) -> Vec<String>`: one line per device that has ports, `"{ip}  {port} {service} \"{banner}\" · {port} {service} …"`. The banner is omitted when `None`, and the OS guess is appended as `"  OS guess: {name} ({basis})"`.
- **Behaviour:**
  - With `--deep`, `scan_once` first prints to stderr: `Deep scan: only scan networks you own or have permission to scan.` It then calls `set_scan_consent(true)` on its throwaway store and starts a deep scan.
  - After the existing table (left unchanged), it prints a blank line and then `deep_lines`.

- [ ] **Step 1: Write the failing tests**
  - `cli.rs`: `--scan-once --deep` parses; `--deep` alone is a clap error.
  - `report.rs`, `deep_lines_list_ports_banners_and_the_guess`: one device `.1` with ports `[(80, http, "Archer AX55"), (443, https, None)]` and guess `("Linux, Android or embedded", "TTL 64")` gives exactly `192.168.1.1  80 http "Archer AX55" · 443 https  OS guess: Linux, Android or embedded (TTL 64)`.
  - `tests/exe.rs`, `demo_scan_once_deep_prints_ports`: `netlantern --demo=scenarios/home.json --scan-once --deep` exits 0. Stdout contains `80 http "Archer AX55"`; stderr contains the consent line.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `cargo test -p netlantern`
  Expected: FAIL (no `--deep`).

- [ ] **Step 3: Implement**

- [ ] **Step 4: Run the tests to make sure they pass**

  Run: `cargo test -p netlantern`
  Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/netlantern
git commit -m "Add --deep to --scan-once, printing each device's open ports, banners and OS guess after the table."
```

---

### Task 10: Deep scan in the UI

**Files:**
- Modify:
  - `ui/src/api/queries.ts` (types, `setScanConsent`, `cancelScan`)
  - `ui/src/pages/Dashboard.tsx`
  - `ui/src/components/DeviceDetails.tsx`
  - `ui/src/test/fixtures.ts`
- Create: `ui/src/components/ConsentPrompt.tsx`
- Test:
  - `ui/src/pages/Dashboard.test.tsx`
  - `ui/src/components/DeviceDetails.test.tsx`

**Interfaces:**
- Consumes: the Task 8 routes and fields.
- Produces, in `queries.ts`:
  - the `Network` type gains `network_id: number | null` and `scan_consent: boolean`;
  - `Device` gains `ports: { port: number; service: string | null; banner: string | null }[]`, `os_guess: { name: string; basis: string } | null` and `deep_scanned_at: number | null`;
  - `setScanConsent(granted: boolean): Promise<void>` invalidates `["network"]`;
  - `cancelScan(id: number): Promise<void>`.
- `ConsentPrompt({ onConfirm, onCancel, busy })`: a `role="dialog"` card (not a modal overlay) with `aria-labelledby` its heading.
  - Heading: "Is this a network you own or have permission to scan?"
  - Body: "A deep scan checks 100 common ports on every device and reads what they say about themselves. Only scan networks you own or have permission to scan."
  - Buttons: "Yes, I have permission" and "Not now".
- **Dashboard:**
  - The Deep scan button is enabled, and the "Arrives in phase 4" note is gone.
  - Clicking it with `scan_consent` false shows the prompt. "Yes" calls `setScanConsent(true)` and then starts the deep scan. A `consent_needed` answer from the server also shows the prompt.
  - While a scan runs, a "Stop scan" button calls `cancelScan`. It reads "Stopping…" and is disabled until `scan.done` arrives.
  - `STAGE_LABEL` gains `port_scan: "Scanning ports"` and `banners: "Reading what open ports say"`. The "x of y" count shows for `sweep`, `port_scan` and `banners`.
  - The cancelled summary is `The scan stopped: ${error}` when there is an error, else "The scan was cancelled."
- **DeviceDetails:**
  - The "Open ports" row lists one `<li>` per port: `{port} {service}`, plus ` — {banner}` when present. With no ports it reads "None of the 100 ports checked" after a deep scan, and "None of the common ports" otherwise.
  - New row "OS": `{name} · guess, from {basis}`, or "Unknown".
  - New row "Deep scan": `formatAgo(deep_scanned_at)`, or "Not yet — a deep scan checks 100 common ports".

- [ ] **Step 1: Write the failing tests**

  Dashboard (`mockFetch`):
  - `deep scan asks for consent first, then starts`: network `scan_consent: false`. Click "Deep scan": the dialog appears and no `POST /api/scans` has been sent. Click "Yes, I have permission": expect `PUT /api/network/consent {granted:true}`, then `POST /api/scans {mode:"deep"}`.
  - `"Not now" closes the prompt without scanning`.
  - `deep scan with consent starts straight away`.
  - `a consent_needed answer shows the prompt`.
  - `stop scan cancels the running scan`: a running scan in the live store. Click "Stop scan": `POST /api/scans/7/cancel` is sent and the button reads "Stopping…".
  - `port scan progress shows probe counts`: live scan `{stage: "port_scan", done: 640, total: 2000}` shows "Scanning ports · 640 of 2000".
  - `a network change shows its reason`: `{state: "cancelled", error: "The network changed during the scan."}` shows "The scan stopped: The network changed during the scan."
  - Replace the old "shows a disabled Deep scan button with its phase" test.

  DeviceDetails:
  - `lists ports with services and banners`.
  - `renders banner markup as text`: a banner `"<script>alert(1)</script>"` appears verbatim through `getByText`, and the container holds no `script` element.
  - `shows the OS guess labelled as a guess`: text "Windows · guess, from TTL 128 and ports 135, 445".
  - `says when no ports answered a deep scan`.
  - `invites a deep scan when there has been none`.

- [ ] **Step 2: Run them to make sure they fail**

  Run: `npm --prefix ui test -- --run Dashboard DeviceDetails`
  Expected: FAIL.

- [ ] **Step 3: Implement**

  Update `fixtures.ts` devices with `ports`, `os_guess: null` and `deep_scanned_at: null`. Plain React text nodes only; never `dangerouslySetInnerHTML`.

- [ ] **Step 4: Run the full UI checks**

  Run: `npm --prefix ui test -- --run`, `npm --prefix ui run lint`, `npm --prefix ui run build`
  Expected: PASS, no lint errors.

- [ ] **Step 5: Commit**

```bash
git add ui/src
git commit -m "Enable Deep scan with a one-time permission prompt, add a Stop button, and show each device's ports, banners, OS guess and last deep scan."
```

---

### Task 11: Spec records and the Phase 4 checkpoint

**Files:**
- Modify: `docs/superpowers/specs/2026-10-31-netlantern-design.md`
- Create: `docs/checkpoints/phase-4.md`

**Interfaces:** none (documentation).

- [ ] **Step 1: Record the phase in the spec**
  - **§6.3:** a *(changed in phase 4)* note. The deep scan is the quick-scan pipeline with the 100-port list (at most 8 connects in flight per device) and a banner stage (HTTP/HTTPS titles via a LAN-only fetch that accepts self-signed certificates, greetings for 10 text protocols). The OS guess is computed from the starting TTL plus open ports and mDNS (the rules in `identify/os.rs`), and is shown as a guess. A deep scan replaces the rows for the ports it checked only on devices that responded.
  - **§6.2 step 4:** *(changed in phase 4)* discovery also sends each present device a unicast mDNS query (reverse name and service list, 1.5 s) from an ephemeral port.
  - **§6.12, Deep scan rows:** the port list ("100 ports, `DEEP_PORTS` in `identify/ports.rs`"), and the values from this plan's Global Constraints table, each marked *(changed in phase 4)*. Add the monitor stop grace under Monitoring.
  - **§7:** `devices` gains `deep_scanned_at` *(phase 4)*; `device_ports.service` and `banner` are now filled.
  - **§8:**
    - routes: `PUT /api/network/consent {granted}`; `POST /api/scans/:id/cancel` returns `409 scan_not_running` for a finished scan;
    - `GET /api/network` adds `network_id` and `scan_consent`;
    - errors: add `consent_needed`, `network_unidentified`, `scan_not_running` *(Phase 4)*;
    - scan stages: add `port_scan`, `banners`.
  - **§10:** "Network change mid-scan" gets *(done in phase 4: the scan ends `cancelled` with the reason)*.

- [ ] **Step 2: Write `docs/checkpoints/phase-4.md`**

  Follow the shape of `docs/checkpoints/phase-3.md` (numbered items, each with steps and a blank "Result:" line). Items:
  1. Consent prompt on the home network (first deep scan asks; the second does not; a quick scan never asks).
  2. Deep scan time on the home /24, and the number of devices. There is no target: record it.
  3. Ports and banners sanity check: compare the router's ports and page title with its admin page.
  4. OS guess on the laptop (Windows), a phone and the router: right, wrong, or none.
  5. Type rate again, after the unicast mDNS change: how many of the devices now have a type. Compare with 2 of 4 on 2026-11-13. This decides the deferred "best-guess phone" rule.
  6. Stop button mid deep scan.
  7. Turn Wi-Fi off mid scan: the scan stops with the connection reason.
  8. Star the router and add an HTTPS check on port 443: it passes despite the router's own certificate.
  9. Quit the exe while monitoring: it exits within about 2 s.
  10. Run unelevated: everything above works from a normal (non-admin) window.
  11. `--scan-once --deep` on the home network: paste the output.

- [ ] **Step 3: Run the full checks**

  Run:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `npm --prefix ui test -- --run`
  - `npm --prefix ui run lint`
  - `npm --prefix ui run build`

  Expected: all PASS.

- [ ] **Step 4: Commit**

```bash
git add docs
git commit -m "Record the Phase 4 deep scan in the spec and add the Phase 4 home-network checkpoint."
```
