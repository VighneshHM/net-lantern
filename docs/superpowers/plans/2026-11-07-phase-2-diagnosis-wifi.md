# Net Lantern Phase 2: Diagnosis and Wi-Fi Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Diagnose screen checks the connection one layer at a time (Wi-Fi link, router, internet, DNS, website). It says which layer failed in plain language and suggests a fix. It also shows the current Wi-Fi connection, nearby networks and how crowded each channel is. When Windows hides Wi-Fi details, it explains how to turn on location access.

**Architecture:**
- `netlantern-platform` gains:
  - Wi-Fi types and `Platform::wifi_current` / `wifi_scan`, implemented on Windows with the WLAN API and in the simulator from scenario files;
  - a `location` header and a certificate-error variant for `http_get`;
  - outside-internet hosts, DNS servers that fail, and canned HTTPS answers in scenarios.
- `netlantern-core` gains:
  - a pure channel-congestion analysis and a `WifiService` that runs one radio scan at a time and caches the result;
  - a `diagnose` module made of three parts:
    - probe structs that hold raw results;
    - pure judge functions that turn each layer's probe into a `LayerResult`, plus a `verdict` function;
    - a `DiagnoseManager` that gathers probes layer by layer, streams `diagnose.step` events and stops at the first failed layer.
- The server adds the diagnose and Wi-Fi routes. The exe adds `--diagnose-once` and `--wifi-once`. The UI replaces the Diagnose placeholder.

**Tech Stack:** Phase 1's stack plus:
- `windows` feature `Win32_NetworkManagement_WiFi`
- `rustls` 0.23 (`default-features = false`) as a direct dependency of `netlantern-platform`, only to recognise certificate errors inside `reqwest`'s error chain

There are no new UI libraries. The congestion chart is plain SVG.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`. Relevant sections:
- the Phase 2 row of §12;
- §2.1 (diagnosis target), §3 (location rule), §5.1;
- §6.6, §6.7, §6.12, §8, §9 and journeys A and C in §9.1;
- §10, §11, the Phase 2 learning goals in §13, and §16 (location-rule risk).

Earlier plans: `docs/superpowers/plans/2026-10-31-phase-0-foundations.md` and `docs/superpowers/plans/2026-11-06-phase-1-quick-scan.md`.

**Branch:** `phase-2-diagnosis-wifi`, cut from `phase-1-quick-scan` (a0cb610). Phases 0 and 1 are not merged yet, so this branch stacks on them.

## Global Constraints

- **Windows 11, no admin rights for anything.** CI runs on `windows-latest`. Never parse the text output of OS tools (`netsh wlan`, `ping`, `nslookup`).
- **IPv4 only.**
- **Outside traffic happens only during a diagnosis the user starts** (button, `POST /api/diagnose` or `--diagnose-once`). Nothing is scheduled in this phase. The complete list:
  - ICMP echo to `1.1.1.1` and `8.8.8.8`;
  - a TCP connect to `1.1.1.1:443`, closed straight away;
  - plain-HTTP GET `http://www.msftconnecttest.com/connecttest.txt`, redirects not followed;
  - DNS A queries for `www.microsoft.com` and `www.cloudflare.com`, sent to the system DNS servers and to `1.1.1.1`;
  - HTTPS GET `https://www.microsoft.com/`, redirects not followed.

  No scan data, device data or identifiers are sent. The README lists these endpoints (spec §6.6).
- **Radio scans only on demand.** `wifi_scan` runs only from `POST /api/wifi/scan` (the Diagnose screen opening or Refresh), from a diagnosis, or from `--wifi-once`. One scan runs at a time. The monitoring scheduler (phase 3) never calls it.
- **Diagnosis defaults.** Spec §6.12 values are kept. Values new in this plan are recorded in §6.12 in Task 12:

  | Setting | Value | New? |
  |---|---|---|
  | Wi-Fi warn below | −70 dBm | no |
  | Gateway pings | 5, timeout 1 s each | timeout is new |
  | Router warn | any loss of the 5 pings (> 5%), or average RTT > 50 ms | 50 ms is new |
  | ARP fallback when the gateway answers no ping | `arp_resolve`, timeout 3 s | new |
  | Internet pings | 3 per target, timeout 1 s | new |
  | TCP 443 to 1.1.1.1 | timeout 2 s | new |
  | Internet warn latency | average > 100 ms (the same number as the §6.9 alert) | new |
  | Captive-portal expected body | `Microsoft Connect Test` | new |
  | DNS test names | `www.microsoft.com`, `www.cloudflare.com` | new |
  | Website URL | `https://www.microsoft.com/` | new |
  | Per-layer time limit | 15 s | new |
  | Nearby scan wait after `WlanScan` | 4 s | new |
  | Reuse a nearby scan | 10 s for `POST /api/wifi/scan`, 60 s for a diagnosis | new |
  | Crowded channel | ≥ 5 overlapping networks on 2.4 GHz; ≥ 3 on the same channel on 5 or 6 GHz | new |
  | Signal labels | ≥ −50 Excellent, ≥ −60 Good, ≥ −70 Fair, else Weak | new |

- **Diagnosis target (§2.1):** the correct verdict in **100%** of simulated scenarios. On the real network, a wrong DNS server and an unplugged WAN cable each produce the right verdict.
- **Location permission:**
  - Windows' `ERROR_ACCESS_DENIED` from the WLAN API maps to `PlatformError::PermissionNeeded { what: "location" }`.
  - The API returns it as `403` with code `location_permission`.
  - It never fails a diagnosis: the link layer still passes on the interface alone.
- Never log tokens or session ids. Evidence strings taken from the network (redirect `Location`, error text) are cut to 100 characters.
- **API:**
  - `API_VERSION` stays **1**. New routes and events are additions.
  - Every error body is `{"code","message","hint"}`.
  - Timestamps are milliseconds since the Unix epoch, as in Phase 1.
  - All JSON enums are `snake_case`, except `WifiBand`: `"2.4ghz"`, `"5ghz"`, `"6ghz"`.
- **UI:** status is always shown as text, touch targets are ≥ 44 px, it works at phone width with no horizontal page scroll, and colours come only from the tokens.
- **Commit messages are one plain sentence.** No `feat:` prefix, no body, no `Co-Authored-By` and no Claude or Anthropic attribution. The author is the sole author.

## Review Focus

1. **The router ignores ping** (some ISP routers do). Expected: the Router layer warns "doesn't answer ping, but it is reachable" when the gateway answers ARP, and diagnosis continues. It must never say "router down". Tested in Task 5 (`judge_router`) and Task 6 (`router-silent` scenario).
2. **The network blocks ICMP to the internet** (some offices and ISPs) while TCP and HTTP work. Expected: the Internet layer passes and says pings are blocked; it is not "internet down". Tested in Task 5.
3. **Captive portal edge cases:**
   - The portal blocks ping and outside DNS. Expected: "Sign in to this network", not "Internet down" or a DNS verdict.
   - The probe fails only because DNS is broken. Expected: never treated as a portal. The DNS layer gives the verdict.

   Tested in Task 5 and Task 6 (`cafe`, `dns-failure`).
4. **Location access is off, the laptop is on Ethernet, or there is no Wi-Fi adapter.** Expected:
   - diagnosis still runs all five layers;
   - the Wi-Fi card explains what is missing and the one action that fixes it;
   - nothing shows a raw error.

   Tested in Task 6 (`location-denied`), Task 7 (403 and 503 bodies) and Task 11 (UI states).
5. **Run twice, a second tab, or the WebSocket drops mid-run.** Expected: one run at a time, and a second start returns the running run's id. A reload or reconnect shows the steps so far over REST and then follows events. Tested in Task 6 (manager), Task 7 (route) and Task 9 (UI resync).

## Not in this phase

- **Storing the SSID as the network label (`networks.ssid_label`) and the replaced-router prompt.** Phase 6. Phase 2 only shows the SSID on the Dashboard, read live from `GET /api/wifi`.
- **A Playwright test for all of journey C.** It needs the first-run screen and phone-access settings (phase 6). Journey C's portal step is covered in the simulator by Task 6 and by an exe test in Task 8, which is what the §12 done-when asks for.
- **Diagnosis history in the database.** Runs are kept in memory (the last 8). The spec's data model has no diagnosis table.
- **Linking diagnosis to incidents** ("diagnosed" in the §6.9 timeline). Phase 3.
- **6 GHz channel width beyond the HE 6 GHz operation field, and 320 MHz (Wi-Fi 7).** These are reported as unknown width.

---

### Task 1: Platform seam for Wi-Fi, richer HTTP results, and simulator support

**Files:**
- Create: `crates/netlantern-platform/src/wifi.rs` (types plus pure helpers)
- Modify:
  - `crates/netlantern-platform/src/{platform.rs,netio.rs,lib.rs,sim/mod.rs,sim/scenario.rs,win/platform.rs}`
  - `crates/netlantern-platform/src/net/http.rs`: fill the new `location` field only; Task 2 does the rest.
  - The test stubs that implement the traits: `crates/netlantern-core/tests/{discover.rs,scan_sim.rs}` and `crates/netlantern-server/tests/api.rs`.
- Test:
  - `crates/netlantern-platform/src/wifi.rs` (unit tests)
  - `crates/netlantern-platform/tests/sim.rs` (extend)

**Interfaces:**
- Consumes: Phase 1's `Platform`, `NetIo`, `SimNetwork`, `MacAddr`, `InterfaceKind` and `PlatformError`.
- Produces, all re-exported from `netlantern_platform`:
  - `enum WifiBand { Ghz2_4, Ghz5, Ghz6 }`:
    - serde names `"2.4ghz"`, `"5ghz"`, `"6ghz"`;
    - `fn label(&self) -> &'static str` returns `"2.4 GHz"`, `"5 GHz"`, `"6 GHz"`;
    - derives `Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize`.
  - `struct WifiConnection { ssid: String, bssid: MacAddr, rssi_dbm: i32, channel: u32, band: WifiBand, width_mhz: Option<u32>, phy: String, rx_rate_mbps: Option<u32>, tx_rate_mbps: Option<u32>, security: String }`
  - `struct NearbyNetwork { ssid: String /* "" when hidden */, bssid: MacAddr, rssi_dbm: i32, channel: u32, band: WifiBand, width_mhz: Option<u32> }`
  - Both derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`. Field names are the JSON names.
  - `pub const LOCATION_PERMISSION: &str = "location";` is the `what` of `PermissionNeeded` for Wi-Fi.
  - New `Platform` methods, with no default bodies:
    - `async fn wifi_current(&self) -> Result<Option<WifiConnection>, PlatformError>`:
      - `Ok(None)` means there is no Wi-Fi adapter or it is not connected;
      - `Err(PermissionNeeded { what: "location" })` means Windows is hiding the details.
    - `async fn wifi_scan(&self) -> Result<Vec<NearbyNetwork>, PlatformError>`:
      - triggers a radio scan and returns what it found;
      - `Err(Unavailable)` when there is no adapter;
      - `Err(PermissionNeeded)` as above.
  - `HttpResponse` gains `location: Option<String>`, the `Location` header.
  - `PlatformError` gains `TlsCertificate { message: String }`, displayed as `certificate rejected: {message}`.
  - Pure helpers in `wifi.rs`. They are used by Task 3 and tested here:
    - `pub fn channel_from_mhz(mhz: u32) -> Option<(WifiBand, u32)>`:
      - 2412–2472 → 2.4 GHz, `(mhz − 2407) / 5`;
      - 2484 → channel 14;
      - 5160–5885 → 5 GHz, `(mhz − 5000) / 5`;
      - 5955–7115 → 6 GHz, `(mhz − 5950) / 5`;
      - anything else → `None`.
    - `pub fn rssi_from_quality(quality: u32) -> i32` returns `quality / 2 − 100`, with quality clamped to 0..=100.
    - `pub fn phy_label(phy_type: u32, band: WifiBand) -> &'static str`. It takes `DOT11_PHY_TYPE` numbers:
      - 4 → `"802.11a"`
      - 5 → `"802.11b"`
      - 6 → `"802.11g"`
      - 7 → `"Wi-Fi 4 (802.11n)"`
      - 8 → `"Wi-Fi 5 (802.11ac)"`
      - 10 → `"Wi-Fi 6 (802.11ax)"`, or `"Wi-Fi 6E (802.11ax)"` on 6 GHz
      - 11 → `"Wi-Fi 7 (802.11be)"`
      - else → `"Unknown"`
    - `pub fn security_label(auth_algorithm: u32) -> &'static str`. It takes `DOT11_AUTH_ALGORITHM` numbers:
      - 1 → `"Open"`
      - 2 → `"Shared key (WEP)"`
      - 3 → `"WPA-Enterprise"`
      - 4 → `"WPA-Personal"`
      - 6 → `"WPA2-Enterprise"`
      - 7 → `"WPA2-Personal"`
      - 8 or 11 → `"WPA3-Enterprise"`
      - 9 → `"WPA3-Personal"`
      - 10 → `"Enhanced Open"`
      - else → `"Unknown"`
    - `pub fn channel_width_mhz(ies: &[u8]) -> Option<u32>` reads 802.11 information elements, each a 1-byte id, a 1-byte length and the payload. It stops cleanly on a truncated element. The rules:
      - **HT Operation (id 61):** payload byte 1 bits 0–1 are the secondary channel offset; a value of 1 or 3 means 40 MHz, else 20 MHz.
      - **VHT Operation (id 192):** payload byte 0 is the channel width.
        - 0: use the HT answer.
        - 1: 80 MHz, unless payload byte 2 (CCFS1) is non-zero, in which case 160 MHz.
        - 2 or 3: 160 MHz.
      - **HE Operation (id 255, extension id 36):** only when the 6 GHz Operation Information Present bit is set (bit 17 of the 3-byte little-endian HE Operation Parameters).
        - Layout after the extension id: parameters (3 bytes), BSS colour (1), basic HE-MCS set (2), VHT Operation Information (3, only if bit 14 is set), Max Co-Hosted BSSID Indicator (1, only if bit 15 is set), then the 6 GHz Operation Information: primary channel (1) and control (1).
        - Control bits 0–1 mean 0 → 20, 1 → 40, 2 → 80, 3 → 160 MHz.
      - The widest answer found wins. No HT, VHT or HE element → `None`.

**Scenario additions.** All are optional, and existing scenario files keep loading unchanged:
```json
"wifi": {
  "permission": "granted",
  "current": { "ssid": "Lantern Home", "bssid": "50:C7:BF:3A:11:05", "rssi_dbm": -52, "channel": 36, "band": "5ghz", "width_mhz": 80,
               "phy": "Wi-Fi 6 (802.11ax)", "rx_rate_mbps": 866, "tx_rate_mbps": 780, "security": "WPA2-Personal" },
  "nearby": [ { "ssid": "Lantern Home", "bssid": "50:C7:BF:3A:11:04", "rssi_dbm": -48, "channel": 6, "band": "2.4ghz", "width_mhz": 20 } ]
},
"internet": [ { "ip": "1.1.1.1", "ping_ms": 14, "open_ports": [443] }, { "ip": "8.8.8.8", "ping_ms": 18 } ],
"dns_down": ["192.168.1.1"],
"http": { "https://www.microsoft.com/": { "status": 302, "location": "https://www.microsoft.com/en-us/" },
          "https://bad.example/": { "status": 0, "tls_error": "unknown issuer" } }
```
- `wifi.permission` is `"granted"` (default) or `"denied"`. `current` and `nearby` reuse `WifiConnection` and `NearbyNetwork` directly.
- `internet` hosts answer `ping` and `tcp_probe` like devices do. They are **not** in `arp_table`, `arp_resolve`, discovery or the sweep.
- `http` entries gain optional `location` and `tls_error`. When `tls_error` is set, `http_get` returns `Err(TlsCertificate { message })` and ignores `status`.

**Sim behaviour:**
- **`wifi_current`:**
  - no interface → `Err(NotConnected)`;
  - interface kind not `wifi` → `Ok(None)`;
  - `wifi` block absent → `Err(Unavailable { reason: "no Wi-Fi data in this scenario" })`;
  - `permission: "denied"` → `Err(PermissionNeeded { what: "location" })`;
  - otherwise `Ok(current)`.
- **`wifi_scan`:** sleeps 1 s (the paused clock makes this instant in tests). Then the same permission and absent-block rules apply, and it returns `nearby`. It works with an Ethernet interface too, if the scenario has a `wifi` block.
- **`dns_resolve`:**
  - no interface → `Err(NotConnected)`;
  - `server` in `dns_down` → sleeps 1 s, then `Err(Io { message: "DNS query to {server} timed out" })`;
  - otherwise, as today.
- **`ping` and `tcp_probe`** look in `devices`, then in `internet`.

- [ ] **Step 1: Write the failing tests**

```rust
// wifi.rs unit tests
#[test] fn frequencies_map_to_band_and_channel() {
    assert_eq!(channel_from_mhz(2437), Some((WifiBand::Ghz2_4, 6)));
    assert_eq!(channel_from_mhz(2484), Some((WifiBand::Ghz2_4, 14)));
    assert_eq!(channel_from_mhz(5180), Some((WifiBand::Ghz5, 36)));
    assert_eq!(channel_from_mhz(5975), Some((WifiBand::Ghz6, 5)));
    assert_eq!(channel_from_mhz(3000), None);
}
#[test] fn widths_from_information_elements() {
    let ht40 = [61, 22, 6, 0x01, /* 20 more bytes */ 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0];
    assert_eq!(channel_width_mhz(&ht40), Some(40));
    let vht80 = [&ht40[..], &[192, 5, 1, 42, 0, 0, 0][..]].concat();
    assert_eq!(channel_width_mhz(&vht80), Some(80));
    let vht160 = [&ht40[..], &[192, 5, 1, 42, 50, 0, 0][..]].concat();
    assert_eq!(channel_width_mhz(&vht160), Some(160));
    assert_eq!(channel_width_mhz(&[]), None);
    assert_eq!(channel_width_mhz(&[61, 200, 1]), None); // truncated: no panic
}
#[test] fn labels() {
    assert_eq!(phy_label(10, WifiBand::Ghz6), "Wi-Fi 6E (802.11ax)");
    assert_eq!(security_label(7), "WPA2-Personal");
    assert_eq!(rssi_from_quality(100), -50);
    assert_eq!(serde_json::to_string(&WifiBand::Ghz2_4).unwrap(), "\"2.4ghz\"");
}
// tests/sim.rs: a scenario JSON written inline in each test
#[tokio::test(start_paused = true)] async fn sim_wifi_permission_denied_is_permission_needed() { /* permission "denied" → both calls Err(PermissionNeeded{what:"location"}) */ }
#[tokio::test(start_paused = true)] async fn sim_wifi_on_ethernet_has_no_current_connection() { /* kind ethernet → wifi_current Ok(None) */ }
#[tokio::test(start_paused = true)] async fn sim_internet_hosts_answer_ping_but_stay_out_of_arp() { /* ping 1.1.1.1 → Some(14 ms); arp_table has no 1.1.1.1 */ }
#[tokio::test(start_paused = true)] async fn sim_dns_down_server_times_out_but_others_answer() { /* dns_down [192.168.1.1]: via .1 → Err(Io), via 1.1.1.1 → Ok */ }
#[tokio::test(start_paused = true)] async fn sim_http_location_and_tls_error() { /* 302 + location; tls_error → Err(TlsCertificate{message:"unknown issuer"}) */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-platform`. Expected: FAIL to compile, because `wifi` and the new fields don't exist yet.
- [ ] **Step 3: Implement** the types, helpers, trait methods and sim behaviour above.
  - `WindowsPlatform` gets temporary bodies that return `Err(Unavailable { reason: "Wi-Fi support arrives in Task 3" })`.
  - Every test stub that implements `Platform` returns `Err(Unavailable)` from both new methods.
  - Every `HttpResponse { .. }` literal gains `location: None`.
  - `net/http.rs` fills `location` from the response's `Location` header.
- [ ] **Step 4:** Run `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`. Expected: PASS.
- [ ] **Step 5: Commit:** `Add Wi-Fi types and Platform methods, HTTP redirect locations and certificate errors, and simulator support for Wi-Fi, outside hosts and failing DNS servers.`

---

### Task 2: Real HTTP: certificate errors and same-host redirects

**Files:**
- Modify: `crates/netlantern-platform/src/net/http.rs`, `crates/netlantern-platform/Cargo.toml`, `Cargo.toml` (add `rustls = { version = "0.23", default-features = false }` to `[workspace.dependencies]`)
- Test: `crates/netlantern-platform/src/net/http.rs` (unit), `crates/netlantern-platform/tests/net.rs` (extend)

**Interfaces:**
- Consumes: Task 1's `HttpResponse.location` and `PlatformError::TlsCertificate`.
- Produces:
  - `RealNetIo::http_get` returns `Err(TlsCertificate { message })` when the server's certificate is rejected. The message is the rustls error's `Display`, cut to 100 characters. Every other failure stays `Io`.
  - `fn is_certificate_error(err: &(dyn std::error::Error + 'static)) -> Option<String>` (private) walks `source()`. At each level it tries to downcast to `rustls::Error`, and to `std::io::Error`, whose `get_ref()` it then downcasts to `rustls::Error`. It returns the message for `rustls::Error::InvalidCertificate(_)` only.
  - **Carry-over from Phase 1:** with `follow_redirects = true`, redirects are followed (up to 5) **only while the target host and port equal the original URL's**. A redirect to another host is returned as-is, as the 3xx with its `location`. This uses `reqwest::redirect::Policy::custom`. It stops a LAN device's UPnP description URL from bouncing Net Lantern to the internet. Nothing in Phase 2 passes `true`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn certificate_errors_are_found_inside_io_errors() {
    let tls = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
    let io = std::io::Error::new(std::io::ErrorKind::InvalidData, tls);
    assert!(is_certificate_error(&io).unwrap().contains("UnknownIssuer"));
    let other = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
    assert_eq!(is_certificate_error(&other), None);
}
// tests/net.rs: a one-shot HTTP responder on 127.0.0.1 (tokio TcpListener writing a raw response)
#[tokio::test] async fn location_header_is_returned_without_following() { /* 302 Location: /next, follow=false → status 302, location Some("/next") */ }
#[tokio::test] async fn redirects_to_another_host_are_not_followed() { /* 302 Location: http://example.invalid/ with follow=true → status 302 */ }
#[tokio::test] async fn same_host_redirects_are_followed() { /* 302 → /final on the same listener → status 200, body "final" */ }
#[tokio::test] #[ignore = "needs the internet"] async fn self_signed_site_is_a_certificate_error() { /* https://self-signed.badssl.com/ → Err(TlsCertificate{..}) */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-platform`. Expected: FAIL.
- [ ] **Step 3:** Implement. Map `reqwest` send errors through `is_certificate_error` before falling back to `io_error`.
- [ ] **Step 4:** Run `cargo test -p netlantern-platform`, then `cargo test -p netlantern-platform -- --ignored self_signed`, which needs the internet. Expected: both PASS.
- [ ] **Step 5: Commit:** `Report rejected TLS certificates as their own error and only follow redirects that stay on the same host.`

---

### Task 3: Windows Wi-Fi with the WLAN API

**Files:**
- Create: `crates/netlantern-platform/src/win/wlan.rs`
- Modify: `crates/netlantern-platform/src/win/{mod.rs,platform.rs}`, `crates/netlantern-platform/Cargo.toml` (feature `Win32_NetworkManagement_WiFi`)
- Test: `crates/netlantern-platform/tests/windows.rs` (extend; the real-hardware tests are `#[ignore]`)

**Interfaces:**
- Consumes: Task 1's types and helpers (`channel_from_mhz`, `channel_width_mhz`, `phy_label`, `security_label`, `rssi_from_quality`).
- Produces:
  - `fn wifi_current_blocking() -> Result<Option<WifiConnection>, PlatformError>`
  - `fn wifi_scan_start_blocking() -> Result<(), PlatformError>`
  - `fn wifi_bss_list_blocking() -> Result<Vec<NearbyNetwork>, PlatformError>`
  - `WindowsPlatform::wifi_current` and `wifi_scan` wrap these in `spawn_blocking`. `wifi_scan` starts a scan, sleeps **4 s** with `tokio::time::sleep`, then reads the BSS list.

**How it works:**
- **Handles:** `WlanOpenHandle(2, …)`, then `WlanEnumInterfaces`. Use the first interface whose state is `wlan_interface_state_connected` for `wifi_current`, and the first interface of any state for scans.
  - No interfaces: `wifi_current` → `Ok(None)`, scans → `Err(Unavailable { reason: "no Wi-Fi adapter" })`.
  - The handle is closed and every WLAN allocation is freed by drop guards (`WlanCloseHandle`, `WlanFreeMemory`). Each `unsafe` block gets a `// SAFETY:` comment, as in Phase 1's `win/` code.
- **`wifi_current`:**
  - `WlanQueryInterface(wlan_intf_opcode_current_connection)` gives the SSID, the BSSID, `wlanSignalQuality`, `ulRxRate` and `ulTxRate` (kbps, divided by 1000), `dot11PhyType` and `dot11AuthAlgorithm`.
  - Then `wlan_intf_opcode_rssi` gives the RSSI in dBm. When that fails, use `rssi_from_quality`.
  - Then `wlan_intf_opcode_channel_number` gives the channel. The band comes from the BSS entry's frequency when it is found, else from the channel number (1–14 is 2.4 GHz, otherwise 5 GHz).
  - Width comes from the BSS list entry whose BSSID matches, through `channel_width_mhz` over its IEs (`ulIeOffset` and `ulIeSize` are relative to the entry). If the BSS list is denied or missing, width is `None`.
- **`wifi_bss_list`:** `WlanGetNetworkBssList(…, dot11_BSS_type_any, false, …)`. Each entry gives `dot11Ssid` (an empty SSID means hidden), `dot11Bssid`, `lRssi`, `ulChCenterFrequency` (kHz, divided by 1000, then `channel_from_mhz`) and the IE width. Entries whose frequency doesn't map to a channel are skipped.
- **Error mapping**, applied to every WLAN call:
  - `ERROR_ACCESS_DENIED` (5) → `PermissionNeeded { what: LOCATION_PERMISSION }`. This is Windows' location rule.
  - `ERROR_INVALID_STATE` (5023) from `current_connection` → `Ok(None)`.
  - `ERROR_SERVICE_NOT_ACTIVE` (1062) → `Unavailable { reason: "the WLAN AutoConfig service is not running" }`.
  - Any other code → `Io { message: "WLAN API error {code}" }`.

- [ ] **Step 1: Write the tests**

```rust
#[tokio::test] #[ignore = "needs a Wi-Fi adapter"] async fn wifi_current_reads_the_connection_or_asks_for_location() {
    match WindowsPlatform::new().wifi_current().await {
        Ok(Some(c)) => { assert!((-100..=0).contains(&c.rssi_dbm)); assert!(c.channel > 0); println!("{c:?}"); }
        Ok(None) => println!("not connected to Wi-Fi"),
        Err(PlatformError::PermissionNeeded { what }) => assert_eq!(what, "location"),
        Err(e) => panic!("unexpected: {e}"),
    }
}
#[tokio::test] #[ignore = "needs a Wi-Fi adapter"] async fn wifi_scan_lists_networks_or_asks_for_location() { /* Ok(list) with every channel > 0, or PermissionNeeded */ }
#[test] fn wlan_error_codes_map_to_platform_errors() { /* map_wlan_error(5) → PermissionNeeded{what:"location"}; 1062 → Unavailable; 87 → Io */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-platform --test windows`. Expected: FAIL to compile.
- [ ] **Step 3:** Implement `win/wlan.rs` and wire it into `WindowsPlatform`, replacing Task 1's temporary bodies.
- [ ] **Step 4:** Run `cargo test -p netlantern-platform`. Expected: PASS. Then run `cargo test -p netlantern-platform --test windows -- --ignored wifi --nocapture` on the author's laptop and **record the output in the task report**: either the connection, or `PermissionNeeded` with location services on or off. This is the first data point for the §16 location-rule risk.
- [ ] **Step 5: Commit:** `Read the current Wi-Fi connection and nearby networks on Windows through the WLAN API, reporting when location access is needed.`

---

### Task 4: Channel congestion and the Wi-Fi service

**Files:**
- Create: `crates/netlantern-core/src/wifi.rs`
- Modify: `crates/netlantern-core/src/lib.rs`
- Test: `crates/netlantern-core/src/wifi.rs` (unit), `crates/netlantern-core/tests/wifi_sim.rs`

**Interfaces:**
- Consumes: Task 1's `Platform::wifi_current` and `wifi_scan`, `WifiConnection`, `NearbyNetwork`, `WifiBand` and `InterfaceKind`.
- Produces, re-exported from `netlantern_core`:
  - `struct CongestionConfig { crowded_2_4: u32 /* 5 */, crowded_other: u32 /* 3 */ }` with `Default`.
  - `struct ChannelLoad { band: WifiBand, channel: u32, networks: u32 }`: distinct BSSIDs on exactly this channel.
  - `struct CurrentLoad { band: WifiBand, channel: u32, overlapping: u32, crowded: bool, own_5ghz_visible: bool, suggested_channel: Option<u32> }`
  - `struct Congestion { channels: Vec<ChannelLoad> /* sorted by band then channel */, current: Option<CurrentLoad>, advice: String }`
  - `pub fn analyze_congestion(current: Option<&WifiConnection>, nearby: &[NearbyNetwork], cfg: &CongestionConfig) -> Congestion`
  - `struct NearbyScan { scanned_at: u64, networks: Vec<NearbyNetwork> /* strongest first */, congestion: Congestion }`
  - `struct WifiStatus { interface_kind: Option<InterfaceKind> /* None = not connected */, connection: Option<WifiConnection> }`
  - `struct WifiService`:
    - `pub fn new(platform: Arc<dyn Platform>, cfg: CongestionConfig) -> Self`
    - `pub async fn status(&self) -> Result<WifiStatus, CoreError>`:
      - `PermissionNeeded` passes through as `CoreError::Platform(..)`;
      - `NotConnected` from `active_interface` gives `interface_kind: None` and still asks `wifi_current`;
      - `NotConnected` from `wifi_current` gives `connection: None`.
    - `pub async fn nearby(&self, max_age: Duration) -> Result<NearbyScan, CoreError>`: returns the cached scan if it is younger than `max_age`. Otherwise it runs `wifi_scan`, under a `tokio::sync::Mutex` so only one radio scan runs at a time. A caller that waited on the mutex re-checks the cache first. It also reads `wifi_current` (errors there count as no current connection) and caches the result.
  - All of these derive `Clone, Debug, PartialEq, Serialize`.

**Congestion rules (spec §6.7, decided here):**
- **Overlap.** Two 2.4 GHz networks overlap when their channels differ by less than 5. On 5 and 6 GHz, only the same channel counts. Channel width is ignored, which is a known simplification recorded in §6.7.
- **`overlapping`** counts distinct BSSIDs, other than the connected BSSID, that overlap the current channel. Hidden networks count.
- **`crowded`** is `overlapping ≥ crowded_2_4` on 2.4 GHz, else `overlapping ≥ crowded_other`.
- **`own_5ghz_visible`**: a nearby network with the current SSID on 5 or 6 GHz while connected on 2.4 GHz.
- **`suggested_channel`**, only when crowded:
  - on 2.4 GHz, the channel among 1, 6 and 11 with the fewest overlapping networks;
  - on 5 GHz, the channel among 36, 40, 44, 48, 149, 153, 157 and 161 with the fewest networks on it;
  - ties go to the lower channel; `None` on 6 GHz.
- **`advice`** is exact copy:
  - not on Wi-Fi: `Not on Wi-Fi, so channel congestion doesn't affect this connection.`
  - not crowded: `Your channel isn't crowded ({n} other networks share it).`
  - crowded, own 5 GHz visible: `Channel {ch} is shared with {n} other networks. Your router also broadcasts on 5 GHz — join that for less interference.`
  - crowded on 2.4 GHz: `Channel {ch} is shared with {n} other networks. In your router's settings, try channel {s} (the least crowded of 1, 6 and 11).`
  - crowded on 5 GHz: `Channel {ch} is shared with {n} other networks. In your router's settings, try channel {s}.`
  - crowded on 6 GHz: `Channel {ch} is shared with {n} other networks.`

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn nine_overlapping_networks_on_channel_6_is_crowded_with_own_5ghz_advice() {
    // current: 2.4 GHz ch 6 SSID "Lantern Home"; nearby: 9 others on ch 4..8, plus "Lantern Home" on 5 GHz ch 36
    let c = analyze_congestion(Some(&current), &nearby, &CongestionConfig::default());
    let cur = c.current.unwrap();
    assert_eq!((cur.overlapping, cur.crowded, cur.own_5ghz_visible), (9, true, true));
    assert!(c.advice.starts_with("Channel 6 is shared with 9 other networks. Your router also broadcasts on 5 GHz"));
}
#[test] fn channel_11_does_not_overlap_channel_6() { /* one net on ch 11 → overlapping 0 */ }
#[test] fn suggests_least_crowded_of_1_6_11_ties_low() { /* crowded ch 6, ch 1 and 11 empty → Some(1) */ }
#[test] fn five_ghz_counts_same_channel_only() { /* 3 on ch 36 → crowded; nets on ch 40 don't count */ }
#[test] fn connected_bssid_is_not_counted() { /* nearby includes the connected BSSID → overlapping excludes it */ }
#[test] fn no_current_connection_gives_not_on_wifi_advice() { /* current None → current None, advice "Not on Wi-Fi, …" */ }
// tests/wifi_sim.rs (paused clock, SimNetwork wrapped in a counting Platform)
#[tokio::test(start_paused = true)] async fn nearby_reuses_a_fresh_scan_and_rescans_a_stale_one() { /* nearby(10 s) twice → 1 wifi_scan; advance 11 s → 2 */ }
#[tokio::test(start_paused = true)] async fn concurrent_nearby_calls_share_one_radio_scan() { /* join!(nearby, nearby) → 1 wifi_scan */ }
#[tokio::test(start_paused = true)] async fn status_passes_location_permission_through() { /* denied → Err(CoreError::Platform(PermissionNeeded{what:"location"})) */ }
#[tokio::test(start_paused = true)] async fn status_when_not_connected_has_no_interface_kind() { /* offline.json → Ok(WifiStatus{interface_kind:None, connection:None}) */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core wifi`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p netlantern-core`. Expected: PASS.
- [ ] **Step 5: Commit:** `Analyse channel congestion from nearby networks and add a Wi-Fi service that caches radio scans and runs one at a time.`

---

### Task 5: Diagnosis model, judges and verdict (pure)

**Files:**
- Create: `crates/netlantern-core/src/diagnose/{mod.rs,model.rs,judge.rs,copy.rs}`
- Modify: `crates/netlantern-core/src/lib.rs`
- Test: `crates/netlantern-core/src/diagnose/judge.rs` (unit)

**Interfaces:**
- Consumes: Task 1's `WifiConnection`, `HttpResponse`, `PlatformError` and `InterfaceKind`, and Task 4's `CurrentLoad`.
- Produces, re-exported from `netlantern_core`:
  - **Wire types** in `model.rs`, all deriving `Clone, Debug, PartialEq, Serialize, Deserialize`, `snake_case`:
    - `enum Layer { Link, Router, Internet, Dns, Website }`
    - `enum LayerStatus { Pass, Warn, Fail, Skipped }`
    - `enum Finding`, one of:
      - `EthernetOk, LinkOk, SignalHidden, NotConnected, WeakSignal, CrowdedChannel, WeakAndCrowded`
      - `RouterOk, RouterIgnoresPing, RouterUnreachable, RouterLossy, RouterSlow`
      - `InternetOk, CaptivePortal, InternetDown, InternetLossy, InternetSlow`
      - `DnsOk, NoDnsServer, DnsServerProblem, DnsUnreachable, DnsPartial`
      - `WebsiteOk, TlsIntercepted, WebsiteUnreachable`
      - `Healthy, Skipped`
    - `struct LayerResult { layer: Layer, title: String, status: LayerStatus, finding: Finding, evidence: Vec<String>, explanation: String, fix: Option<String> }`
    - `struct Verdict { finding: Finding, status: LayerStatus, layer: Option<Layer>, headline: String, fix: Option<String> }`
    - `struct DiagnosisId(pub u64)` (`#[serde(transparent)]`)
    - `enum RunState { Running, Done, Failed }`
    - `struct DiagnosisRun { id: DiagnosisId, state: RunState, started_at: u64, finished_at: Option<u64>, steps: Vec<LayerResult>, verdict: Option<Verdict>, error: Option<String> }`
  - **Probe types** in `model.rs`, `Clone, Debug, PartialEq`, no serde:
    - `enum WifiReading { NotWifi, Connected(WifiConnection), Disconnected, PermissionNeeded, Unavailable(String) }`
    - `struct LinkProbe { interface: Option<InterfaceKind> /* None = not connected */, wifi: WifiReading, congestion: Option<CurrentLoad>, congestion_advice: Option<String> }`
    - `struct PingStats { target: Ipv4Addr, sent: u32, rtts_ms: Vec<u32> }` with `replies() -> u32`, `avg_ms() -> Option<u32>` and `lost() -> u32`
    - `struct RouterProbe { pings: PingStats, answers_arp: Option<bool> /* asked only when no ping replied */ }`
    - `enum PortalCheck { Expected, Redirect { status: u16, location: Option<String> }, UnexpectedAnswer { status: u16 }, Failed(String) }`
    - `struct TcpCheck { target: Ipv4Addr, port: u16, open: bool }`
    - `struct InternetProbe { pings: Vec<PingStats>, tcp: TcpCheck, portal: PortalCheck }`
    - `struct DnsLookup { name: String, server: Ipv4Addr, result: Result<Vec<Ipv4Addr>, String> }`. An `Ok(vec![])` counts as failed.
    - `struct DnsProbe { system_servers: Vec<Ipv4Addr>, direct_server: Ipv4Addr, system: Vec<DnsLookup> /* one per name */, direct: Vec<DnsLookup> }`
    - `enum WebsiteOutcome { Status(u16), Certificate(String), Failed(String) }`
    - `struct WebsiteProbe { url: String, outcome: WebsiteOutcome }`
  - `struct DiagnoseConfig`, with the Global Constraints values in `Default`:
    - `wifi_warn_dbm: i32`
    - `gateway_pings: u32`, `gateway_ping_timeout: Duration`, `router_warn_rtt_ms: u32`
    - `arp_timeout: Duration`
    - `internet_targets: Vec<Ipv4Addr>`, `internet_pings: u32`, `internet_ping_timeout: Duration`, `internet_warn_rtt_ms: u32`
    - `tcp_target: (Ipv4Addr, u16)`, `tcp_timeout: Duration`
    - `portal_url: String`, `portal_expected_body: String`
    - `dns_names: Vec<String>`, `direct_dns: Ipv4Addr`
    - `website_url: String`
    - `layer_timeout: Duration`, `nearby_max_age: Duration`
  - **Pure functions** in `judge.rs`:
    - `pub fn classify_portal(res: &Result<HttpResponse, PlatformError>, expected_body: &str) -> PortalCheck`:
      - 200 with the trimmed body equal to `expected_body` → `Expected`;
      - 3xx or 511 → `Redirect`;
      - any other HTTP answer → `UnexpectedAnswer`;
      - `Err` → `Failed(message)`.
    - `pub fn judge_link(p: &LinkProbe, cfg: &DiagnoseConfig) -> LayerResult`
    - `pub fn judge_router(p: &RouterProbe, cfg: &DiagnoseConfig) -> LayerResult`
    - `pub fn judge_internet(p: &InternetProbe, cfg: &DiagnoseConfig) -> LayerResult`
    - `pub fn judge_dns(p: &DnsProbe) -> LayerResult`
    - `pub fn judge_website(p: &WebsiteProbe) -> LayerResult`
    - `pub fn skipped(layer: Layer) -> LayerResult`
    - `pub fn verdict(steps: &[LayerResult]) -> Verdict`
  - **`copy.rs`:** `pub fn explanation(f: Finding) -> &'static str`, `pub fn headline(f: Finding) -> &'static str` and `pub fn fix(f: Finding) -> Option<&'static str>`, filled from the copy table below.

**Decision rules (spec §6.6, with the builder's calls marked "decided"):**
- **Link.**
  - Title: `Wi-Fi link` when the interface kind is Wi-Fi, `Cable link` for Ethernet, else `Network link`.
  - `interface: None` → Fail `NotConnected`.
  - Ethernet or Other → Pass `EthernetOk`.
  - Wi-Fi with `PermissionNeeded`, `Disconnected` or `Unavailable` → Pass `SignalHidden`. **Decided:** the interface being up proves the link, and the reading is only extra detail.
  - Wi-Fi `Connected`:
    - `weak` means `rssi_dbm < wifi_warn_dbm`, and `crowded` means `congestion.crowded`;
    - both → Warn `WeakAndCrowded`, weak only → Warn `WeakSignal`, crowded only → Warn `CrowdedChannel`, else Pass `LinkOk`.
  - Fix overrides:
    - `CrowdedChannel` uses `congestion_advice` as its fix.
    - `WeakAndCrowded` uses its table fix when `own_5ghz_visible`, else `Move closer to the router, and change the router's channel (see Wi-Fi quality below).`
- **Router.** Title `Router`.
  - 0 replies, with `answers_arp == Some(true)` → Warn `RouterIgnoresPing`. **Decided:** spec §15 leaves this fallback to the builder, and diagnosis continues.
  - 0 replies otherwise → Fail `RouterUnreachable`.
  - Any lost reply → Warn `RouterLossy`. With 5 pings, one loss is 20%, which is over 5%.
  - Average RTT > `router_warn_rtt_ms` → Warn `RouterSlow`.
  - Otherwise Pass `RouterOk`.
- **Internet.** Title `Internet`. The rules apply in this order:
  1. The portal is `Redirect` or `UnexpectedAnswer` → Fail `CaptivePortal`. This is checked first, because portals block ping and outside DNS (spec §6.6).
  2. `reachable` means any ping replied, or `tcp.open`, or the portal is `Expected`. Not reachable → Fail `InternetDown`.
  3. Some targets replied and some pings were lost → Warn `InternetLossy`. When **no** ping replied but TCP or the portal worked, ICMP is blocked: Pass `InternetOk`, with the evidence line `Pings are blocked on this network; TCP and web checks worked.`
  4. The lowest average RTT among the targets that replied is over `internet_warn_rtt_ms` → Warn `InternetSlow`.
  5. Otherwise Pass `InternetOk`.
- **DNS.** Title `DNS`.
  - No system servers → Fail `NoDnsServer`.
  - Every system lookup succeeded → Pass `DnsOk`.
  - Some succeeded → Warn `DnsPartial`.
  - None succeeded, while any direct lookup succeeded → Fail `DnsServerProblem`.
  - None succeeded anywhere → Fail `DnsUnreachable`.
- **Website.** Title `Website`.
  - `Status(_)`, any HTTP status → Pass `WebsiteOk`. **Decided:** any answer proves the path, and redirects aren't followed.
  - `Certificate` → Fail `TlsIntercepted`.
  - `Failed` → Fail `WebsiteUnreachable`.
- **`skipped(layer)`:** status Skipped, finding `Skipped`, explanation `Not checked because an earlier layer failed.`, no evidence and no fix. The title comes from the layer (the link title is `Network link`).
- **`verdict`:**
  - the first Fail step → its finding, headline and fix;
  - else the first Warn step → the same, using the step's own `fix`, so the link overrides carry through;
  - else `Healthy` with status Pass and `layer: None`.

**Evidence lines (exact formats):**
- **Link:**
  - `Wi-Fi "{ssid}", {band label} channel {ch}, signal {rssi} dBm`
  - `Channel {ch} is shared with {n} other networks`, only when congestion is known
  - `Connected by cable`
  - `Signal details hidden: location access is off`
  - `Not connected to any network`
- **Router and internet pings:**
  - `{ip}: {r} of {n} replies, average {avg} ms`
  - `{ip}: no replies`
- **Router ARP:**
  - `{gw} answered at the network level (ARP)`
  - `{gw} didn't answer ARP either`
- **TCP:**
  - `TCP {port} to {ip}: open`
  - `TCP {port} to {ip}: no answer`
- **Portal:**
  - `Connectivity check: answered as expected`
  - `Connectivity check: redirected to {location}`, or `(no location)` when there is none
  - `Connectivity check: unexpected answer (HTTP {status})`
  - `Connectivity check: failed ({error})`
- **DNS:**
  - `{name} via {server}: {first address}`
  - `{name} via {server}: failed ({error})`
  - System lookups come first, then direct ones.
- **Website:**
  - `{url}: HTTP {status}`
  - `{url}: certificate rejected ({message})`
  - `{url}: failed ({message})`
- Text from the network (`location`, error messages) is cut to 100 characters.

**Copy table.** This is exact copy: the pass explanation is shown under passed layers, and the headline is used when that finding is the verdict.

| Finding | Headline | Explanation | Fix |
|---|---|---|---|
| EthernetOk | Your cable connection is fine. | Your laptop is connected by cable. | — |
| LinkOk | Your Wi-Fi connection is fine. | Your laptop is connected and the signal is fine. | — |
| SignalHidden | Your Wi-Fi connection is up. | Connected. Windows is hiding the signal details until location access is on. | Turn on location access to see signal and channel details (see Wi-Fi quality below). |
| NotConnected | You're not connected to a network. | Your laptop isn't connected to Wi-Fi or a cable, so nothing else can be checked. | Connect to your Wi-Fi network, or check that Wi-Fi is turned on. |
| WeakSignal | Your Wi-Fi signal is weak. | The signal is below −70 dBm, which causes slow speeds and dropouts. | Move closer to the router, or remove obstacles between you and it. |
| CrowdedChannel | Your Wi-Fi channel is crowded. | Many nearby networks share your channel, so they take turns and slow each other down. | (the congestion advice) |
| WeakAndCrowded | Your Wi-Fi signal is weak and the channel is crowded. | A weak signal and a busy channel together cause freezes and slow speeds. | Move closer to the router, or join the 5 GHz network shown in the nearby list. |
| RouterOk | Your router is fine. | Your router answers quickly and reliably. | — |
| RouterIgnoresPing | Your router doesn't answer ping, but it is reachable. | Some routers ignore ping. This one answered at the network level, so the checks continue. | — |
| RouterUnreachable | Your laptop can't reach the router. | The router didn't answer pings or an ARP request. It may be off or frozen, or the Wi-Fi link isn't really working. | Restart the router (unplug it for 30 seconds), then reconnect. |
| RouterLossy | Some messages to your router are being lost. | Lost messages between your laptop and router usually mean Wi-Fi interference or a weak signal. | Move closer to the router, or restart it. |
| RouterSlow | Your router is slow to answer. | Replies took over 50 ms on average; on a home network they normally take a few milliseconds. | Restart the router. If it stays slow, something on the network may be using all the bandwidth. |
| InternetOk | Your internet connection is fine. | Your router reaches the internet. | — |
| CaptivePortal | Sign in to this network. | This network sends web traffic to a sign-in page first, so the internet is blocked until you sign in or accept its terms. | Open any website in your browser to get the sign-in page. |
| InternetDown | Your internet connection is down. | Your router works, but nothing on the internet answered: not 1.1.1.1, not 8.8.8.8, and not a web request. | Restart the router and modem. If that doesn't help, check your provider's status page or call them. |
| InternetLossy | Some messages to the internet are being lost. | Your router answers reliably, but some messages beyond it go unanswered, which points to the modem or your provider. | Restart the modem. If it keeps happening, tell your provider. |
| InternetSlow | Your internet connection is slow to respond. | Replies from the internet took over 100 ms, which makes calls and games lag. | Pause large downloads or streams on the network. If it stays slow, tell your provider. |
| DnsOk | Name lookups are fine. | Names like www.microsoft.com turn into addresses as they should. | — |
| NoDnsServer | Your laptop has no DNS server. | Without a DNS server, names like www.microsoft.com can't be turned into addresses. | Reconnect to the network. If it keeps happening, set DNS to 1.1.1.1 in your network settings. |
| DnsServerProblem | Your DNS server isn't working. | Your network's DNS server didn't answer, but 1.1.1.1 did, so the internet works and only name lookups fail. | Restart the router. If that doesn't help, set DNS to 1.1.1.1 in your network or router settings. |
| DnsUnreachable | Name lookups are failing everywhere. | Neither your network's DNS server nor 1.1.1.1 answered, although the internet answered. Something may be blocking DNS. | Restart the router. On a work or school network, ask its IT team. |
| DnsPartial | Some name lookups are failing. | Your DNS server answered some lookups but not others. | Restart the router. If it keeps happening, set DNS to 1.1.1.1. |
| WebsiteOk | Websites load. | A secure website answered. | — |
| TlsIntercepted | Something is intercepting secure connections. | A secure website's certificate wasn't trusted. A network filter, antivirus or hostile network may be reading encrypted traffic. | Don't enter passwords on this network. On a work network, ask IT whether they inspect traffic; otherwise check your antivirus' web protection. |
| WebsiteUnreachable | Websites don't load. | The lower layers work, but a secure website didn't load. | Turn off any VPN or proxy and try again. If it keeps happening, restart the router. |
| Healthy | Everything looks fine. | — | — |
| Skipped | — | Not checked because an earlier layer failed. | — |

- [ ] **Step 1: Write the failing tests.** There is one test per rule and Review Focus line. A few as code:

```rust
#[test] fn gateway_that_ignores_ping_but_answers_arp_warns_and_continues() {
    let r = judge_router(&RouterProbe { pings: stats("192.168.1.1", 5, &[]), answers_arp: Some(true) }, &cfg());
    assert_eq!((r.status, r.finding), (LayerStatus::Warn, Finding::RouterIgnoresPing));
}
#[test] fn silent_gateway_without_arp_is_unreachable() { /* answers_arp Some(false) → Fail RouterUnreachable */ }
#[test] fn one_lost_gateway_ping_warns_lossy() { /* 4 of 5 → Warn RouterLossy; evidence "192.168.1.1: 4 of 5 replies, average 3 ms" */ }
#[test] fn blocked_icmp_with_working_tcp_passes() {
    let p = InternetProbe { pings: vec![stats("1.1.1.1", 3, &[]), stats("8.8.8.8", 3, &[])], tcp: tcp(true), portal: PortalCheck::Expected };
    let r = judge_internet(&p, &cfg());
    assert_eq!((r.status, r.finding), (LayerStatus::Pass, Finding::InternetOk));
    assert!(r.evidence.contains(&"Pings are blocked on this network; TCP and web checks worked.".to_string()));
}
#[test] fn portal_redirect_beats_everything_failing() { /* pings none, tcp closed, Redirect{302, Some("http://portal/")} → Fail CaptivePortal; evidence "Connectivity check: redirected to http://portal/" */ }
#[test] fn portal_failure_from_broken_dns_is_not_a_portal() { /* pings ok, tcp open, Failed("dns error") → Pass InternetOk */ }
#[test] fn classify_portal_cases() { /* 200 "Microsoft Connect Test\n" → Expected; 302 → Redirect; 511 → Redirect; 200 "<html>login" → UnexpectedAnswer; Err → Failed */ }
#[test] fn system_dns_down_but_direct_works_is_server_problem() { /* → Fail DnsServerProblem */ }
#[test] fn both_dns_paths_down_is_unreachable() { /* → Fail DnsUnreachable */ }
#[test] fn empty_answer_counts_as_failed_lookup() { /* Ok(vec![]) → not a success */ }
#[test] fn certificate_error_is_tls_intercepted() { /* → Fail TlsIntercepted */ }
#[test] fn redirect_from_website_passes() { /* Status(302) → Pass WebsiteOk */ }
#[test] fn weak_and_crowded_with_own_5ghz_gives_journey_a_fix() {
    let v = verdict(&[judge_link(&weak_crowded_probe(true), &cfg()), pass(Layer::Router), pass(Layer::Internet), pass(Layer::Dns), pass(Layer::Website)]);
    assert_eq!(v.headline, "Your Wi-Fi signal is weak and the channel is crowded.");
    assert_eq!(v.fix.as_deref(), Some("Move closer to the router, or join the 5 GHz network shown in the nearby list."));
}
#[test] fn location_permission_off_still_passes_the_link() { /* WifiReading::PermissionNeeded → Pass SignalHidden */ }
#[test] fn verdict_prefers_first_fail_then_first_warn_then_healthy() { /* ... */ }
#[test] fn every_finding_has_copy() { /* iterate all Finding values: explanation non-empty; headline non-empty except Skipped */ }
#[test] fn wire_shapes() { /* serde_json of LayerResult → keys layer,title,status,finding,evidence,explanation,fix; finding "weak_and_crowded" */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core diagnose`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p netlantern-core`. Expected: PASS.
- [ ] **Step 5: Commit:** `Add the diagnosis model with pure judges for each layer, the verdict rules and their plain-language copy.`

---

### Task 6: Diagnosis runner, events and simulated scenarios

**Files:**
- Create:
  - `crates/netlantern-core/src/diagnose/{probe.rs,manager.rs}`
  - `scenarios/{weak-wifi.json,dns-failure.json,provider-down.json,cafe.json,tls-intercept.json,router-down.json,router-silent.json,location-denied.json}`
- Modify:
  - `crates/netlantern-core/src/{events.rs,lib.rs}`
  - `scenarios/home.json` (add `wifi`, `internet`, DNS names and the website entry)
- Test:
  - `crates/netlantern-core/tests/diagnose_sim.rs`
  - `crates/netlantern-platform/tests/sim.rs`: the existing scenario-loading test also loads every new file.

**Interfaces:**
- Consumes: Task 4's `WifiService` and `analyze_congestion`, and Task 5's judges, `verdict`, `skipped` and types.
- Produces:
  - `EventKind` gains two variants, serialized as `"diagnose.step"` and `"diagnose.done"`:
    - `DiagnoseStep { run_id: DiagnosisId, step: LayerResult }`
    - `DiagnoseDone { run_id: DiagnosisId, state: RunState, verdict: Option<Verdict>, error: Option<String> }`
  - `enum DiagnoseStart { Started(DiagnosisId), AlreadyRunning(DiagnosisId) }`
  - `struct DiagnoseManager`:
    - `pub fn new(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>, wifi: Arc<WifiService>, bus: Arc<EventBus>, cfg: DiagnoseConfig) -> Self`
    - `pub async fn start(&self) -> DiagnoseStart`. It never fails: not being connected is a result (spec §10), not an error.
    - `pub fn get(&self, id: DiagnosisId) -> Option<DiagnosisRun>`
    - `pub fn latest(&self) -> Option<DiagnosisRun>`
    - `pub async fn wait(&self, id: DiagnosisId) -> Option<DiagnosisRun>` resolves once the run is no longer `Running`. Unknown id → `None`.
  - `probe.rs`, one gather function per layer, each taking `(&dyn Platform` or `&dyn NetIo, …, &DiagnoseConfig)`:
    - `gather_link(..) -> (LinkProbe, Option<Interface>)`
    - `gather_router(.., gateway) -> RouterProbe`
    - `gather_internet(..) -> InternetProbe`
    - `gather_dns(.., dns_servers) -> DnsProbe`
    - `gather_website(..) -> WebsiteProbe`

**Runner rules:**
- **Ids and memory.** Ids start at 1. The last **8** runs are kept in memory; older ones are dropped.
- **Order.** Link, router, internet, DNS, website. After each layer: append the step, publish `DiagnoseStep`. On `Fail`, every later layer is appended as `skipped(layer)` (each published), the verdict is computed, and the run finishes.
- **Finishing.** `DiagnoseDone` is published last, with `state: Done` and the verdict.
- **Panics.** A panic inside the run task is caught (`catch_unwind`, as in Phase 1's scan). The run becomes `Failed` with error `Internal error during the diagnosis.` and `DiagnoseDone` is still published.
- **Time limits.** Each gather runs under `tokio::time::timeout(cfg.layer_timeout)`. On timeout, the layer is judged with every unfinished check counted as failed:
  - no ping replies;
  - `answers_arp: Some(false)`;
  - TCP closed;
  - the portal `Failed("timed out")`;
  - lookups `Err("timed out")`;
  - the website `Failed("timed out")`.
- **Link.** `active_interface`:
  - `NotConnected` → `interface: None`;
  - any other error is treated the same, with the error logged.

  For Wi-Fi, read `wifi_current` and map it to a `WifiReading`:
  - `Ok(Some(c))` → `Connected(c)`
  - `Ok(None)` → `Disconnected`
  - `PermissionNeeded` → `PermissionNeeded`
  - any other error → `Unavailable(message)`

  When `Connected`, call `wifi.nearby(cfg.nearby_max_age)` and take its `congestion`. Set `congestion = congestion.current` and `congestion_advice = Some(congestion.advice)`. Any error there leaves both `None`. Other interface kinds get `WifiReading::NotWifi`.
- **Router.** `gateway_pings` pings, one after another, each with `gateway_ping_timeout`. Only if none replied, `arp_resolve(gateway, arp_timeout)`.
- **Internet.** These run concurrently:
  - each target's pings, sequential within the target;
  - the TCP probe (`Open` counts as open; `Closed` and `Filtered` don't);
  - `http_get(portal_url, false)` through `classify_portal`.
- **DNS.**
  - For each name, ask the system servers in order until one returns a non-empty answer. Record that lookup, or the last failure.
  - Separately, ask `direct_dns` for each name.
  - Names run concurrently.
- **Website.** `http_get(website_url, false)`:
  - `Ok(r)` → `Status(r.status)`;
  - `Err(TlsCertificate { message })` → `Certificate(message)`;
  - any other error → `Failed(e.to_string())`.

**Scenarios.** Each is a full scenario file. Only the gateway and the laptop are needed as devices unless noted. Every one has `internet`, `dns`, `http` (the portal URL, the website URL) and `wifi` blocks.

| File | What differs from healthy | Expected verdict (`Finding`) |
|---|---|---|
| `home.json` (extend) | Healthy. The `wifi` block: the 5 GHz connection from Task 1 plus ~8 nearby networks, including "Lantern Home" on 2.4 GHz channel 6. Keep all 13 devices: the e2e test expects "13 devices found". | `Healthy` |
| `weak-wifi.json` | On 2.4 GHz channel 6 at −74 dBm. 9 other networks on channels 4–8, plus "Lantern Home" on 5 GHz channel 36 at −80 dBm. | `WeakAndCrowded` (journey A) |
| `dns-failure.json` | `dns_down: ["192.168.1.1"]` | `DnsServerProblem` |
| `provider-down.json` | Internet hosts have `ping_ms: null` and no open ports. Portal and website URLs are absent (so they error). `dns_down` has the gateway and 1.1.1.1. | `InternetDown` |
| `cafe.json` | SSID "Cafe Guest", security "Open". Portal URL → `302` with `location: "http://login.cafe-wifi.example/"`. Internet hosts don't answer ping and have no open ports. `dns_down: ["1.1.1.1"]`. Devices: gateway and laptop only. | `CaptivePortal` (journey C) |
| `tls-intercept.json` | Website URL has `tls_error: "invalid peer certificate: UnknownIssuer"` | `TlsIntercepted` |
| `router-down.json` | Gateway has `ping_ms: null` and `arp: false` | `RouterUnreachable` |
| `router-silent.json` | Gateway has `ping_ms: null` and `arp: true`; everything else healthy | `RouterIgnoresPing` (warn) |
| `location-denied.json` | `wifi.permission: "denied"`; everything else healthy | `Healthy`, with the link step `SignalHidden` |
| `offline.json` (exists) | No interface | `NotConnected`, with 4 skipped layers |

- [ ] **Step 1: Write the failing tests**

```rust
// tests/diagnose_sim.rs (paused clock)
async fn run(name: &str) -> DiagnosisRun { /* SimNetwork::load(scenario(name)); manager; start; wait */ }

#[tokio::test(start_paused = true)] async fn every_scenario_gets_its_verdict() {
    for (name, finding) in [("home", Finding::Healthy), ("weak-wifi", Finding::WeakAndCrowded), ("dns-failure", Finding::DnsServerProblem),
        ("provider-down", Finding::InternetDown), ("cafe", Finding::CaptivePortal), ("tls-intercept", Finding::TlsIntercepted),
        ("router-down", Finding::RouterUnreachable), ("router-silent", Finding::RouterIgnoresPing), ("location-denied", Finding::Healthy),
        ("offline", Finding::NotConnected)] {
        let r = run(name).await;
        assert_eq!(r.state, RunState::Done, "{name}");
        assert_eq!(r.verdict.unwrap().finding, finding, "{name}");
        assert_eq!(r.steps.len(), 5, "{name}: always five steps");
    }
}
#[tokio::test(start_paused = true)] async fn cafe_stops_at_internet_and_skips_dns_and_website() { /* steps[2] Fail CaptivePortal; steps[3..] Skipped */ }
#[tokio::test(start_paused = true)] async fn events_stream_five_steps_then_done_in_order() { /* subscribe; 5 diagnose.step for the run id, layers in order, then diagnose.done */ }
#[tokio::test(start_paused = true)] async fn second_start_while_running_returns_the_same_id() { /* start; start → AlreadyRunning(same) */ }
#[tokio::test(start_paused = true)] async fn get_returns_steps_so_far_mid_run() { /* after the first diagnose.step event: get(id).steps.len() == 1, state Running */ }
#[tokio::test(start_paused = true)] async fn a_hung_netio_is_cut_off_by_the_layer_limit() { /* NetIo whose http_get never returns → run finishes; internet evidence contains "timed out" */ }
#[tokio::test(start_paused = true)] async fn a_panicking_platform_fails_the_run_cleanly() { /* ping panics → state Failed, error "Internal error during the diagnosis.", DiagnoseDone published */ }
#[tokio::test(start_paused = true)] async fn only_eight_runs_are_kept() { /* 9 runs → get(first) None, latest() is the ninth */ }
#[tokio::test(start_paused = true)] async fn weak_wifi_mentions_the_crowded_channel_in_evidence() { /* steps[0].evidence contains "Channel 6 is shared with 9 other networks" */ }
// events.rs unit test
#[test] fn diagnose_event_json_shape() { /* {"type":"diagnose.done","data":{"run_id":1,"state":"done","verdict":{...},"error":null}} */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-core --test diagnose_sim`. Expected: FAIL.
- [ ] **Step 3:** Implement the runner, the events and the scenario files. If a scenario's verdict is wrong, fix the scenario only when the scenario contradicts the table above. Otherwise fix the code.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS. Phase 1's scan tests still pass on the extended `home.json`.
- [ ] **Step 5: Commit:** `Run diagnosis layer by layer with streamed steps, per-layer time limits and simulated scenarios for every verdict.`

---

### Task 7: Server routes for diagnosis and Wi-Fi

**Files:**
- Modify: `crates/netlantern-server/src/{state.rs,routes.rs,error.rs}`, `crates/netlantern/src/main.rs` (the `AppState::new` call)
- Test: `crates/netlantern-server/tests/api.rs` (extend), `crates/netlantern-server/src/error.rs` (unit)

**Interfaces:**
- Consumes: Task 4's `WifiService`, `WifiStatus` and `NearbyScan`, and Task 6's `DiagnoseManager`, `DiagnoseStart` and `DiagnosisRun`.
- Produces:
  - `AppState` gains `wifi: Arc<WifiService>` and `diagnose: Arc<DiagnoseManager>`. The constructor becomes `AppState::new(platform, netio, store, scan: ScanConfig, diagnose: DiagnoseConfig, policy)`, and every caller is updated.
  - **Routes**, all behind the existing guard:
    - `POST /api/diagnose` → `202 {"run_id": n, "already_running": bool}`. It never returns `409`: the UI just follows the id.
    - `GET /api/diagnose/latest` → the `DiagnosisRun`, or `404 not_found`.
    - `GET /api/diagnose/{id}` → the `DiagnosisRun`, or `404` (also for a non-numeric id).
    - `GET /api/wifi` → `200` `WifiStatus`.
    - `POST /api/wifi/scan` → `200` `NearbyScan` with `max_age` **10 s**.
  - **`ApiError` additions:**
    - `location_permission()`:
      - status `403`, code `location_permission`;
      - message `Windows is hiding Wi-Fi details from Net Lantern.`;
      - hint `Turn on Location services and "Let desktop apps access your location" in Settings > Privacy & security > Location.`
    - `wifi_unavailable(reason)`:
      - status `503`, code `wifi_unavailable`;
      - message `Wi-Fi details aren't available: {reason}.`;
      - hint `Check that Wi-Fi is turned on.`
    - `From<CoreError>` maps `Platform(PermissionNeeded { what: "location" })` to `location_permission()` for every route.
    - The two Wi-Fi routes map `Platform(Unavailable { reason })` to `wifi_unavailable(reason)`. Other routes keep `platform_error`.

- [ ] **Step 1: Write the failing tests** (the same session and test-client helpers as Phase 1's `api.rs`)

```rust
#[tokio::test] async fn diagnose_starts_and_can_be_fetched() { /* POST → 202 {run_id:1, already_running:false}; GET /api/diagnose/1 → 200 with "steps" array; eventually state "done" */ }
#[tokio::test] async fn second_diagnose_returns_the_running_id() { /* two POSTs back to back → same run_id, second already_running true */ }
#[tokio::test] async fn latest_diagnose_is_404_before_any_run() { }
#[tokio::test] async fn unknown_or_bad_diagnose_id_is_404_json() { /* /api/diagnose/99 and /api/diagnose/abc → 404 {"code":"not_found"} */ }
#[tokio::test] async fn wifi_status_on_the_home_scenario() { /* 200 {"interface_kind":"wifi","connection":{"ssid":"Lantern Home",...,"band":"5ghz"}} */ }
#[tokio::test] async fn wifi_with_location_denied_is_403_location_permission() { /* body {"code":"location_permission","message":"Windows is hiding Wi-Fi details from Net Lantern.","hint":"Turn on Location services ..."} */ }
#[tokio::test] async fn nearby_scan_returns_networks_and_congestion() { /* 200; networks sorted strongest first; congestion.advice non-empty */ }
#[tokio::test] async fn nearby_scan_without_an_adapter_is_503_wifi_unavailable() { /* stub platform wifi_scan → Unavailable{reason:"no Wi-Fi adapter"} → 503 code wifi_unavailable */ }
#[tokio::test] async fn diagnose_routes_need_a_session() { /* no cookie → 401 for POST /api/diagnose and GET /api/wifi */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern-server`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS.
- [ ] **Step 5: Commit:** `Serve diagnosis runs, Wi-Fi status and on-demand nearby scans over the API, with structured location-permission and no-adapter errors.`

---

### Task 8: `--diagnose-once` and `--wifi-once`

**Files:**
- Modify: `crates/netlantern/src/{cli.rs,main.rs,report.rs}`
- Test: `crates/netlantern/src/{cli.rs,report.rs}` (unit), `crates/netlantern/tests/exe.rs` (extend)

**Interfaces:**
- Consumes: Task 6's `DiagnoseManager` and Task 4's `WifiService`. Both are built the same way as `--scan-once` builds its backend: in memory, no server, nothing on disk.
- Produces:
  - **CLI flags.** `--diagnose-once` runs one diagnosis, prints it and exits. `--wifi-once` prints the current connection and a nearby scan, then exits. Both:
    - combine with `--demo` and `--demo=<file>`;
    - conflict with each other and with `--scan-once` (a clap `conflicts_with_all`).
  - `report.rs`:
    - `pub fn diagnosis_report(run: &DiagnosisRun) -> String`
    - `pub fn wifi_report(status: &Result<WifiStatus, CoreError>, nearby: &Result<NearbyScan, CoreError>) -> String`
  - **Exit codes:** 0 when the diagnosis ran, whatever the verdict, or when the Wi-Fi report printed (a permission message is a normal report). 1 when the backend can't start.

**Report formats (exact):**
- **Diagnosis.** One line per step, `format!("{:<13}{:<9}{}", title, status_word, explanation)`:
  - status words are `PASSED`, `WARNING`, `FAILED` and `SKIPPED`;
  - each evidence line follows as `  - {line}`;
  - then `  Try this: {fix}` when the step has a fix.

  After a blank line come `Verdict: {headline}` and, when present, `Try this: {fix}`.
- **Wi-Fi:**
  - Connected:
    - `Connected to "{ssid}" ({security})`
    - `  {band label} channel {ch}{, {w} MHz wide}, signal {rssi} dBm ({signal label}), {phy}, link {rx}/{tx} Mbps`
    - `  BSSID {bssid}`
  - Ethernet: `Connected by cable; Wi-Fi details don't apply.`
  - Not connected: `Not connected to Wi-Fi.`
  - Location: `Windows is hiding Wi-Fi details from Net Lantern.` and then the hint on the next line.
  - **Nearby:**
    - a tab-separated table `SSID\tBSSID\tBAND\tCHANNEL\tSIGNAL`, with hidden SSIDs shown as `(hidden)`;
    - then `Advice: {congestion.advice}`;
    - on error, the same location or unavailable messages.

- [ ] **Step 1: Write the failing tests**

```rust
// cli.rs
#[test] fn once_flags_conflict() { /* --diagnose-once --scan-once → ArgumentConflict; --diagnose-once --wifi-once → ArgumentConflict */ }
// report.rs
#[test] fn diagnosis_report_lists_steps_and_verdict() { /* a run with a Warn link step → contains "Wi-Fi link   WARNING  " and "Verdict: Your Wi-Fi signal is weak and the channel is crowded." */ }
#[test] fn wifi_report_explains_location_permission() { /* Err(Platform(PermissionNeeded{location})) → contains "Windows is hiding Wi-Fi details from Net Lantern." */ }
// tests/exe.rs (each with its own pipe-safe setup, as in Phase 1)
#[test] fn diagnose_once_on_the_cafe_scenario_says_sign_in() { /* netlantern --demo=<repo>/scenarios/cafe.json --diagnose-once → exit 0; stdout has "Verdict: Sign in to this network." and "SKIPPED" twice */ }
#[test] fn diagnose_once_on_weak_wifi_gives_journey_a_verdict() { /* stdout "Verdict: Your Wi-Fi signal is weak and the channel is crowded." */ }
#[test] fn wifi_once_on_the_demo_prints_the_connection_and_nearby_table() { /* --demo --wifi-once → "Connected to \"Lantern Home\"", "SSID\tBSSID", "Advice: " */ }
```

- [ ] **Step 2:** Run `cargo test -p netlantern`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test --workspace`. Expected: PASS. Then run the real thing on Windows, `cargo run -p netlantern -- --wifi-once` and `cargo run -p netlantern -- --diagnose-once`, and paste both outputs into the task report. Redact the SSID and BSSID if the author asks.
- [ ] **Step 5: Commit:** `Add --diagnose-once and --wifi-once to print a diagnosis or the Wi-Fi report from the terminal.`

---

### Task 9: UI: API types, live diagnosis state and resync

**Files:**
- Create: `ui/src/api/diagnose.ts`, `ui/src/api/wifi.ts`, `ui/src/lib/signal.ts`
- Modify: `ui/src/state/live.ts`, `ui/src/api/events.ts`
- Test:
  - `ui/src/api/diagnose.test.ts`
  - `ui/src/state/live.test.ts` (extend)
  - `ui/src/api/events.test.ts` (extend)
  - `ui/src/lib/signal.test.ts`

**Interfaces:**
- Consumes: Task 7's JSON shapes.
- Produces:
  - **`diagnose.ts`:**
    - TypeScript mirrors `DiagnosisRun`, `LayerResult`, `Verdict`, `LayerName` (`"link" | "router" | "internet" | "dns" | "website"`) and `LayerStatus`.
    - `startDiagnosis(): Promise<number>` returns `run_id`, and throws `ApiRequestError` otherwise.
    - `fetchDiagnosis(id: number): Promise<DiagnosisRun | null>`
    - `fetchLatestDiagnosis(): Promise<DiagnosisRun | null>`, where 404 gives `null`.
    - `export const LAYERS: LayerName[] = ["link", "router", "internet", "dns", "website"]`
  - **`wifi.ts`:**
    - mirrors `WifiStatus`, `WifiConnection`, `NearbyNetwork`, `NearbyScan`, `Congestion`, `ChannelLoad` and `CurrentLoad`;
    - `useWifi()` is a query with key `["wifi"]` and `retry: false`;
    - `useNearby()` is a query with key `["nearby"]` whose `queryFn` POSTs `/api/wifi/scan`, with `staleTime: Infinity` and `retry: false`; Refresh calls `refetch()`.
  - **`signal.ts`:** `signalLabel(dbm: number): "Excellent" | "Good" | "Fair" | "Weak"` (≥ −50, ≥ −60, ≥ −70, else).
  - **`live.ts`** gains:
    - `diagnosis: DiagnosisRun | null`
    - `followDiagnosis(run: DiagnosisRun)`, which replaces the followed run
    - `applyEvent` handling:
      - `diagnose.step` for the followed `run_id` replaces or appends the step by `layer`;
      - `diagnose.done` for the followed id sets `state`, `verdict` and `error`;
      - events for another id are ignored.
  - **`events.ts`:**
    - `onResync` also does two things:
      - if a diagnosis is followed and running, `fetchDiagnosis(id)`, then `followDiagnosis`;
      - `fetchLatestDiagnosis()`, and if that run is running with a different id, `followDiagnosis(it)`.
    - The 2 s polling tick while the socket is down also refreshes a running followed diagnosis.

- [ ] **Step 1: Write the failing tests**

```ts
it("applies steps and done for the followed run only", () => {
  useLive.getState().followDiagnosis(run({ id: 3, state: "running", steps: [] }));
  useLive.getState().applyEvent({ seq: 1, api_version: 1, type: "diagnose.step", data: { run_id: 3, step: step("link", "pass") } });
  useLive.getState().applyEvent({ seq: 2, api_version: 1, type: "diagnose.step", data: { run_id: 9, step: step("router", "fail") } });
  expect(useLive.getState().diagnosis!.steps.map((s) => s.layer)).toEqual(["link"]);
  useLive.getState().applyEvent({ seq: 3, api_version: 1, type: "diagnose.done", data: { run_id: 3, state: "done", verdict: verdict("healthy"), error: null } });
  expect(useLive.getState().diagnosis!.state).toBe("done");
});
it("resync reloads a running diagnosis and attaches to one started elsewhere", async () => { /* mockFetch GET /api/diagnose/latest → running id 5 → hello → live diagnosis id 5 */ });
it("polls a running diagnosis while the socket is down", async () => { /* fake timers; close; 2 s → GET /api/diagnose/3 called */ });
it("startDiagnosis returns the run id even when already running", async () => { /* 202 {run_id:4, already_running:true} → 4 */ });
it("signal labels", () => { expect(signalLabel(-50)).toBe("Excellent"); expect(signalLabel(-60)).toBe("Good"); expect(signalLabel(-70)).toBe("Fair"); expect(signalLabel(-71)).toBe("Weak"); });
```

- [ ] **Step 2:** Run `npm test` in `ui/`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm test`, then `npm run lint && npm run build`. Expected: all PASS.
- [ ] **Step 5: Commit:** `Add UI API types and live state for diagnosis runs and Wi-Fi, with resync and polling while the event stream is down.`

---

### Task 10: UI: the diagnosis trace

**Files:**
- Create: `ui/src/pages/Diagnose.tsx`, `ui/src/components/LayerTrace.tsx`
- Modify: `ui/src/App.tsx` (the route uses `Diagnose` instead of `Placeholder`)
- Test: `ui/src/pages/Diagnose.test.tsx`, `ui/src/components/LayerTrace.test.tsx`

**Interfaces:**
- Consumes: Task 9's `startDiagnosis`, `fetchLatestDiagnosis`, `LAYERS`, `useLive().diagnosis` and `followDiagnosis`, plus Phase 1's `formatAgo`.
- Produces:
  - **The `Diagnose` page.** A heading `Diagnose`, then the diagnosis section (this task), then the Wi-Fi section (Task 11 fills a `<WifiQuality />` slot; this task leaves the slot empty).
  - **On mount:** `fetchLatestDiagnosis()`. If a run comes back, `followDiagnosis(it)`.
  - **The "Run diagnosis" button:**
    - it calls `startDiagnosis()`, then `fetchDiagnosis(id)` and `followDiagnosis`;
    - while the followed run is running, it is disabled and reads `Diagnosing…`;
    - when the start fails, the server message shows in `role="alert"`.
  - **The verdict card** (`<section aria-label="Verdict">`):
    - shows the verdict headline as a heading, and `Try this: {fix}` when present;
    - the status is written as text: `All clear`, `Warning` or `Problem found`;
    - below it, `Checked {formatAgo(finished_at)}`;
    - with no run yet, it says `Run a diagnosis to check each step from your Wi-Fi to a website.`
  - **`LayerTrace`** (`<ol aria-label="Diagnosis steps">`) shows one item per `LAYERS` entry, in order. Each item shows:
    - the step's title, or a default title while pending: `Wi-Fi link`, `Router`, `Internet`, `DNS`, `Website`;
    - the status text: `Passed`, `Warning`, `Failed`, `Not checked`; while running, the first missing layer shows `Checking…` and later ones `Waiting`;
    - the explanation;
    - the evidence as a `<ul>`;
    - `Try this: {fix}`.

    Status colours come from tokens (pass, warn, fail, muted), and the text is always present.
  - A failed run (`state: "failed"`) shows `The diagnosis stopped: {error}` in `role="alert"`.

- [ ] **Step 1: Write the failing tests**

```ts
it("runs a diagnosis and streams steps into the trace", async () => {
  mockFetch({ "GET /api/diagnose/latest": [404, notFound], "POST /api/diagnose": [202, { run_id: 1, already_running: false }],
              "GET /api/diagnose/1": [200, run({ id: 1, state: "running", steps: [] })] });
  renderDiagnose();
  await user.click(await screen.findByRole("button", { name: "Run diagnosis" }));
  expect(await screen.findByRole("button", { name: "Diagnosing…" })).toBeDisabled();
  act(() => useLive.getState().applyEvent(stepEvent(1, step("link", "pass", { explanation: "Your laptop is connected and the signal is fine." }))));
  const items = within(screen.getByRole("list", { name: "Diagnosis steps" })).getAllByRole("listitem");
  expect(items[0]).toHaveTextContent("Passed"); expect(items[1]).toHaveTextContent("Checking…"); expect(items[2]).toHaveTextContent("Waiting");
});
it("shows the verdict with its fix and skipped layers as Not checked", () => { /* followed done run, cafe-like → heading "Sign in to this network.", "Try this: Open any website…", items 4 and 5 "Not checked" */ });
it("loads the latest run on open", async () => { /* GET latest 200 done → verdict visible, "Checked … ago" */ });
it("shows the server's error when starting fails", async () => { /* POST 500 → role=alert with message */ });
it("a failed run says why it stopped", () => { /* state failed, error "Internal error during the diagnosis." */ });
```

- [ ] **Step 2:** Run `npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npm test`, then lint and build. Expected: all PASS.
- [ ] **Step 5: Commit:** `Replace the Diagnose placeholder with a layer-by-layer trace, a verdict card and a run button that follows live steps.`

---

### Task 11: UI: Wi-Fi quality, the location-permission flow, nearby networks and congestion

**Files:**
- Create: `ui/src/components/{WifiQuality.tsx,LocationPermission.tsx,NearbyNetworks.tsx,CongestionChart.tsx}`
- Modify: `ui/src/pages/Diagnose.tsx` (fill the slot), `ui/src/pages/Dashboard.tsx` (the SSID line)
- Test: `ui/src/components/WifiQuality.test.tsx`, `ui/src/components/NearbyNetworks.test.tsx`, `ui/src/pages/Dashboard.test.tsx` (extend)

**Interfaces:**
- Consumes: Task 9's `useWifi`, `useNearby`, `signalLabel` and the types.
- Produces:
  - **`WifiQuality`** (`<section aria-label="Wi-Fi quality">`):
    - **Connected:** a definition list with:
      - Network (SSID)
      - Signal: `{rssi} dBm · {signalLabel}`
      - Band: `5 GHz`
      - Channel: `36`, plus ` · 80 MHz wide` when the width is known
      - Standard (phy)
      - Link speed: `{rx} Mbps down · {tx} Mbps up`, with missing values left out
      - Security
      - BSSID
    - **Ethernet:** `Connected by cable — Wi-Fi details don't apply.`
    - **Not connected:** `Not connected to Wi-Fi.`
    - **`403 location_permission`:** renders `<LocationPermission />`.
    - **Other errors:** the server message.
  - **`LocationPermission`** is exact copy:
    - heading `Windows is hiding Wi-Fi details`
    - text `Windows only shares Wi-Fi names and signal details with apps that have location access. Net Lantern doesn't use your location — it only reads Wi-Fi details.`
    - an ordered list:
      1. `Open Settings > Privacy & security > Location.`
      2. `Turn on Location services.`
      3. `Turn on Let desktop apps access your location.`
    - a link `Open location settings` with `href="ms-settings:privacy-location"`
    - a button `Check again`, which refetches `["wifi"]` and `["nearby"]`
    - the note `Diagnosis still checks your router, internet and DNS without it.`
  - **`NearbyNetworks`** (`<section aria-label="Nearby networks">`):
    - On mount it reads `useNearby()`, which triggers the POST. That is the "Diagnose screen open" scan from spec §6.7.
    - A `Refresh` button calls `refetch()` and reads `Scanning…` while fetching.
    - **The table** has the columns Network, Signal, Band, Channel. Hidden SSIDs show `(hidden network)`, and the current BSSID's row says `(you)`. At phone width the Band column hides and the band joins the Channel cell.
    - **The advice:** `congestion.advice` as a paragraph.
    - **`<CongestionChart band="2.4ghz" />` and `<CongestionChart band="5ghz" />`:**
      - inline SVG, `role="img"`, with `aria-label` `"2.4 GHz channels: channel 6 has 9 networks, …"`;
      - one bar per channel in `congestion.channels` for that band, plus the current channel;
      - every bar has its count as a text label, and the current channel's bar is labelled `Your channel`;
      - the 2.4 GHz chart always shows channels 1–13; the 5 GHz chart shows only channels with networks;
      - it scales to the container width.
    - **Errors:** `403` shows a one-line pointer `Turn on location access (above) to see nearby networks.`, and `503 wifi_unavailable` shows the server message.
  - **Dashboard.** The network card adds `Wi-Fi: {ssid}` from `useWifi()` when connected, and adds nothing on error.

- [ ] **Step 1: Write the failing tests**

```ts
it("shows the current connection with a signal label", async () => {
  mockFetch({ "GET /api/wifi": [200, { interface_kind: "wifi", connection: conn({ ssid: "Lantern Home", rssi_dbm: -52, channel: 36, band: "5ghz", width_mhz: 80 }) }] });
  render(<WifiQuality />);
  expect(await screen.findByText("Lantern Home")).toBeInTheDocument();
  expect(screen.getByText("-52 dBm · Good")).toBeInTheDocument();
  expect(screen.getByText("36 · 80 MHz wide")).toBeInTheDocument();
});
it("walks the user through location permission", async () => {
  mockFetch({ "GET /api/wifi": [403, { code: "location_permission", message: "Windows is hiding Wi-Fi details from Net Lantern.", hint: "…" }] });
  render(<WifiQuality />);
  expect(await screen.findByRole("heading", { name: "Windows is hiding Wi-Fi details" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Open location settings" })).toHaveAttribute("href", "ms-settings:privacy-location");
  await user.click(screen.getByRole("button", { name: "Check again" }));
  expect(fetchCount("GET /api/wifi")).toBe(2);
});
it("explains Ethernet", async () => { /* interface_kind ethernet → "Connected by cable — Wi-Fi details don't apply." */ });
it("lists nearby networks, marks yours and hidden ones, and shows the advice", async () => { /* POST /api/wifi/scan 200 → rows; "(you)"; "(hidden network)"; advice text */ });
it("labels the congestion chart for screen readers and marks your channel", async () => { /* role=img name /2.4 GHz channels: channel 6 has 9 networks/; text "Your channel" */ });
it("refresh scans again", async () => { /* click Refresh → second POST */ });
it("no adapter shows the server message", async () => { /* POST 503 wifi_unavailable → message */ });
it("dashboard shows the Wi-Fi name", async () => { /* GET /api/wifi 200 → "Wi-Fi: Lantern Home" */ });
```

- [ ] **Step 2:** Run `npm test`. Expected: FAIL.
- [ ] **Step 3:** Implement. Check the Diagnose page at 375 px width with `npm run dev` against `--demo --dev-origin`: no horizontal page scroll, and the charts fit.
- [ ] **Step 4:** Run `npm test`, then lint and build. Expected: all PASS.
- [ ] **Step 5: Commit:** `Show Wi-Fi quality, a location-permission guide, nearby networks and channel congestion charts on the Diagnose screen, and the Wi-Fi name on the Dashboard.`

---

### Task 12: End-to-end, docs and spec records

**Files:**
- Modify:
  - `ui/e2e/demo.spec.ts`
  - `README.md`
  - `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§6.6, §6.7, §6.12, §15)
- Test: `ui/e2e/demo.spec.ts`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Write the e2e tests.** They join the existing serial tests on `--demo` (the healthy `home.json`):

```ts
test("diagnose: every layer passes on the demo network", async () => {
  await page.getByRole("link", { name: "Diagnose" }).click();
  await page.getByRole("button", { name: "Run diagnosis" }).click();
  await expect(page.getByRole("heading", { name: "Everything looks fine." })).toBeVisible({ timeout: 30_000 });
  const steps = page.getByRole("list", { name: "Diagnosis steps" }).getByRole("listitem");
  await expect(steps).toHaveCount(5);
  for (let i = 0; i < 5; i++) await expect(steps.nth(i)).toContainText("Passed");
});
test("diagnose: Wi-Fi quality and nearby networks", async () => {
  await expect(page.getByLabel("Wi-Fi quality")).toContainText("Lantern Home");
  await expect(page.getByLabel("Nearby networks").getByRole("row").nth(1)).toBeVisible();
  await expect(page.getByRole("img", { name: /2\.4 GHz channels/ })).toBeVisible();
});
test("diagnose: the last run is still shown after a reload", async () => {
  await page.reload();
  await expect(page.getByRole("heading", { name: "Everything looks fine." })).toBeVisible();
});
```

- [ ] **Step 2:** Run `cargo build -p netlantern`, then `npm run build && npm run e2e` in `ui/`. Expected: PASS. If a test fails, fix the owning task's code, not the test.
- [ ] **Step 3: Update the spec.** Each change below is tagged "(changed in phase 2)":
  - **§6.6:**
    - a gateway that answers ARP but not ping warns and continues;
    - the portal probe counts 3xx, 511 and any unexpected answer as a portal;
    - blocked ICMP with working TCP or HTTP passes the internet layer;
    - the website check treats any HTTP status as reachable and doesn't follow redirects;
    - the location permission never fails the link layer.
  - **§6.7:** the overlap rule (2.4 GHz channels less than 5 apart; same channel on 5 and 6 GHz; width ignored for now) and the crowded thresholds.
  - **§6.12:** every row marked "new" in the Global Constraints table.
  - **§15:** the gateway-ignores-ping fallback for diagnosis, and that redirects are only followed on the same host.
- [ ] **Step 4: Update the README.**
  - Status becomes "Phase 2: diagnosis and Wi-Fi quality".
  - Add what diagnosis contacts on the internet, as the exact list from the Global Constraints, and that it only happens when you run a diagnosis.
  - Add the location-permission note and the Settings path.
  - Add `--diagnose-once`, `--wifi-once` and the scenario files, for example `netlantern --demo=scenarios/cafe.json`.
- [ ] **Step 5: Run the full local check.**
  - `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  - Then, in `ui/`: `npm run lint && npm test && npm run build && npm run e2e`
  - Everything must PASS.
- [ ] **Step 6: Commit:** `Add diagnosis end-to-end tests and record the phase 2 rules and defaults in the README and spec.`

---

## Phase 2 review checkpoint (spec §13)

- [ ] **Demo:** run `netlantern` on the author's network and open Diagnose. Show:
  - the layers filling in live;
  - the verdict;
  - the Wi-Fi card and the nearby networks with the congestion charts.

  Then run `netlantern --demo=scenarios/cafe.json` and show "Sign in to this network", and `--demo=scenarios/weak-wifi.json` for journey A.
- [ ] **Done-when (spec §12 and §2.1):**
  - Every simulated scenario gives its verdict. This is Task 6's matrix test, plus `--diagnose-once` on each file.
  - **Real network: wrong DNS server.** Set the Wi-Fi adapter's DNS to an unused LAN address (for example `192.168.1.250`) in Windows network settings. Expected: `DnsServerProblem`. Then restore automatic DNS.
  - **Real network: unplugged WAN cable.** Unplug the router's internet cable. Expected: `InternetDown` (or `CaptivePortal`, if the router serves its own "no internet" page; record which). Plug it back in.
  - Record both results in §2.1.
- [ ] **Location-rule risk (§16):** run `netlantern --wifi-once` three times:
  - with Location services off;
  - with Location on but "Let desktop apps access your location" off;
  - with both on.

  Record in §16 which give Wi-Fi details for the unpackaged exe. If none do, the guidance screen is the fallback, and the retest moves to phase 7's packaged app.
- [ ] **Manual checklist:**
  - On Ethernet: the link says "Cable link", and the Wi-Fi card says cable.
  - Airplane mode: `NotConnected`, four layers "Not checked".
  - A public hotspot with a portal, if available: "Sign in to this network".
  - A VPN on: the verdict is still sensible; note anything odd.
  - Diagnose at phone width (375 px): no horizontal scroll.
- [ ] **Summary** for the author:
  - why the layer order matters;
  - how the captive-portal probe works;
  - what the WLAN API can and can't see;
  - why nearby scans are on demand only.
- [ ] **Learning check.** Ask the author to explain two or three of the phase 2 goals in their own words:
  - what failure looks like at each layer, and why diagnosis stops at the first failure;
  - how captive portals work and how a plain-HTTP probe detects them;
  - the DNS resolution path (system server versus 1.1.1.1, and what each failure means);
  - the WLAN API and Windows' location rule;
  - RSSI, channels, channel width, and why 2.4 GHz channels 1, 6 and 11 don't overlap.

  Walk through any gaps in the code.
- [ ] **Next:** write the Phase 3 plan (monitor and alerts), starting with the toast and Tauri spikes.
