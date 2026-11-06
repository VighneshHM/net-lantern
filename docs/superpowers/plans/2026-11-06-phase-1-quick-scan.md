# Net Lantern Phase 1 — Quick Scan + Devices Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Running `netlantern` with no flags scans the real network the laptop is on. It lists every device with its vendor, an announced name and a type, and names and stars set by the user persist across scans and restarts.

**Architecture:**
- `netlantern-platform` gains:
  - a Windows `Platform` (`win::WindowsPlatform`, IP Helper APIs);
  - a portable `NetIo` (`net::RealNetIo`: DNS over UDP, HTTP, SSDP, NetBIOS and one-shot mDNS);
  - the discovery methods on `NetIo`, which the simulator also fakes.
- `netlantern-core` gains:
  - the OUI table and identification (names and the type classifier);
  - SQLite storage on one writer thread;
  - a quick-scan pipeline that streams devices while the sweep runs and persists them per network.
- The server, the exe and the UI grow around those pieces.

**Tech Stack:** Phase 0's stack plus:
- `windows` (IP Helper and WinSock features)
- `rusqlite` (`bundled`)
- `hickory-proto` (DNS message encode/decode)
- `reqwest` (`rustls-tls`, no default features) as a normal dependency
- `csv` (xtask only)

Add each with `cargo add` at its current release and record it in `[workspace.dependencies]`.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`: the Phase 1 row of §12, §2.1, §3, §5.1, §6.1, §6.2, §6.3, §6.4, §6.5, §6.12, §7, §8, §10 and §13. Phase 0 plan: `docs/superpowers/plans/2026-10-31-phase-0-foundations.md`.

## Global Constraints

- **Windows 11, no admin rights for anything.** CI runs on `windows-latest`. Never parse the text output of OS tools (`arp -a`, `netsh`, `ipconfig`).
- **IPv4 only.** Sweep rule unchanged from Phase 0: up to a /22, else the /24 around the laptop.
- **Quick-scan defaults (spec §6.12):**
  - ICMP: concurrency **64**, timeout **800 ms**, retries **1**.
  - TCP probe ports **80, 443, 445, 22, 62078**.
  - ARP resolve concurrency **32**.
  - Discovery listen window **3 s**.
  - New defaults introduced by this plan are recorded in §6.12 in Task 16:
    - TCP probe timeout **800 ms** and concurrency **64**
    - ARP resolve timeout **3 s**
    - NetBIOS timeout **1 s**
    - DNS query timeout **1 s**
    - UPnP description fetch timeout **2 s**
    - ARP poll interval during the sweep **1 s**
- **Success targets (§2.1):**
  - A /24 finishes in **≤ 20 s**, with the **first devices on screen within 3 s**.
  - **≥ 95%** of the router's client list is found.
  - **100%** of non-private MACs show a vendor.
  - **≥ 70%** of devices get a type other than unknown.
- **Network traffic stays on the LAN:**
  - ICMP, ARP and TCP go only to swept addresses.
  - SSDP and mDNS go to their multicast groups from an ephemeral port bound to the interface address. Port 5353 is never bound.
  - NetBIOS and reverse DNS are unicast, to found devices and the gateway respectively.
  - HTTP fetches only an SSDP `LOCATION` whose host is the replying device's own private IPv4 address.
  - Nothing goes to the internet in this phase.
- **Demo mode never touches user data.** `--demo` uses an in-memory database. Real mode uses `%LOCALAPPDATA%\NetLantern\netlantern.db`, or `--data-dir <dir>`.
- Never log tokens or session ids. Host and Origin values in logs are truncated to 100 characters.
- **API:**
  - `API_VERSION` stays **1**. The API has no outside clients yet, so the device shape changes without a bump.
  - Every error body is `{"code","message","hint"}`.
  - The JSON field for a device's type is `"type"`.
- **UI:** status is always shown as text, touch targets are ≥ 44 px, it works at phone width, and colours come only from the tokens.
- **Commit messages carry no `Co-Authored-By` line and no Claude or Anthropic attribution.** The author is the sole author.

## Review Focus

1. **A VPN, Hyper-V's `vEthernet` or a second adapter is up.** Expected: Net Lantern picks the physical Wi-Fi or Ethernet adapter that has a gateway, not the VPN, the virtual switch or an APIPA address. Tested in Task 2 (`choose_interface`).
2. **A phone rejoins with a new private MAC.** Expected: it keeps its name and does not become a new device. But two different phones that both announce `iPhone` in the same scan must stay two devices. Tested in Task 8.
3. **DHCP moves a device to a new IP, or hands its old IP to another device.** Expected: names follow the device (MAC), the other one shows "Offline", and there are no duplicate rows. Tested in Task 8 and Task 10.
4. **The laptop moves to another network (home, then café).** Expected: the Devices list shows only the current network's devices, and the home names come back on return. Tested in Task 8 and Task 11.
5. **mDNS, SSDP, NetBIOS or reverse DNS are blocked or unsupported.** Expected: the scan still finishes with IPs, MACs and vendors, and discovery errors never fail it. Tested in Task 9 and Task 10 with a failing `NetIo`.

## Not in this phase

These spec items are deliberately left to later phases, so no task covers them:
- **Cancelling a scan cleanly when the network changes mid-scan (§10).** This needs network-change detection, which arrives with monitoring in phase 3.
- **The replaced-router prompt and the first-run screen (§6.1).** Phase 6.
- **SSID labels.** Phase 2 (WLAN API).
- **Deep scan and its consent prompt.** Phase 4.
- **Log files.** Phase 7.
- **The separate `netlantern-cli` crate (§5).** `--scan-once` covers the scan part of that dev tool for now.

---

### Task 1: Discovery types on `NetIo`, simulator support, richer home scenario

**Files:**
- Modify:
  - `crates/netlantern-platform/src/{netio.rs,lib.rs,sim/mod.rs,sim/scenario.rs}`
  - `scenarios/home.json`
- Create:
  - `scenarios/isolated.json`
  - `crates/netlantern-platform/src/tcp.rs`, a portable `tcp_connect_probe` shared by every `Platform`
- Test: `crates/netlantern-platform/tests/sim.rs` (extend)

**Interfaces:**
- Consumes: Phase 0's `NetIo`, `SimNetwork`, `Platform` and `MacAddr`.
- Produces, all re-exported from `netlantern_platform`:
  - `MdnsService { ip: Ipv4Addr, service_type: String /* "_ipp._tcp" */, instance: String /* "Office printer" */, hostname: Option<String> /* "brn3c2af4.local" */, friendly_name: Option<String> /* TXT fn */, model: Option<String> /* TXT md, model or ty */ }`
  - `SsdpReply { ip: Ipv4Addr, st: String, server: Option<String>, location: Option<String> }`
  - `NetbiosInfo { name: String, workgroup: Option<String> }`
  - All three derive `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`.
  - New `NetIo` methods:
    - `reverse_dns(&self, server: Ipv4Addr, ip: Ipv4Addr) -> Result<Option<String>, PlatformError>`
    - `mdns_browse(&self, local: Ipv4Addr, window: Duration) -> Result<Vec<MdnsService>, PlatformError>`
    - `ssdp_search(&self, local: Ipv4Addr, window: Duration) -> Result<Vec<SsdpReply>, PlatformError>`
    - `netbios_status(&self, ip: Ipv4Addr, timeout: Duration) -> Result<Option<NetbiosInfo>, PlatformError>`
  - `pub async fn tcp_connect_probe(ip: Ipv4Addr, port: u16, timeout: Duration) -> TcpProbe`:
    - accepted → `Open`
    - `ConnectionRefused` → `Closed`
    - timeout or any other error → `Filtered`
  - `SimNetwork::tcp_probe` keeps its scenario behaviour.

**Scenario additions.** These are optional per-device fields, and all default to absent:
```json
"mdns":    [{ "service": "_googlecast._tcp", "instance": "Google-Nest-Mini-3c1e", "hostname": "3c1e9a7b-1f.local", "friendly_name": "Kitchen speaker", "model": "Google Nest Mini" }],
"ssdp":    { "st": "urn:schemas-upnp-org:device:MediaRenderer:1", "server": "Samsung UPnP/1.0", "location": "http://192.168.1.52:9197/dmr" },
"netbios": { "name": "DISKSTATION", "workgroup": "WORKGROUP" },
"rdns":    "echo-dot.lan"
```

**Sim behaviour:**
- `mdns_browse` and `ssdp_search` sleep for `window`, then return every device's entries in IP order.
- `netbios_status` and `reverse_dns` answer from the device's fields, or return `Ok(None)`.
- An unknown or absent device sleeps for `timeout` first.
- When `interface` is null, all four return `Err(NotConnected)`.
- `http_get` already serves the `http` map; it is used for UPnP description XML.

**`scenarios/home.json` identities.** Keep the IPs, MACs and pings from Phase 0. For each MAC, check its real OUI vendor in `oui.tsv` once Task 6 exists, and if it differs, change the MAC to a real OUI of the intended vendor; never change the test to fit.

| ip | intended vendor | additions | expected type |
|---|---|---|---|
| .1 | TP-Link | ssdp `urn:schemas-upnp-org:device:InternetGatewayDevice:1`, location `http://192.168.1.1:1900/igd.xml` serving `<friendlyName>Archer AX55</friendlyName><deviceType>urn:schemas-upnp-org:device:InternetGatewayDevice:1</deviceType>`; rdns `router.lan` | router |
| .23 | Dell (this laptop) | netbios `LANTERN-LAPTOP` | laptop |
| .31 | Google | mdns `_googlecast._tcp`, friendly_name `Kitchen speaker`, model `Google Nest Mini` | speaker |
| .44 | private MAC | rdns `Pixel-7.lan` | phone |
| .52 | Samsung | ssdp MediaRenderer, server `Samsung UPnP/1.0`, location serving `<friendlyName>[TV] Living Room</friendlyName><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType>` | tv |
| .57 | Apple | open_ports add `62078`; mdns `_apple-mobdev2._tcp` hostname `iPhone.local` | phone |
| .61 | Amazon | rdns `echo-dot.lan` | speaker |
| .70 | Synology | netbios `DISKSTATION`; mdns `_smb._tcp` and `_afpovertcp._tcp`, hostname `DiskStation.local` | nas |
| .88 | Espressif | (none; no ping, port 80) | iot |
| .95 | Sony Interactive | (none) | console |
| .102 | Ring | (none) | camera |
| .120 | LCFC (Lenovo) | netbios `WORK-LAPTOP`; open_ports add `445` | laptop |
| .130 | Signify | mdns `_hue._tcp` instance `Hue Bridge - 1A2B3C`; ssdp server `Hue/1.0 UPnP/1.0 IpBridge` | iot |

**`scenarios/isolated.json`:**
- Same interface as home.
- Devices: the gateway `.1` and the laptop `.23` only.

- [ ] **Step 1: Write the failing tests** (`tests/sim.rs`, paused clock)

```rust
let svcs = home.mdns_browse(ip("192.168.1.23"), secs(3)).await.unwrap();
assert!(svcs.iter().any(|s| s.ip == ip("192.168.1.31") && s.friendly_name.as_deref() == Some("Kitchen speaker")));
let ssdp = home.ssdp_search(ip("192.168.1.23"), secs(3)).await.unwrap();
assert!(ssdp.iter().any(|r| r.ip == ip("192.168.1.52") && r.st.contains("MediaRenderer")));
assert_eq!(home.netbios_status(ip("192.168.1.70"), secs(1)).await.unwrap().unwrap().name, "DISKSTATION");
assert_eq!(home.netbios_status(ip("192.168.1.31"), secs(1)).await.unwrap(), None);
assert_eq!(home.reverse_dns(ip("192.168.1.1"), ip("192.168.1.61")).await.unwrap().as_deref(), Some("echo-dot.lan"));
assert!(matches!(offline.mdns_browse(ip("0.0.0.0"), secs(3)).await, Err(PlatformError::NotConnected)));
// tcp_connect_probe against real sockets (not paused)
let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); let port = l.local_addr().unwrap().port();
assert_eq!(tcp_connect_probe(ip("127.0.0.1"), port, ms(800)).await, TcpProbe::Open);
drop(l);
assert_eq!(tcp_connect_probe(ip("127.0.0.1"), port, ms(800)).await, TcpProbe::Closed);
```

- [ ] **Step 2:** `cargo test -p netlantern-platform` → FAIL (missing methods).
- [ ] **Step 3:** Implement the types, the trait methods, the sim fields and behaviour, `tcp.rs`, and the scenario files.
  - Every other `NetIo` implementor in the tests (stubs in `netlantern-server/tests`) gets the new methods returning `Ok(vec![])` or `Ok(None)`.
- [ ] **Step 4:** `cargo test --workspace` → PASS; `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- [ ] **Step 5: Commit:** `feat(platform): discovery methods on NetIo, simulated identities, tcp connect probe`

---

### Task 2: Windows adapters, interface choice and neighbour table

**Files:**
- Create:
  - `crates/netlantern-platform/src/iface_select.rs`: pure and portable, unit tests inline
  - `crates/netlantern-platform/src/win/{mod.rs,adapters.rs,neighbors.rs}`: `#[cfg(windows)]`, and the only place for `unsafe`
- Modify:
  - `crates/netlantern-platform/Cargo.toml`: `[target.'cfg(windows)'.dependencies] windows` with features `Win32_Foundation`, `Win32_NetworkManagement_IpHelper`, `Win32_NetworkManagement_Ndis`, `Win32_Networking_WinSock`; `tokio` gains `rt`, `net`
  - `lib.rs`
- Test: `crates/netlantern-platform/tests/windows.rs` (`#![cfg(windows)]`)

**Interfaces:**
- Produces:
  - `AdapterInfo { if_index: u32, if_type: u32, up: bool, ipv4: Vec<(Ipv4Addr, u8)>, gateways: Vec<Ipv4Addr>, dns: Vec<Ipv4Addr>, mac: Option<MacAddr>, metric: u32 }`
  - `pub fn choose_interface(adapters: &[AdapterInfo]) -> Option<(Interface, u32 /* if_index */)>`. The rules, in order:
    1. Keep adapters that are `up`, have a 6-byte MAC, at least one IPv4 address outside `169.254.0.0/16`, and at least one IPv4 gateway that isn't `0.0.0.0`.
    2. Rank `if_type` 71 (IEEE 802.11 → `Wifi`) and 6 (Ethernet → `Ethernet`) above everything else (`Other`: VPN, tunnel, PPP).
    3. Then rank the lowest `metric` first.
    4. Then the lowest `if_index`.
    - Use the adapter's first non-APIPA IPv4 address and its prefix.
  - `pub fn neighbor_state(raw: i32) -> Option<NeighborState>` for `NL_NEIGHBOR_STATE`: 0 Unreachable, 1 Incomplete, 2 Probe, 3 Delay, 4 Stale, 5 Reachable, 6 Permanent; anything else → `None`.
  - `win::read_adapters() -> Result<Vec<AdapterInfo>, PlatformError>`:
    - Calls `GetAdaptersAddresses(AF_INET, GAA_FLAG_INCLUDE_GATEWAYS | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST)`.
    - Retries the buffer size on `ERROR_BUFFER_OVERFLOW`.
  - `win::read_neighbors(if_index: u32) -> Result<Vec<ArpEntry>, PlatformError>`:
    - Calls `GetIpNetTable2(AF_INET)` and frees the table with `FreeMibTable`.
    - Keeps rows of `if_index` only.
    - Skips rows whose MAC length ≠ 6 or whose MAC is all zeros, and rows whose state maps to `None`.

- [ ] **Step 1: Write the failing tests** (inline in `iface_select.rs`; builder `adapter(if_type, ip, prefix, gw, metric)`)

```rust
#[test] fn prefers_wifi_with_gateway_over_vpn_and_hyperv() {
    let vpn    = adapter(53, "10.8.0.6", 24, Some("10.8.0.1"), 1);      // tunnel, lowest metric
    let vswitch= adapter(6,  "172.20.0.1", 20, None, 5);                // vEthernet: no gateway
    let wifi   = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 35);
    let (iface, idx) = choose_interface(&[vpn, vswitch, wifi.clone()]).unwrap();
    assert_eq!((iface.ip, iface.prefix, iface.kind, idx), (ip("192.168.1.23"), 24, InterfaceKind::Wifi, wifi.if_index));
}
#[test] fn skips_apipa_and_down_adapters() {
    let apipa = adapter(6, "169.254.10.2", 16, Some("169.254.0.1"), 1);
    let mut down = adapter(71, "192.168.1.23", 24, Some("192.168.1.1"), 1); down.up = false;
    assert!(choose_interface(&[apipa, down]).is_none());
}
#[test] fn falls_back_to_other_when_only_a_vpn_has_a_gateway() {
    let (iface, _) = choose_interface(&[adapter(53, "10.8.0.6", 24, Some("10.8.0.1"), 1)]).unwrap();
    assert_eq!(iface.kind, InterfaceKind::Other);
}
#[test] fn lower_metric_wins_between_physical_adapters() { /* ethernet metric 25 beats wifi metric 35 */ }
#[test] fn maps_neighbor_states() {
    assert_eq!(neighbor_state(5), Some(NeighborState::Reachable));
    assert_eq!(neighbor_state(1), Some(NeighborState::Incomplete));
    assert_eq!(neighbor_state(42), None);
}
```
In `tests/windows.rs`:

```rust
#[test] fn reads_adapters_without_admin() { let a = read_adapters().unwrap(); assert!(!a.is_empty()); }
#[test] fn neighbor_table_reads() { if let Some((_, idx)) = choose_interface(&read_adapters().unwrap()) { read_neighbors(idx).unwrap(); } }
```

- [ ] **Step 2:** `cargo test -p netlantern-platform` → FAIL.
- [ ] **Step 3:** Implement `iface_select.rs`, `win/adapters.rs` and `win/neighbors.rs`.
  - Convert `SOCKADDR_IN` addresses with `Ipv4Addr::from(u32::from_be(...))`, or byte-wise from `S_un_b`.
  - Each `unsafe` block gets a `// SAFETY:` comment.
- [ ] **Step 4:** `cargo test -p netlantern-platform` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(platform): Windows adapter and neighbour-table readers, interface choice`

---

### Task 3: `WindowsPlatform`: ping, ARP resolve, TCP probe

**Files:**
- Create: `crates/netlantern-platform/src/win/{icmp.rs,arp.rs,platform.rs}`
- Modify: `win/mod.rs`, `lib.rs`, which re-exports `win::WindowsPlatform` under `#[cfg(windows)]`
- Test: `crates/netlantern-platform/tests/windows.rs` (extend)

**Interfaces:**
- Consumes: `read_adapters`, `choose_interface`, `read_neighbors` (Task 2) and `tcp_connect_probe` (Task 1).
- Produces: `WindowsPlatform::new() -> Self` implementing `Platform`:
  - `active_interface`:
    - `read_adapters` + `choose_interface`.
    - `None` → `Err(NotConnected)`.
    - Caches the chosen `if_index` for `arp_table`.
  - `ping`:
    - `IcmpCreateFile` + `IcmpSendEcho` (32-byte payload, the reply buffer sized for one `ICMP_ECHO_REPLY` + payload + 8) + `IcmpCloseHandle`, all inside `tokio::task::spawn_blocking`.
    - Returns `Some(PingReply { rtt: RoundTripTime ms, ttl: Options.Ttl })` only when `Status == IP_SUCCESS` and the reply `Address` equals the target.
    - **Every per-address failure is `Ok(None)`**: zero replies, `IP_REQ_TIMED_OUT`, `IP_DEST_HOST_UNREACHABLE` and any other status. This is a Phase 0 ledger contract; an `Err` fails the whole scan.
    - Only a failed `IcmpCreateFile` or a panicked blocking task returns `Err(Io)`.
  - `arp_resolve`:
    - `SendARP(dest, 0, &mut [u32; 2], &mut 6)` in `spawn_blocking`, wrapped in `tokio::time::timeout(timeout)`.
    - `NO_ERROR` → `Some(mac)`; any other code or a timeout → `Ok(None)`.
  - `arp_table`: `read_neighbors(cached if_index)`. If no interface has been chosen yet, call `active_interface` first.
  - `tcp_probe`: `Ok(tcp_connect_probe(..).await)`.

- [ ] **Step 1: Write the failing tests** (`tests/windows.rs`)

```rust
#[tokio::test] async fn pings_loopback() {
    let r = WindowsPlatform::new().ping(ip("127.0.0.1"), ms(800)).await.unwrap().unwrap();
    assert!(r.rtt < ms(800));
}
#[tokio::test] async fn unanswered_ping_is_none_not_an_error() {
    // TEST-NET-1 is never on-link; the OS reports unreachable or a timeout.
    assert_eq!(WindowsPlatform::new().ping(ip("192.0.2.1"), ms(300)).await.unwrap(), None);
}
#[tokio::test] async fn tcp_probe_open_and_closed() { /* listener on 127.0.0.1 → Open; after drop → Closed */ }
#[tokio::test] async fn arp_resolve_never_errors() {
    let p = WindowsPlatform::new();
    assert!(p.arp_resolve(ip("192.0.2.1"), secs(3)).await.is_ok());
}
#[tokio::test] async fn interface_or_not_connected() {
    match WindowsPlatform::new().active_interface().await {
        Ok(i) => assert!((1..=32).contains(&i.prefix)),
        Err(e) => assert_eq!(e, PlatformError::NotConnected),
    }
}
#[tokio::test] #[ignore = "needs a real LAN; run manually"]
async fn real_lan_gateway_answers() {
    let p = WindowsPlatform::new(); let i = p.active_interface().await.unwrap();
    assert!(p.arp_resolve(i.gateway, secs(3)).await.unwrap().is_some());
}
```

- [ ] **Step 2:** `cargo test -p netlantern-platform --test windows` → FAIL.
- [ ] **Step 3:** Implement `icmp.rs`, `arp.rs` and `platform.rs`. Cache `if_index` in a `Mutex<Option<u32>>`.
- [ ] **Step 4:** Run `cargo test -p netlantern-platform` → PASS. Also run `cargo test -p netlantern-platform -- --ignored` on the author's network and record the result in the commit message body.
- [ ] **Step 5: Commit:** `feat(platform): WindowsPlatform with ICMP, SendARP and TCP probes`

---

### Task 4: `RealNetIo`: DNS over UDP, reverse DNS, HTTP

**Files:**
- Create: `crates/netlantern-platform/src/net/{mod.rs,dns.rs,http.rs}`
- Modify:
  - `Cargo.toml`: `hickory-proto`; `reqwest` with `rustls-tls`, no default features; tokio `net`, `time`
  - `lib.rs`, which re-exports `net::RealNetIo`
- Test: inline unit tests in `dns.rs`; `crates/netlantern-platform/tests/net.rs`

**Interfaces:**
- Produces:
  - `RealNetIo::new() -> Self`, implementing `NetIo`. Task 5 fills in the discovery methods; until then they return `Err(Unavailable { reason: "arrives in task 5" })`.
  - `pub(crate) async fn dns_query(server: SocketAddr, query: Message, timeout: Duration) -> Result<Message, PlatformError>`:
    - UDP from an ephemeral port.
    - Random id; responses with another id are ignored.
    - One retry.
  - `dns_resolve(server, name)`: A query to `server:53` → the A records; NXDOMAIN → `Ok(vec![])`; a timeout → `Err(Io)`.
  - `reverse_dns(server, ip)`: PTR query for `d.c.b.a.in-addr.arpa.` → the first PTR name without its trailing dot. No answer, NXDOMAIN or a timeout → `Ok(None)`.
  - `pub fn ptr_name(ip: Ipv4Addr) -> String`.
  - `http_get(url, follow_redirects)`:
    - reqwest with a 5 s total timeout.
    - Redirects: `Policy::none()` unless following, else `limited(5)`.
    - `body_prefix` = the first 16 KiB of the body, decoded lossily as UTF-8.
    - Errors → `Err(Io { message })`.
- DNS timeout default: **1 s** (a const in `net::dns`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn ptr_name_reverses_octets() { assert_eq!(ptr_name(ip("192.168.1.61")), "61.1.168.192.in-addr.arpa."); }
// tests/net.rs: a fake DNS server on 127.0.0.1:0 (UdpSocket) answering with hickory-built responses
#[tokio::test] async fn resolves_a_records_from_the_given_server() { /* fake answers 10.0.0.9 → vec![10.0.0.9] */ }
#[tokio::test] async fn reverse_dns_returns_name_without_trailing_dot() { /* fake PTR "echo-dot.lan." → Some("echo-dot.lan") */ }
#[tokio::test] async fn reverse_dns_silence_is_none() { /* server that never answers → Ok(None) within ~2 s */ }
#[tokio::test] async fn http_get_does_not_follow_redirects_when_asked() {
    /* axum or hand-rolled TcpListener answering 302 Location: /x → status 302 */
}
```
Add a public `RealNetIo::with_ports(dns: u16, netbios: u16) -> Self`. `new()` is `with_ports(53, 137)`. The integration tests use it to point `dns_resolve`, `reverse_dns` and (in Task 5) `netbios_status` at fake servers on loopback.

- [ ] **Step 2:** `cargo test -p netlantern-platform --test net` → FAIL.
- [ ] **Step 3:** Implement `dns.rs` and `http.rs` with hickory-proto `Message`, `Query` and `RecordType`.
- [ ] **Step 4:** `cargo test -p netlantern-platform` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(platform): RealNetIo DNS, reverse DNS and HTTP`

---

### Task 5: `RealNetIo` discovery: SSDP, NetBIOS, one-shot mDNS

**Ruling:** mDNS uses hand-built one-shot ("legacy unicast", RFC 6762 §6.7) queries sent from an ephemeral port. Responders answer by unicast to that port, so Net Lantern never binds 5353 and doesn't compete with Windows' own mDNS responder. That replaces the `mdns-sd` crate named in early notes, which binds 5353. Crate choice is left to the builder (spec §15).

**Files:**
- Create: `crates/netlantern-platform/src/net/{ssdp.rs,netbios.rs,mdns.rs}`
- Test: inline unit tests for the parsers; `tests/net.rs` (extend) with fake responders on loopback

**Interfaces:**
- Produces:
  - **SSDP:**
    - `pub fn msearch_request() -> &'static str`: `M-SEARCH * HTTP/1.1`, `HOST: 239.255.255.250:1900`, `MAN: "ssdp:discover"`, `MX: 2`, `ST: ssdp:all`, CRLF line endings, ending in a blank line.
    - `pub fn parse_ssdp_response(text: &str) -> Option<(String /*st*/, Option<String> /*server*/, Option<String> /*location*/)>`: needs a `HTTP/1.1 200` status line; header names are case-insensitive; no `ST` → `None`.
    - `ssdp_search(local, window)`:
      - Bind UDP `local:0`, set the multicast interface to `local` and TTL 2.
      - Send M-SEARCH to `239.255.255.250:1900` twice, 500 ms apart.
      - Collect replies until `window` ends.
      - Deduplicate on (ip, st).
  - **NetBIOS:**
    - `pub fn nbstat_request(txid: u16) -> [u8; 50]`:
      - Header: flags 0, qdcount 1.
      - Name: length 0x20 + `"CKAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"` + 0.
      - Type 0x0021, class 0x0001.
    - `pub fn parse_nbstat(buf: &[u8], txid: u16) -> Option<NetbiosInfo>`:
      - Check the txid.
      - Skip the 12-byte header, the encoded name, type, class, TTL and rdlength.
      - `num_names` = u8, then 18-byte entries: 15 bytes of name (trim trailing spaces), a suffix byte, and big-endian flags (`0x8000` = group).
      - `name` = the first unique (non-group) entry with suffix `0x00`.
      - `workgroup` = the first group entry with suffix `0x00`.
      - No unique `0x00` entry → `None`.
    - `netbios_status(ip, timeout)`: UDP to `ip:137` from an ephemeral port; a timeout → `Ok(None)`.
  - **mDNS:**
    - `mdns_browse(local, window)`:
      - From `local:0` to `224.0.0.251:5353`, send a PTR query for `_services._dns-sd._udp.local.`.
      - For each service type in the answers, send one PTR query (queries batched up to 10 questions per message).
      - Collect every response until `window` ends.
      - Build `MdnsService` records:
        - PTR → instance
        - SRV → target hostname
        - A → ip
        - TXT `fn`, `md`, `model` or `ty` → friendly_name and model
      - Records are matched across all received messages, and the source address is used when no A record arrives.
    - `pub fn services_from_messages(msgs: &[(Ipv4Addr, Message)]) -> Vec<MdnsService>`: pure, unit-tested.
    - `instance` is the instance label only (`"Office printer"`, not the full name). Labels are DNS-unescaped (`\032` → space).
  - **Errors:** socket bind or send failures → `Err(Io)`. Silence is an empty result, never an error.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn parses_ssdp_reply_case_insensitively() {
    let r = "HTTP/1.1 200 OK\r\nst: upnp:rootdevice\r\nSERVER: Hue/1.0 UPnP/1.0 IpBridge\r\nLocation: http://192.168.1.130/description.xml\r\n\r\n";
    assert_eq!(parse_ssdp_response(r), Some(("upnp:rootdevice".into(), Some("Hue/1.0 UPnP/1.0 IpBridge".into()), Some("http://192.168.1.130/description.xml".into()))));
    assert_eq!(parse_ssdp_response("NOTIFY * HTTP/1.1\r\nNT: x\r\n\r\n"), None);
}
#[test] fn nbstat_request_layout() {
    let p = nbstat_request(0xBEEF);
    assert_eq!(&p[0..2], &[0xBE, 0xEF]); assert_eq!(p[12], 0x20); assert_eq!(&p[13..15], b"CK"); assert_eq!(&p[46..50], &[0, 0x21, 0, 1]);
}
#[test] fn parses_nbstat_names() {
    let buf = nbstat_reply(0xBEEF, &[("DISKSTATION", 0x00, 0x0400), ("WORKGROUP", 0x00, 0x8400), ("DISKSTATION", 0x20, 0x0400)], [0x00,0x11,0x32,0xAB,0xCD,0x70]);
    assert_eq!(parse_nbstat(&buf, 0xBEEF), Some(NetbiosInfo { name: "DISKSTATION".into(), workgroup: Some("WORKGROUP".into()) }));
    assert_eq!(parse_nbstat(&buf, 0x0001), None);          // wrong txid
    assert_eq!(parse_nbstat(&buf[..20], 0xBEEF), None);    // truncated, no panic
}
#[test] fn mdns_records_combine_across_messages() {
    // msg A from .31: PTR _googlecast._tcp.local → "Google-Nest-Mini-3c1e._googlecast._tcp.local", SRV → "3c1e.local", TXT fn=Kitchen speaker md=Google Nest Mini
    // msg B from .31: A 3c1e.local → 192.168.1.31
    let s = services_from_messages(&[a, b]);
    assert_eq!(s, vec![MdnsService { ip: ip("192.168.1.31"), service_type: "_googlecast._tcp".into(), instance: "Google-Nest-Mini-3c1e".into(),
        hostname: Some("3c1e.local".into()), friendly_name: Some("Kitchen speaker".into()), model: Some("Google Nest Mini".into()) }]);
}
#[test] fn mdns_instance_labels_are_unescaped() { /* "Office\032printer" → "Office printer" */ }
```
(`nbstat_reply` is a test helper that builds a reply in the layout above.) In `tests/net.rs`: `netbios_status` against a fake UDP responder on `127.0.0.1` returns the name; the same call against a closed port returns `Ok(None)` within the timeout.

- [ ] **Step 2:** `cargo test -p netlantern-platform` → FAIL.
- [ ] **Step 3:** Implement the three modules and wire them into `RealNetIo`.
  - NetBIOS uses the port given to `with_ports`.
  - Malformed packets are dropped silently, never `unwrap`ped.
- [ ] **Step 4:** `cargo test -p netlantern-platform` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(platform): SSDP, NetBIOS node status and one-shot mDNS discovery`

---

### Task 6: OUI vendor table

**Files:**
- Create:
  - `crates/xtask/{Cargo.toml,src/main.rs}`: `publish = false`; deps `csv`
  - `crates/netlantern-core/data/oui.tsv` (generated, committed)
  - `crates/netlantern-core/src/identify/{mod.rs,oui.rs}`
- Modify: `crates/netlantern-core/src/lib.rs`; `README.md` adds a line naming the IEEE MA-L registry as the vendor data source and the refresh command
- Test: unit tests in `xtask/src/main.rs` and `identify/oui.rs`

**Interfaces:**
- Produces:
  - `cargo run -p xtask -- oui <path/to/oui.csv>`:
    - Reads the IEEE MA-L CSV (columns `Registry,Assignment,Organization Name,Organization Address`).
    - Writes `crates/netlantern-core/data/oui.tsv`: one `HEXHEX\tShort name` line per assignment, sorted by prefix, LF line endings.
    - The CSV is downloaded by hand from `https://standards-oui.ieee.org/oui/oui.csv`; the xtask never touches the network.
  - `pub fn short_vendor(org: &str) -> String` (in xtask). It applies, in order:
    1. **Alias table.** A case-insensitive prefix → fixed name:

       | prefix | name |
       |---|---|
       | `TP-LINK` | `TP-Link` |
       | `SAMSUNG ELECTRONICS` | `Samsung` |
       | `SONY INTERACTIVE` | `Sony Interactive` |
       | `AMAZON TECHNOLOGIES` | `Amazon` |
       | `SIGNIFY` | `Signify (Philips Hue)` |
       | `PHILIPS LIGHTING` | `Signify (Philips Hue)` |
       | `LCFC` | `Lenovo` |
       | `BEIJING XIAOMI` | `Xiaomi` |
       | `XIAOMI` | `Xiaomi` |
       | `HEWLETT PACKARD` | `HP` |
       | `INTEL CORPORATE` | `Intel` |
       | `RING LLC` | `Ring` |
       | `HON HAI` | `Foxconn` |
       | `GOOGLE` | `Google` |
       | `APPLE` | `Apple` |
       | `ESPRESSIF` | `Espressif` |
       | `SYNOLOGY` | `Synology` |

    2. **Otherwise strip suffixes, repeatedly:** trailing punctuation and spaces, then one of `Inc`, `Inc.`, `Incorporated`, `Co.`, `Co.,Ltd`, `Co., Ltd.`, `Ltd`, `Ltd.`, `LLC`, `Corporation`, `Corp.`, `GmbH`, `AG`, `S.A.`, `B.V.`, `Limited`, `Technologies`, `Technology`, `Electronics`, matched case-insensitively and only as whole trailing words.
    3. If the result is all caps and longer than 4 characters, convert it to title case.
  - `pub fn vendor(mac: MacAddr) -> Option<&'static str>` in `netlantern_core::identify::oui`:
    - `include_str!("../../data/oui.tsv")`, parsed once (`OnceLock<Vec<(u32, &'static str)>>`), then a binary search on the top 24 bits.
    - Locally administered (private) MACs → `None` without a lookup.

- [ ] **Step 1: Write the failing tests**

```rust
// xtask
assert_eq!(short_vendor("TP-LINK TECHNOLOGIES CO.,LTD."), "TP-Link");
assert_eq!(short_vendor("Synology Incorporated"), "Synology");
assert_eq!(short_vendor("Dell Inc."), "Dell");
assert_eq!(short_vendor("Espressif Inc."), "Espressif");
assert_eq!(short_vendor("Raspberry Pi Trading Ltd"), "Raspberry Pi Trading");
assert_eq!(short_vendor("AzureWave Technology Inc."), "AzureWave");
// core
assert_eq!(vendor(mac("00:11:32:AB:CD:70")), Some("Synology"));
assert_eq!(vendor(mac("F0:18:98:7C:3D:5E")), Some("Apple"));
assert_eq!(vendor(mac("24:6F:28:3A:9F:21")), Some("Espressif"));
assert_eq!(vendor(mac("DA:A1:19:5C:2E:01")), None);   // private
assert!(oui_table_is_sorted_and_large());              // > 30_000 rows, strictly ascending
```

- [ ] **Step 2:** `cargo test -p xtask -p netlantern-core` → FAIL.
- [ ] **Step 3:** Implement the xtask and `oui.rs`. Download `oui.csv` to the scratchpad (not the repo), run the xtask and commit `oui.tsv`. Then check the home-scenario MACs (Task 1 table) and fix any whose OUI isn't the intended vendor.
- [ ] **Step 4:** `cargo test --workspace` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(core): embedded IEEE OUI vendor table with xtask refresh`

---

### Task 7: Identification: names, UPnP descriptions, type classifier

**Files:**
- Create: `crates/netlantern-core/src/identify/{names.rs,upnp.rs,classify.rs}`
- Modify: `identify/mod.rs`, `model.rs` (add `DeviceType`)
- Test: unit tests inline in each file

**Interfaces:**
- Produces:
  - `enum DeviceType { Router, Laptop, Phone, Tablet, Printer, Tv, Speaker, Camera, Nas, Console, Iot, Unknown }`, serde `snake_case`.
  - `UpnpDescription { friendly_name: Option<String>, device_type: Option<String>, manufacturer: Option<String>, model_name: Option<String> }`.
  - `pub fn parse_upnp_description(xml: &str) -> UpnpDescription`:
    - Takes the text of the **first** `<friendlyName>`, `<deviceType>`, `<manufacturer>` and `<modelName>` elements.
    - Ignores namespace prefixes, trims whitespace, and decodes `&amp; &lt; &gt; &quot; &apos;`.
    - Plain string search; no XML crate.
  - `Signals { vendor: Option<String>, mdns: Vec<MdnsService>, ssdp: Vec<(SsdpReply, Option<UpnpDescription>)>, netbios: Option<NetbiosInfo>, rdns: Option<String>, open_ports: Vec<u16>, ttl: Option<u8>, private_mac: bool, is_gateway: bool, is_self: bool }`. `Default`, `Clone`.
  - `pub fn clean_name(raw: &str, ip: Ipv4Addr) -> Option<String>`:
    - Trim spaces and a trailing `.`, then strip one suffix from `.local`, `.lan`, `.home`, `.localdomain`, `.home.arpa`, `.internal`.
    - Reject if what remains still contains a `.`, which means an ISP or FQDN name.
    - Reject if it contains the IP as `a-b-c-d` or `a.b.c.d`.
    - Reject if it's empty.
  - `pub fn looks_generated(name: &str) -> bool`: a run of ≥ 8 hex digits (ignoring `-`), or a UUID.
  - `pub fn announced_name(s: &Signals, ip: Ipv4Addr) -> Option<String>`: the first acceptable name, in this order:
    1. mDNS `friendly_name`
    2. mDNS `hostname` (cleaned, not generated)
    3. mDNS `instance` (not generated)
    4. NetBIOS `name`
    5. UPnP `friendly_name` (trimmed only; it may contain dots or brackets)
    6. rDNS (cleaned, not generated)
  - `Classification { device_type: DeviceType, confidence: f32 }` and `pub fn classify(s: &Signals, hostname: Option<&str>) -> Classification`:
    - Sum the weights of every matching row below, per type.
    - The winning type is the highest sum; ties go to the earlier row in the table.
    - A top sum below **4** → `Unknown`.
    - `confidence = min(top, 10) / 10`.
    - Every match is case-insensitive, and "contains" is a substring test.

| Signal | Type +weight |
|---|---|
| `is_self` | laptop +10 |
| `is_gateway` | router +10 |
| SSDP st/deviceType contains `InternetGatewayDevice` | router +8 |
| mDNS `_ipp._tcp`, `_ipps._tcp`, `_printer._tcp`, `_pdl-datastream._tcp`; SSDP contains `Printer` | printer +8 |
| mDNS `_sonos._tcp`; SSDP contains `ZonePlayer` | speaker +8 |
| mDNS model contains `nest mini`, `nest audio`, `home mini`, `nest hub`, `homepod`, `echo` | speaker +8 |
| mDNS `_googlecast._tcp` | tv +5 |
| mDNS `_androidtvremote2._tcp`, `_amzn-wplay._tcp`; SSDP contains `dial-multiscreen`; server contains `Roku` | tv +7 |
| SSDP contains `MediaRenderer`; UPnP friendly_name contains `TV` as a word | tv +5 |
| mDNS `_raop._tcp`, `_spotify-connect._tcp` | speaker +4 |
| mDNS `_airplay._tcp` | tv +3 |
| mDNS `_hap._tcp`, `_hue._tcp`, `_matter._tcp`; server contains `IpBridge` | iot +6 |
| mDNS `_afpovertcp._tcp` | nas +4 |
| mDNS `_smb._tcp` | nas +3, laptop +2 |
| mDNS `_apple-mobdev2._tcp` or port 62078 open | phone +5, tablet +3 |
| mDNS `_companion-link._tcp` | phone +2, tablet +2, laptop +1 |
| mDNS `_workstation._tcp`, `_rdp._tcp` | laptop +5 |
| port 445 open | laptop +3, nas +2 |
| port 22 open | nas +1, laptop +1 |
| hostname contains `iphone`, `android`, `pixel`, `galaxy`, `oneplus`, `redmi` | phone +8 |
| hostname contains `ipad`, `tab-`, `galaxy-tab` | tablet +9 |
| hostname contains `macbook`, `desktop-`, `laptop-`, `-pc`, `imac`, `laptop` | laptop +6 |
| hostname contains `printer`, `epson`, `canon`, `brother`, `hp` followed by 6 hex digits | printer +6 |
| hostname contains `bravia`, `roku`, `firetv`, `chromecast`, or the word `tv` | tv +6 |
| hostname contains `echo`, `sonos`, `homepod`, `nest-mini`, `speaker` | speaker +6 |
| hostname contains `cam`, `camera`, `doorbell` | camera +5 |
| hostname contains `nas`, `synology`, `diskstation`, `qnap` | nas +7 |
| hostname contains `xbox`, `playstation`, `ps4`, `ps5`, `nintendo` | console +7 |
| hostname contains `esp_`, `esp-`, `tasmota`, `shelly`, `tuya`, `wled` | iot +6 |
| vendor `Synology`, `QNAP` | nas +7 |
| vendor `Sonos` | speaker +8 |
| vendor `Roku` | tv +8 |
| vendor `Ring`, `Hikvision`, `Dahua`, `Reolink`, `Wyze`, `Arlo` | camera +7 |
| vendor `Sony Interactive`, `Nintendo` | console +7 |
| vendor `Espressif`, `Tuya`, `Shelly`, `Signify (Philips Hue)`, `Itead`, `Wiz` | iot +5 |
| vendor `Brother`, `Epson`, `Seiko Epson`, `Canon`, `Lexmark`, `Xerox` | printer +6 |
| vendor `Intel`, `Dell`, `Lenovo`, `AzureWave`, `Liteon`, `Realtek`, `Raspberry Pi Trading` | laptop +4 |
| vendor `Amazon` | speaker +3, tv +2 |
| vendor `Google` | speaker +2, tv +2, phone +1 |
| vendor `Samsung` | phone +2, tv +2 |
| vendor `Apple` | phone +2, laptop +1, tablet +1 |
| vendor `HP` | printer +3, laptop +2 |
| vendor `Microsoft` | console +2, laptop +2 |
| `private_mac` | phone +2, tablet +1 |
| `ttl == 128` | laptop +2 |

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn upnp_description_fields() {
    let d = parse_upnp_description("<root><device><deviceType>urn:schemas-upnp-org:device:MediaRenderer:1</deviceType><friendlyName> [TV] Living Room </friendlyName><manufacturer>Samsung &amp; Co</manufacturer></device></root>");
    assert_eq!(d.friendly_name.as_deref(), Some("[TV] Living Room")); assert_eq!(d.manufacturer.as_deref(), Some("Samsung & Co"));
}
#[test] fn clean_name_rules() {
    let ip = ip("192.168.1.61");
    assert_eq!(clean_name("echo-dot.lan.", ip).as_deref(), Some("echo-dot"));
    assert_eq!(clean_name("DiskStation.local", ip).as_deref(), Some("DiskStation"));
    assert_eq!(clean_name("192-168-1-61.isp.example.net", ip), None);
    assert_eq!(clean_name("host.corp.example.com", ip), None);
    assert_eq!(clean_name("  ", ip), None);
}
#[test] fn generated_names() { assert!(looks_generated("3c1e9a7b-1f")); assert!(looks_generated("4f6c2a1e-9b7d-4c1a-8e2f-0a1b2c3d4e5f")); assert!(!looks_generated("Kitchen speaker")); }
#[test] fn announced_name_priority() { /* friendly_name beats netbios; netbios beats rdns; generated mDNS hostname skipped */ }
// one test per home-scenario device, built from its Task 1 signals + vendor:
#[test] fn classifies_home_devices() {
    for (signals, expected) in home_signals() { assert_eq!(classify(&signals, announced_name(&signals, ip0()).as_deref()).device_type, expected); }
}
#[test] fn weak_evidence_is_unknown() {
    let s = Signals { vendor: Some("Apple".into()), ..Default::default() };   // phone 2 < 4
    assert_eq!(classify(&s, None), Classification { device_type: DeviceType::Unknown, confidence: 0.2 });
}
#[test] fn gateway_always_router() { /* is_gateway + Samsung vendor → Router, confidence 1.0 */ }
```

- [ ] **Step 2:** `cargo test -p netlantern-core identify` → FAIL.
- [ ] **Step 3:** Implement the three files. Keep the rule table as a `const` slice of `(matcher, &[(DeviceType, u8)])` so new rows are data, not code.
- [ ] **Step 4:** `cargo test -p netlantern-core` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(core): device identification: announced names, UPnP descriptions, type classifier`

---

### Task 8: SQLite store: writer thread, migrations, networks and devices

**Files:**
- Create: `crates/netlantern-core/src/store/{mod.rs,db.rs,migrations.rs,networks.rs,devices.rs,scans.rs}`
- Modify:
  - `Cargo.toml`: `rusqlite` (`bundled`); `tracing`
  - `lib.rs`; `model.rs` (add `Device`, `DeviceId`, `NetworkId`, `Service`, `Sighting`)
  - Delete `devices.rs` (the in-memory `DeviceStore`) at the end of Task 10, not here.
- Test: `crates/netlantern-core/tests/store.rs`

**Interfaces:**
- Consumes: `identify::{oui::vendor, announced_name, classify, Signals}` (Tasks 6–7).
- Produces:
  - `Db::open(path: &Path) -> Result<Db, StoreError>` and `Db::open_in_memory() -> Result<Db, StoreError>`:
    - One dedicated `std::thread` owns the `rusqlite::Connection`.
    - `pub async fn call<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static) -> Result<T, StoreError>` sends closures over an mpsc channel and returns through a oneshot.
    - `Db` is `Clone` (it shares the sender).
    - Opening sets `PRAGMA journal_mode=WAL; foreign_keys=ON; busy_timeout=5000`.
  - **Migrations** (`PRAGMA user_version`):
    - `MIGRATIONS: &[&str]`; version 1 is the schema below.
    - `pub(crate) fn apply(conn: &mut Connection, path: Option<&Path>, list: &[&str]) -> Result<(), StoreError>`. Tests pass their own two-entry list.
    - If the file's version is greater than `list.len()` → `StoreError::NewerSchema { found, supported }`.
    - If 0 < version < `list.len()` and `path` is a file, copy the file to `<name>.bak` first (spec §10.1).
    - On open, scans left `running` are set to `failed` with error `"Net Lantern stopped during the scan."`
  - **Schema v1.** Times are unix milliseconds; MAC and IP are text:
    ```sql
    networks(id INTEGER PRIMARY KEY, gateway_mac TEXT NOT NULL, subnet TEXT NOT NULL, ssid_label TEXT,
             first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, scan_consent INTEGER NOT NULL DEFAULT 0,
             UNIQUE(gateway_mac, subnet))
    devices(id INTEGER PRIMARY KEY, network_id INTEGER NOT NULL REFERENCES networks(id), last_ip TEXT,
            hostname TEXT, vendor TEXT, type TEXT NOT NULL DEFAULT 'unknown', type_confidence REAL NOT NULL DEFAULT 0,
            name TEXT, starred INTEGER NOT NULL DEFAULT 0, is_new INTEGER NOT NULL DEFAULT 1,
            answers_ping INTEGER NOT NULL DEFAULT 0, rtt_ms INTEGER, ttl INTEGER, is_self INTEGER NOT NULL DEFAULT 0,
            is_gateway INTEGER NOT NULL DEFAULT 0, last_scan_id INTEGER, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL)
    device_macs(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, mac TEXT NOT NULL,
                private_mac INTEGER NOT NULL, first_seen INTEGER NOT NULL, last_seen INTEGER NOT NULL, PRIMARY KEY(device_id, mac))
    device_services(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, source TEXT NOT NULL,
                    value TEXT NOT NULL, last_seen INTEGER NOT NULL, PRIMARY KEY(device_id, source, value))
    device_ports(device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE, port INTEGER NOT NULL,
                 proto TEXT NOT NULL DEFAULT 'tcp', service TEXT, banner TEXT, seen_at INTEGER NOT NULL, PRIMARY KEY(device_id, port, proto))
    scans(id INTEGER PRIMARY KEY, network_id INTEGER REFERENCES networks(id), mode TEXT NOT NULL, state TEXT NOT NULL,
          error TEXT, isolated INTEGER NOT NULL DEFAULT 0, started_at INTEGER NOT NULL, finished_at INTEGER, devices_found INTEGER NOT NULL DEFAULT 0)
    settings(scope TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL, PRIMARY KEY(scope, key))
    ```
    Plus indexes on `devices(network_id)`, `device_macs(mac)` and `scans(network_id, id)`.
  - **Model types** (in `model.rs`):
    - `DeviceId(pub i64)`, `NetworkId(pub i64)`, both serde-transparent.
    - `Service { source: String /* mdns|ssdp|netbios|dns */, value: String }`.
    - `Sighting { ip, mac: Option<MacAddr>, answers_ping: bool, rtt_ms: Option<u32>, ttl: Option<u8>, is_self: bool, is_gateway: bool, open_ports: Vec<u16>, signals: Signals }`. Here `signals.vendor` is filled by the store from the OUI.
    - `Device` (Serialize, Deserialize, Clone, PartialEq):
      - `{ id: DeviceId, ip: Option<Ipv4Addr>, mac: Option<MacAddr>, private_mac: bool, vendor: Option<String>, hostname: Option<String>, name: Option<String>, display_name: String, #[serde(rename = "type")] device_type: DeviceType, type_confidence: f32, starred: bool, is_new: bool, online: bool, answers_ping: bool, rtt_ms: Option<u32>, is_self: bool, is_gateway: bool, first_seen: i64, last_seen: i64, services: Vec<Service>, open_ports: Vec<u16> }`
      - `mac` = the MAC seen most recently.
      - `display_name` = `name` ?? `hostname` ?? ip ?? `"Unknown device"`.
      - `online` = `last_scan_id` ≥ the network's newest scan in state `done`, or equals the network's running scan.
  - **Store** (`Store::new(db: Db)`; every method `async` and `Result<_, StoreError>`):
    - `network_for(gateway_mac: MacAddr, subnet: &str /* "192.168.1.0/24" */, now) -> NetworkId` (find or create; update `last_seen`)
    - `find_network(gateway_mac, subnet) -> Option<NetworkId>` (no insert)
    - `record_sighting(network: NetworkId, scan: ScanId, s: Sighting, now) -> Device`. Matching, in order:
      1. A device in this network owning `s.mac`.
      2. If `s.mac` is private and no device owns it: exactly one device in this network whose `hostname` equals (case-insensitively) the sighting's announced name **and** that device has no other MAC already seen in this `scan`. Attach the MAC to it.
      3. If `s.mac` is `None`: a MAC-less device with `last_ip = s.ip`.
      4. Otherwise insert a new device (`is_new = 1`).
      - Then union the stored services and ports with this sighting's.
      - Build `Signals` from the union plus the OUI `vendor`.
      - Set `hostname = announced_name`, then `type` and `type_confidence = classify`.
      - Set `last_ip`, `answers_ping`, `rtt_ms`, `ttl`, `is_self`, `is_gateway`, `last_scan_id`, `last_seen`.
      - Return the full `Device`.
      - **Services stored:**
        - `mdns`: each `service_type`, plus `friendly_name` or `instance`.
        - `ssdp`: `st`, plus UPnP `friendly_name` and `model_name`.
        - `netbios`: `name`.
        - `dns`: the rDNS name.
    - `list_devices(network) -> Vec<Device>`, sorted with online devices first, then by IP (devices with no IP last).
    - `get_device(network, id) -> Option<Device>`
    - `update_device(network, id, patch: DevicePatch) -> Option<Device>`, where `DevicePatch { name: Option<Option<String>>, starred: Option<bool>, is_new: Option<bool> }`:
      - The name is trimmed; empty → `NULL`.
      - Setting a non-empty name also sets `is_new = 0`.
    - `insert_scan(mode, now) -> ScanId`, `set_scan_network(id, network)`, `finish_scan(id, state, error: Option<String>, isolated: bool, devices_found, now)`
    - `scan(id) -> Option<ScanSnapshot>` and `latest_scan() -> Option<ScanSnapshot>`. Stored rows report `stage: merge` and `done = total = 0`.
    - `ScanSnapshot` moves from `scan.rs` to `model.rs`; Task 10 adds its new fields.
  - `Store` is `Clone` (it wraps a `Db`).
  - `StoreError` (thiserror): `Sqlite(String)`, `NewerSchema { found: u32, supported: u32 }`, `Closed`.
  - Re-exported from `netlantern_core`: `Db`, `Store`, `StoreError`, `DevicePatch`, `Device`, `DeviceId`, `NetworkId`, `DeviceType`, `Service`, `Sighting`, `Signals`.

- [ ] **Step 1: Write the failing tests** (`tests/store.rs`, in-memory unless stated)

```rust
#[tokio::test] async fn first_sighting_creates_a_new_identified_device() {
    let d = store.record_sighting(net, scan1, sighting("192.168.1.70", "00:11:32:AB:CD:70").netbios("DISKSTATION"), t0).await.unwrap();
    assert_eq!((d.vendor.as_deref(), d.hostname.as_deref(), d.device_type, d.is_new), (Some("Synology"), Some("DISKSTATION"), DeviceType::Nas, true));
    assert_eq!(d.display_name, "DISKSTATION");
}
#[tokio::test] async fn name_follows_the_mac_to_a_new_ip_and_old_ip_owner_goes_offline() {
    let a = store.record_sighting(net, scan1, sighting(".50", MAC_A), t0).await.unwrap();
    store.update_device(net, a.id, patch_name("Desk plug")).await.unwrap();
    store.finish_scan(scan1, Done, None, false, 1, t0).await.unwrap();
    let b = store.record_sighting(net, scan2, sighting(".50", MAC_B), t1).await.unwrap();   // B took A's old IP
    let a2 = store.record_sighting(net, scan2, sighting(".51", MAC_A), t1).await.unwrap();
    store.finish_scan(scan2, Done, None, false, 2, t1).await.unwrap();
    assert_eq!((a2.id, a2.name.as_deref()), (a.id, Some("Desk plug")));
    assert_eq!(store.list_devices(net).await.unwrap().len(), 2);
    assert_ne!(b.id, a.id);
}
#[tokio::test] async fn private_mac_rotation_keeps_the_device() {
    let p = store.record_sighting(net, scan1, sighting(".44", "DA:A1:19:5C:2E:01").rdns("Pixel-7.lan"), t0).await.unwrap();
    let again = store.record_sighting(net, scan2, sighting(".45", "DE:11:22:33:44:55").rdns("Pixel-7.lan"), t1).await.unwrap();
    assert_eq!(again.id, p.id);
    assert_eq!(store.list_devices(net).await.unwrap().len(), 1);
}
#[tokio::test] async fn two_phones_with_the_same_name_in_one_scan_stay_apart() {
    let a = store.record_sighting(net, scan1, sighting(".44", "DA:00:00:00:00:01").mdns_host("iPhone.local"), t0).await.unwrap();
    let b = store.record_sighting(net, scan1, sighting(".45", "DA:00:00:00:00:02").mdns_host("iPhone.local"), t0).await.unwrap();
    assert_ne!(a.id, b.id);
}
#[tokio::test] async fn devices_are_scoped_per_network() {
    let home = store.network_for(GW_HOME, "192.168.1.0/24", t0).await.unwrap();
    let cafe = store.network_for(GW_CAFE, "10.0.0.0/24", t0).await.unwrap();
    store.record_sighting(home, scan1, sighting("192.168.1.70", MAC_A), t0).await.unwrap();
    assert!(store.list_devices(cafe).await.unwrap().is_empty());
    assert_eq!(store.network_for(GW_HOME, "192.168.1.0/24", t1).await.unwrap(), home);
}
#[tokio::test] async fn clearing_the_name_returns_to_the_default() { /* name "" → display_name == hostname */ }
#[tokio::test] async fn naming_clears_new() { /* is_new false after a non-empty name */ }
#[tokio::test] async fn refuses_a_newer_schema() {
    // file DB in a temp dir: set PRAGMA user_version = 99, reopen → Err(NewerSchema { found: 99, .. })
}
#[tokio::test] async fn interrupted_scans_are_marked_failed_on_open() { /* insert running scan, reopen file → state failed */ }
#[test] fn upgrade_backs_up_first() {
    // file DB: apply(&mut c, Some(p), &LIST[..1]); reopen; apply(&mut c, Some(p), &LIST) → "<p>.bak" exists, user_version == 2
    // fresh file: apply(.., &LIST) → no .bak
}
```
(`sighting(..)` is a test builder that takes an IP or `".n"` shorthand plus a MAC, with chainable `.netbios() .rdns() .mdns_host()`.)

- [ ] **Step 2:** `cargo test -p netlantern-core --test store` → FAIL.
- [ ] **Step 3:** Implement `db.rs`, `migrations.rs`, `networks.rs`, `devices.rs` and `scans.rs`.
  - `record_sighting` runs inside **one** `db.call` transaction.
  - Use `std::time::SystemTime` for `now` only at the callers; the store takes `now: i64`.
- [ ] **Step 4:** `cargo test -p netlantern-core` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(core): SQLite store with migrations, per-network devices and MAC identity`

---

### Task 9: Discovery orchestration

**Files:**
- Create: `crates/netlantern-core/src/discover.rs`
- Test: `crates/netlantern-core/tests/discover.rs`

**Interfaces:**
- Consumes: the `NetIo` discovery methods (Task 1), `parse_upnp_description` (Task 7).
- Produces:
  - `DiscoverConfig { window: Duration /* 3 s */, netbios_timeout: Duration /* 1 s */, description_timeout: Duration /* 2 s */, concurrency: usize /* 32 */ }`, with `Default`.
  - `pub async fn discover(netio: &dyn NetIo, iface: &Interface, present: &[Ipv4Addr], cfg: &DiscoverConfig) -> HashMap<Ipv4Addr, Signals>`. It runs **concurrently**:
    1. `mdns_browse(iface.ip, window)`
    2. `ssdp_search(iface.ip, window)`
    3. `netbios_status(ip, netbios_timeout)` for each address in `present` except `iface.ip`
    4. `reverse_dns(iface.gateway, ip)` for each address in `present`
    - Steps 3 and 4 use `buffer_unordered(concurrency)`.
    - Results are kept only for IPs in `present`.
    - After SSDP: for each distinct `location` whose URL is `http://<host>[:port]/…` with `host` == the replying IP, and that IP is private (RFC 1918), call `http_get(location, false)` within `description_timeout`.
    - If the status is 200, attach `parse_upnp_description(body_prefix)`.
    - Any `Err` from any call is logged once at `debug` with the method name and skipped. **`discover` itself never fails.**
  - Only the `mdns`, `ssdp`, `netbios` and `rdns` fields of each `Signals` are filled; the scan fills the rest.

- [ ] **Step 1: Write the failing tests** (paused clock)

```rust
#[tokio::test(start_paused = true)] async fn gathers_home_identities_within_the_window() {
    let start = tokio::time::Instant::now();
    let found = discover(&home, &iface, &present_home(), &DiscoverConfig::default()).await;
    assert!(start.elapsed() <= secs(3) + secs(2));
    assert_eq!(found[&ip("192.168.1.70")].netbios.as_ref().unwrap().name, "DISKSTATION");
    assert_eq!(found[&ip("192.168.1.52")].ssdp[0].1.as_ref().unwrap().friendly_name.as_deref(), Some("[TV] Living Room"));
    assert_eq!(found[&ip("192.168.1.61")].rdns.as_deref(), Some("echo-dot.lan"));
}
#[tokio::test(start_paused = true)] async fn failing_netio_yields_empty_signals_not_an_error() {
    let found = discover(&FailingNetIo, &iface, &present_home(), &DiscoverConfig::default()).await;
    assert!(found.values().all(|s| s.mdns.is_empty() && s.netbios.is_none()));
}
#[tokio::test(start_paused = true)] async fn never_fetches_a_location_on_another_host() {
    // RecordingNetIo: ssdp reply from .52 with location http://203.0.113.9/x.xml → http_get never called
}
```

- [ ] **Step 2:** `cargo test -p netlantern-core --test discover` → FAIL.
- [ ] **Step 3:** Implement `discover.rs` (`tokio::join!` over the four branches).
- [ ] **Step 4:** `cargo test -p netlantern-core` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(core): discovery orchestration over mDNS, SSDP, NetBIOS and reverse DNS`

---

### Task 10: Quick-scan pipeline v2

**Ruling:**
- Spec §6.2 step 2 probes TCP ports "in parallel" on every address to catch ping-silent hosts.
- On-link TCP needs ARP first, so any host TCP could reach already has an ARP entry. TCP probing every address therefore finds nobody new and costs 1,270 connects on a /24.
- So TCP probes run only on present devices, and only for identification.
- Task 16 records this change in spec §6.2.

**Files:**
- Modify: `crates/netlantern-core/src/{scan.rs,model.rs,events.rs,lib.rs}`; delete `devices.rs` (`DeviceStore`)
- Test: `crates/netlantern-core/tests/scan_sim.rs` (rewrite onto the store; keep every Phase 0 behaviour test that still applies)

**Interfaces:**
- Consumes: `Store` (Task 8), `discover` (Task 9), `Platform`/`NetIo`, `EventBus`.
- Produces:
  - `ScanStage { Sweep, Arp, Ports, Discover, Merge }`, serde snake_case.
  - `ScanConfig` gains these fields (with `Default`):
    - `tcp_ports: Vec<u16>`: `[80, 443, 445, 22, 62078]`
    - `tcp_timeout`: 800 ms
    - `tcp_concurrency`: 64
    - `arp_resolve_concurrency`: 32
    - `arp_resolve_timeout`: 3 s
    - `arp_poll_interval`: 1 s
    - `discover: DiscoverConfig`
  - `ScanSnapshot` gains `error: Option<String>` and `isolated: bool`.
  - `EventKind::ScanDevice { scan_id, device: Device }`, where the payload is now the persisted `Device`.
  - `EventKind::ScanDone` gains `error: Option<String>` and `isolated: bool`.
  - `ScanManager::new(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>, bus: Arc<EventBus>, store: Store, config: ScanConfig)`
  - `ScanManager::start(mode) -> Result<StartOutcome, CoreError>`: unchanged contract. Scan ids now come from `store.insert_scan`.
  - `ScanManager::snapshot(id)`: from memory if this process ran the scan, else from the store row.
  - `ScanManager::latest() -> Option<ScanSnapshot>`
  - `ScanManager::current_network() -> Option<NetworkId>`:
    - Returns the network of the last scan if the interface `(ip, prefix, gateway)` still matches.
    - Otherwise identifies passively: an `arp_table` entry for the gateway → `store.find_network`. Nothing is sent and nothing is created.
  - `CoreError` gains `Store(StoreError)`.

**Algorithm** (`quick_scan`). Each stage start publishes `scan.progress` and logs its duration at `info` (spec §11 "scan time … logged"):

1. **Network.** Look up the gateway MAC: first in `arp_table`, else `arp_resolve(gateway, arp_resolve_timeout)`.
   - If neither gives one, fail the scan with the error `"Your router at {gateway} didn't answer, so this network can't be identified."`
   - Otherwise `network_for(mac, subnet)`, then `set_scan_network`.
2. **Sweep.**
   - Pings run as in Phase 0.
   - Concurrently, every `arp_poll_interval`, read `arp_table`. Each address that has **replied** and has a present entry (or is `iface.ip`) and hasn't been published yet is recorded with `record_sighting` (no signals yet), and a `scan.device` is published.
   - This is how the first devices appear within 3 s.
3. **Arp.**
   - `arp_resolve` (concurrency 32) only for targets with **no table entry at all**.
   - Then read the table again.
   - Present = replied, or a present entry (as in Phase 0).
4. **Ports.** `tcp_probe` each present address (except `iface.ip`) × `tcp_ports`, concurrency 64. Keep the `Open` ports.
5. **Discover.** `discover(netio, iface, present, cfg.discover)`.
6. **Merge.**
   - For every present address, build a `Sighting` (signals from step 5, ports from step 4) and `record_sighting` it.
   - Publish `scan.device` with the returned `Device`. A device published in step 2 is published again, enriched.
   - `devices_found` = the number of distinct device ids.
7. **Isolation.** If the present addresses other than `iface.ip` are exactly `{gateway}`, set `isolated = true`.
8. **Finish.**
   - `finish_scan(...)` and `scan.done` happen in the Phase 0 critical section (running → snapshots → bus).
   - The state is `failed` with `error = Some(message)` when a stage returned `Err` or panicked. A panic's message is `"Internal error during the scan."`
   - Log the failure at `warn` with the error.

- [ ] **Step 1: Write the failing tests** (`tests/scan_sim.rs`, paused clock, in-memory store, home scenario unless stated)

```rust
#[tokio::test(start_paused = true)] async fn home_scan_identifies_every_device_within_targets() {
    let start = Instant::now();
    let _id = started(rig.mgr.start(ScanMode::Quick).await.unwrap());
    let timed = collect_until_done_timed(&mut rig.rx, start).await;        // Vec<(Event, Duration since start)>
    let events: Vec<Event> = timed.iter().map(|(e, _)| e.clone()).collect();
    assert!(start.elapsed() <= secs(20));                                  // §2.1 ≤ 20 s
    let first_device_at = timed.iter().find(|(e, _)| matches!(e.kind, EventKind::ScanDevice { .. })).unwrap().1;
    assert!(first_device_at <= secs(3));                                   // §2.1 first devices within 3 s
    assert!(matches!(events.last().unwrap().kind, EventKind::ScanDone { devices_found: 13, state: ScanState::Done, isolated: false, .. }));
    let net = rig.mgr.current_network().await.unwrap();
    let list = rig.store.list_devices(net).await.unwrap();
    assert_eq!(list.len(), 13);
    let typed = list.iter().filter(|d| d.device_type != DeviceType::Unknown).count();
    assert!(typed * 10 >= list.len() * 7);                                  // §2.1 ≥ 70%
    assert!(list.iter().filter(|d| !d.private_mac).all(|d| d.vendor.is_some())); // §2.1 100% vendors
    assert_eq!(by_ip(&list, "192.168.1.52").display_name, "[TV] Living Room");
    assert_eq!(by_ip(&list, "192.168.1.88").answers_ping, false);
    assert!(by_ip(&list, "192.168.1.57").open_ports.contains(&62078));
}
#[tokio::test(start_paused = true)] async fn rescan_keeps_names_and_marks_missing_devices_offline() { /* scan, rename .70, remove .95 from a cloned scenario, rescan → .70 keeps name; .95 online == false */ }
#[tokio::test(start_paused = true)] async fn isolated_network_is_flagged() { /* scenarios/isolated.json → ScanDone { isolated: true, devices_found: 2 } */ }
#[tokio::test(start_paused = true)] async fn discovery_failure_does_not_fail_the_scan() { /* home Platform + FailingNetIo → Done, 13 devices, vendors present */ }
#[tokio::test(start_paused = true)] async fn platform_error_mid_sweep_fails_with_a_reason() {
    /* Platform whose ping errs on .40 → ScanDone { state: Failed, error: Some(msg) }; snapshot.error == Some(msg); next start() works */
}
#[tokio::test(start_paused = true)] async fn silent_gateway_fails_with_the_router_message() { /* gateway arp none */ }
#[tokio::test(start_paused = true)] async fn arp_resolve_only_for_addresses_without_entries() { /* CountingPlatform: table has entries for all but 3 targets → arp_resolve called 3 times + gateway lookup 0 */ }
#[tokio::test(start_paused = true)] async fn already_running_and_offline_and_deep_behave_as_in_phase_0() { /* keep the three Phase 0 asserts */ }
```

- [ ] **Step 2:** `cargo test -p netlantern-core --test scan_sim` → FAIL.
- [ ] **Step 3:** Implement. Keep Phase 0's lock order, `catch_unwind` and finish logic. Remove `DeviceStore` and `DeviceSummary` (a `Sighting` replaces the latter). Update the inline unit tests of `merge` to the new types.
- [ ] **Step 4:** `cargo test -p netlantern-core` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(core): quick scan v2: streamed sightings, ARP resolve, port probes, discovery, persistence`

---

### Task 11: Server: devices API, latest scan, refusal log, auth bounds

**Files:**
- Modify:
  - `crates/netlantern-server/src/{state.rs,routes.rs,guard.rs,auth.rs,error.rs}`
  - `Cargo.toml` (`tracing`)
- Test: `crates/netlantern-server/tests/{api.rs,guard.rs}` (extend and update for the new state)

**Interfaces:**
- Consumes: `ScanManager` v2, `Store`, `Device`, `DevicePatch` (Tasks 8, 10).
- Produces:
  - `AppState { platform, netio: Arc<dyn NetIo>, bus, scans, store: Store, auth, policy, refusals: Arc<RefusalLog> }`
  - `AppState::new(platform: Arc<dyn Platform>, netio: Arc<dyn NetIo>, store: Store, scan: ScanConfig, policy: AccessPolicy) -> Self`
  - Routes (all behind the guard):
    - `GET /api/devices` → `[Device]` for `scans.current_network()`; `[]` when it is `None`.
    - `GET /api/devices/{id}` → `Device`, or 404 `not_found` (also when the device belongs to another network).
    - `PATCH /api/devices/{id}`, body `{"name"?: string|null, "starred"?: bool, "is_new"?: false}`:
      - → 200 `Device`.
      - A name longer than 64 characters (after trimming) → 400 `bad_request` "Names can be at most 64 characters."
      - Unknown fields → 400 (`deny_unknown_fields`).
      - `is_new: true` → 400.
    - `GET /api/scans/latest` → `ScanSnapshot`, or 404 when there has never been a scan. Registered before `/api/scans/{id}`.
    - `scan.done` and `ScanSnapshot` carry `error` and `isolated`.
  - `ApiError::store(err)` → 500 code `storage_error`, hint "Restart Net Lantern. If this keeps happening, copy the log into a bug report." `CoreError::Store` maps to it.
  - `RefusalLog`:
    - `first_time(&self, kind: &'static str, value: &str) -> bool`.
    - Remembers up to 256 distinct `(kind, value)` pairs. After that it logs one `"further refusals are not logged"` line, then stays quiet.
    - The guard logs `tracing::warn!(kind, value = %truncate(value, 100), "refused request")` for foreign Host and foreign Origin, the first time only (spec §10).
  - `Auth` bounds (`AuthLimits { token_ttl: 5 min, max_tokens: 8, max_sessions: 32 }`, `Auth::with_limits`):
    - An expired token doesn't redeem.
    - Issuing past `max_tokens` drops the oldest token.
    - Creating past `max_sessions` drops the oldest session.

- [ ] **Step 1: Write the failing tests**

```rust
// api.rs (home sim + in-memory store)
assert!(get_devices(&c).await.is_empty());                       // before any scan
run_scan_to_done(&c).await;
let list = get_devices(&c).await; assert_eq!(list.len(), 13);
let nas = list.iter().find(|d| d["ip"] == "192.168.1.70").unwrap();
assert_eq!(nas["type"], "nas"); assert_eq!(nas["vendor"], "Synology");
let id = nas["id"].as_i64().unwrap();
let r = patch_device(&c, id, json!({"name": "  Backup box  ", "starred": true})).await;
assert_eq!(r.status(), 200); assert_eq!(r.json()["display_name"], "Backup box"); assert_eq!(r.json()["is_new"], false);
assert_eq!(patch_device(&c, id, json!({"name": "x".repeat(65)})).await.status(), 400);
assert_eq!(patch_device(&c, id, json!({"colour": "red"})).await.status(), 400);
assert_eq!(patch_device(&c, 99999, json!({"starred": true})).await.status(), 404);
assert_eq!(patch_device(&c, id, json!({"name": null})).await.json()["display_name"], "DiskStation");
assert_eq!(get_json(&c, "/api/scans/latest").await["state"], "done");
// guard.rs
let auth = Auth::with_limits(AuthLimits { token_ttl: ms(50), max_tokens: 2, max_sessions: 2 });
let t = auth.issue_launch_token(); sleep(ms(100)).await; assert!(auth.redeem(&t).is_none());
let (a, b, c3) = (auth.issue_launch_token(), auth.issue_launch_token(), auth.issue_launch_token());
assert!(auth.redeem(&a).is_none() && auth.redeem(&b).is_some() && auth.redeem(&c3).is_some());
let log = RefusalLog::default(); assert!(log.first_time("host", "evil:1")); assert!(!log.first_time("host", "evil:1"));
```

- [ ] **Step 2:** `cargo test -p netlantern-server` → FAIL.
- [ ] **Step 3:** Implement. Tests and the exe pass the same `Arc<SimNetwork>` as both `platform` and `netio`.
- [ ] **Step 4:** `cargo test --workspace` → PASS; clippy clean.
- [ ] **Step 5: Commit:** `feat(server): device API with rename/star, latest scan, refusal log, bounded tokens and sessions`

---

### Task 12: Executable: real mode, data folder, `--scan-once`

**Files:**
- Modify: `crates/netlantern/src/{main.rs,cli.rs}`, `crates/netlantern/tests/exe.rs`, `Cargo.toml` (netlantern-core store access)
- Test: `cli.rs` unit tests; `tests/exe.rs`

**Interfaces:**
- Consumes: `WindowsPlatform`, `RealNetIo`, `Db`, `Store`, `AppState::new` (Tasks 3–5, 8, 11).
- Produces. CLI additions:
  - `--data-dir <DIR>`
  - `--scan-once`. It may be combined with `--demo`; the exe test relies on that.
- Behaviour:
  - **Startup order (real mode):** single-instance pipe first (a second launch never opens the database, spec §5), then the store, then bind.
  - **No `--demo`:**
    - On Windows: `WindowsPlatform::new()` + `RealNetIo::new()`.
      - The database is at `<data-dir or %LOCALAPPDATA%\NetLantern>\netlantern.db`; create the folder.
      - `StoreError::NewerSchema` → stderr `This data was written by a newer Net Lantern. Update Net Lantern, or start it with --data-dir <folder> to use another folder.`, exit 1.
    - Not on Windows: stderr `Real network scanning only works on Windows so far. Run netlantern --demo instead.`, exit 2.
  - **`--demo`:** `Db::open_in_memory()`, so demo data is never written to disk (`--data-dir` is ignored with a warning).
  - **`--scan-once`:**
    - Skips the single-instance pipe and the server.
    - Uses an in-memory store, so the user's data is not touched. Runs one quick scan.
    - Prints a tab-separated table to stdout:
      - Header line: `ip\tmac\tvendor\tname\ttype\tping`
      - One row per device, sorted by IP. `ping` = `rtt ms`, or `arp-only`.
      - Then the line `Found <n> devices in <s.s> s`.
    - Exit 0 on `done`; on `failed`, exit 1 with the scan error on stderr.
    - Stage durations go to stderr through `tracing`.
    - This is the tool for the OS-ARP coverage check and the scan-time target.
  - The `Real network scanning arrives in phase 1` message is removed.

- [ ] **Step 1: Write the failing tests**

```rust
// cli.rs
assert!(Cli::parse_from(["netlantern", "--scan-once"]).scan_once);
assert_eq!(Cli::parse_from(["netlantern", "--data-dir", "D:\\x"]).data_dir, Some("D:\\x".into()));
// tests/exe.rs
let out = run_exe(&["--demo", "--scan-once"]);
assert!(out.status.success());
let stdout = String::from_utf8(out.stdout).unwrap();
assert!(stdout.starts_with("ip\tmac\tvendor\tname\ttype\tping\n"));
assert!(stdout.contains("192.168.1.70\t00:11:32:AB:CD:70\tSynology\tDiskStation\tnas\t"));
assert!(stdout.lines().last().unwrap().starts_with("Found 13 devices in "));
// newer schema
let dir = tempdir(); make_db_with_user_version(&dir.join("netlantern.db"), 99);
let out = run_exe(&["--data-dir", dir.to_str().unwrap(), "--no-browser"]);
assert_eq!(out.status.code(), Some(1)); assert!(stderr(&out).contains("newer Net Lantern"));
```

- [ ] **Step 2:** `cargo test -p netlantern` → FAIL.
- [ ] **Step 3:** Implement.
  - `main` builds `(Arc<dyn Platform>, Arc<dyn NetIo>, Store)` from the flags, then follows the Phase 0 flow.
  - The `--demo --scan-once` run uses real time, which takes about 10 s; that's acceptable for one exe test.
- [ ] **Step 4:** Run `cargo test -p netlantern` → PASS. Manual: `cargo run -p netlantern -- --scan-once` on the author's network prints real devices, and the commit body records the device count and seconds.
- [ ] **Step 5: Commit:** `feat: real-network mode, data folder and --scan-once`

---

### Task 13: UI: Devices table, filters, search, star; scan stages on the Dashboard

**Files:**
- Modify: `ui/src/api/queries.ts`, `ui/src/state/live.ts`, `ui/src/pages/{Devices.tsx,Dashboard.tsx}`
- Create: `ui/src/lib/{deviceType.ts,time.ts}`, `ui/src/api/mutations.ts`
- Test: `ui/src/pages/{Devices.test.tsx,Dashboard.test.tsx}`, `ui/src/lib/time.test.ts`, `ui/src/state/live.test.ts` (extend)

**Interfaces:**
- Consumes: the Task 11 API.
- Produces:
  - `queries.ts`:
    - `Device` gains `id: number; vendor: string|null; hostname: string|null; name: string|null; display_name: string; type: DeviceType; type_confidence: number; starred: boolean; is_new: boolean; online: boolean; first_seen: number; last_seen: number; services: {source: string; value: string}[]; open_ports: number[]`.
    - `useDevice(id)` (key `["device", id]`).
    - `ScanStatus` gains `error: string|null; isolated: boolean`.
  - `mutations.ts`: `updateDevice(id: number, patch: {name?: string|null; starred?: boolean; is_new?: false}): Promise<Device>` (throws `ApiRequestError`); `useUpdateDevice()`:
    - A TanStack mutation that optimistically updates `["devices"]` and `["device", id]`.
    - It rolls back on error and sets both caches from the server's reply.
  - `lib/deviceType.ts`: `DEVICE_TYPE_LABEL` — router "Router", laptop "Computer", phone "Phone", tablet "Tablet", printer "Printer", tv "TV", speaker "Speaker", camera "Camera", nas "NAS", console "Game console", iot "Smart device", unknown "Unknown".
  - `lib/time.ts`: `formatAgo(ms: number, now: number): string`:
    - < 60 s → "just now"
    - < 60 min → "`n` min ago"
    - < 24 h → "`n` h ago"
    - otherwise "`n` day(s) ago"
  - `live.ts`: `LiveScan` gains `error: string|null; isolated: boolean`, both set from `scan.done` and from `setScan`.
  - **Devices page:**
    - Search box (label "Search devices"). It matches display name, IP, MAC, vendor and type label, case-insensitively.
    - Filter buttons All, Online, Starred, New (`aria-pressed`), each with its count.
    - Table columns: star, Name, IP, MAC, Type, Latency, Status.
      - **Star:** a button with `aria-label` "Star {display_name}" or "Unstar {display_name}", and `aria-pressed`.
      - **Name:** display_name, plus a "New" badge if `is_new`, the existing "This device" and "Router" badges, and vendor on a second muted line.
      - **MAC:** "Private address" when private.
      - **Type:** the label.
      - **Status:**
        - "Online"
        - "Online · doesn't answer ping"
        - "Offline · last seen {formatAgo}"
    - Rows are sorted online-first, then by IP. Offline rows are muted, with the text still readable.
    - Empty search → "No devices match “{q}”."
    - A failed star mutation shows the server's message in a `role="alert"` line above the table.
    - The name cell is a `<button>` labelled with the display name. Task 14 makes it open the details panel.
    - At phone width (< 640 px) the IP, MAC and Latency columns hide; the IP moves under the name.
  - **Dashboard:**
    - `STAGE_LABEL` adds ports "Checking common ports" and discover "Asking devices their names", and changes merge to "Identifying devices".
    - While running: "{n} found so far" after the stage text.
    - Failed: "The scan failed: {error}".
    - Done and isolated: also show "This network appears to isolate devices — only the router answered, so other devices can't be seen from here."
    - The "{n} devices found" text stays as it is (the e2e test depends on it).

- [ ] **Step 1: Write the failing tests**

```ts
it("filters by search and by Starred", async () => {
  mockFetch({ "GET /api/devices": [200, [dev({ id: 1, display_name: "DiskStation", vendor: "Synology", starred: true }), dev({ id: 2, display_name: "Pixel-7", ip: "192.168.1.44" })]] });
  renderDevices();
  await user.type(await screen.findByLabelText("Search devices"), "synol");
  expect(screen.getAllByRole("row")).toHaveLength(2);           // header + 1
  await user.clear(screen.getByLabelText("Search devices"));
  await user.click(screen.getByRole("button", { name: /Starred/ }));
  expect(screen.getByText("DiskStation")).toBeInTheDocument(); expect(screen.queryByText("Pixel-7")).toBeNull();
});
it("stars a device optimistically and rolls back on error", async () => {
  mockFetch({ "GET /api/devices": [200, [dev({ id: 2, display_name: "Pixel-7" })]], "PATCH /api/devices/2": [500, { code: "storage_error", message: "x", hint: null }] });
  renderDevices();
  await user.click(await screen.findByRole("button", { name: "Star Pixel-7" }));
  expect(await screen.findByRole("button", { name: "Star Pixel-7" })).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByRole("alert")).toHaveTextContent("x");
});
it("shows offline devices with last seen", async () => { /* online:false, last_seen now-5min → "Offline · last seen 5 min ago" */ });
it("formatAgo boundaries", () => { expect(formatAgo(0, 59_000)).toBe("just now"); expect(formatAgo(0, 60_000)).toBe("1 min ago"); expect(formatAgo(0, 86_400_000)).toBe("1 day ago"); });
it("dashboard shows isolation and failure reasons", () => { /* setScan done+isolated → isolation text; failed+error → "The scan failed: …" */ });
```

- [ ] **Step 2:** `npm test` in `ui/` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `npm test` → PASS; `npm run lint && npm run build` succeed.
- [ ] **Step 5: Commit:** `feat(ui): devices table with search, filters and stars; scan stages, isolation and failure on the dashboard`

---

### Task 14: UI: device details panel (rename, mark as seen)

**Files:**
- Create: `ui/src/components/DeviceDetails.tsx`
- Modify: `ui/src/pages/Devices.tsx`
- Test: `ui/src/components/DeviceDetails.test.tsx`

**Interfaces:**
- Consumes: `useDevice`, `useUpdateDevice`, `DEVICE_TYPE_LABEL`, `formatAgo` (Task 13).
- Produces:
  - Activating a row's name button (click, or Enter) opens `<aside aria-label="Device details">`:
    - **Desktop:** to the right of the table.
    - **Below 760 px:** full width above the table, with a "Back to list" button.
    - The selected id lives in the URL as `?device=<id>`, so reload and back work.
  - Contents:
    - The display name as a heading.
    - Type label with confidence: "Phone · fairly sure" for ≥ 0.7, "Phone · best guess" for < 0.7, and just "Unknown" when unknown.
    - Vendor, or "Private address — vendor hidden" for a private MAC.
    - IP, MAC, latency, status, First seen and Last seen (`formatAgo`).
    - Open ports as a comma list, or "none of the common ports".
    - "Announced as" listing each service as `{source}: {value}`, or "Nothing announced".
  - Rename form:
    - Input labelled "Name" with a placeholder of the default name (hostname ?? IP), plus a Save button.
    - Enter saves and Escape restores.
    - "Use default name" sends `name: null`; it is shown only when `name` is set.
    - The server's 400 message is shown in `role="alert"`.
  - `is_new` → a "New device" badge and a "Mark as seen" button (`is_new: false`).
  - A Star toggle, the same as the row's.
  - Copy text for things Net Lantern can't do (spec journey B): "Don't recognise it? Net Lantern can't block devices. Check your router's client list, or change the Wi-Fi password."

- [ ] **Step 1: Write the failing tests**

```ts
it("renames, then resets to the default name", async () => {
  mockFetch({ "GET /api/devices/7": [200, dev({ id: 7, hostname: "DiskStation", display_name: "DiskStation" })],
              "PATCH /api/devices/7": [200, dev({ id: 7, name: "Backup box", display_name: "Backup box", hostname: "DiskStation", is_new: false })] });
  renderDetails(7);
  const input = await screen.findByLabelText("Name");
  await user.clear(input); await user.type(input, "Backup box{Enter}");
  expect(await screen.findByRole("heading", { name: "Backup box" })).toBeInTheDocument();
  expect(lastRequestBody("PATCH /api/devices/7")).toEqual({ name: "Backup box" });
});
it("shows the server's validation message", async () => { /* PATCH 400 "Names can be at most 64 characters." → role=alert */ });
it("Mark as seen clears the New badge", async () => { /* is_new true → click → body {is_new:false} → badge gone */ });
it("opens from the table and is addressable by URL", async () => { /* click row name → URL has ?device=1 → aside visible */ });
```

- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `npm test` → PASS; lint and build succeed.
- [ ] **Step 5: Commit:** `feat(ui): device details panel with rename and mark-as-seen`

---

### Task 15: UI: REST fallback while the WebSocket is down; latest-scan resync

**Files:**
- Modify: `ui/src/api/{events.ts,queries.ts}`
- Test: `ui/src/api/events.test.ts` (extend)

**Interfaces:**
- Produces:
  - `EventHandlers` gains `onStatus(open: boolean)`:
    - `true` on each `hello`.
    - `false` on close, and also on start, before the first hello.
  - `connectEvents`: if `new WS(url)` throws, schedule the next reconnect anyway. This closes a Phase 0 deferred minor.
  - `queries.ts`: `fetchLatestScan(): Promise<ScanStatus | null>` (404 → `null`).
  - `useLiveEvents`:
    - **`onResync`:** besides today's invalidations, calls `fetchLatestScan()`. If it is running and not the followed scan, it calls `attach(id)` and then `setScan`. This catches a scan started in another tab before this one connected.
    - **While `open` is false:**
      - Every 2 s, if the followed scan is running: `refreshScan(id)` and invalidate `["devices"]`.
      - If no scan is followed: `fetchLatestScan()` and attach if it is running.
      - It stops when `open` turns true.

- [ ] **Step 1: Write the failing tests** (fake timers, stub WS)

```ts
it("polls the running scan every 2 s while the socket is down, and stops on hello", async () => {
  const refresh = vi.spyOn(queries, "refreshScan").mockResolvedValue();
  useLive.getState().attach(7);
  const ws = mountLiveEvents();                 // renders a component calling useLiveEvents with a stub WS
  ws.last().close();
  await vi.advanceTimersByTimeAsync(4_100);
  expect(refresh).toHaveBeenCalledTimes(2);
  ws.last().open(); ws.last().message({ type: "hello", seq: 3, api_version: 1 });
  refresh.mockClear(); await vi.advanceTimersByTimeAsync(4_100);
  expect(refresh).not.toHaveBeenCalled();
});
it("a throwing WebSocket constructor still reconnects", async () => { /* WS throws once, then succeeds → 2 constructions after 1 s */ });
it("resync attaches to a scan running elsewhere", async () => {
  mockFetch({ "GET /api/scans/latest": [200, scanStatus({ id: 9, state: "running" })] });
  /* hello → useLive scan id 9 */
});
```

- [ ] **Step 2:** `npm test` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `npm test` → PASS; lint and build succeed.
- [ ] **Step 5: Commit:** `feat(ui): REST polling while the event stream is down; resync attaches to running scans`

---

### Task 16: End-to-end, docs and spec records

**Files:**
- Modify:
  - `ui/e2e/demo.spec.ts`
  - `README.md`
  - `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§6.2 step 2, §6.12, §15)
  - `.github/workflows/ci.yml`, only if a new step is needed
- Test: `ui/e2e/demo.spec.ts`

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Write the e2e tests.** They join the existing two, keep the serial shared page, and run on `--demo` with its in-memory store:

```ts
test("devices show vendors, names and types", async ({ page }) => {
  // after the existing scan test has run
  await page.getByRole("link", { name: "Devices" }).click();
  const tv = page.getByRole("row", { name: /\[TV\] Living Room/ });
  await expect(tv).toContainText("Samsung"); await expect(tv).toContainText("TV");
  await expect(page.getByRole("row", { name: /192\.168\.1\.44/ })).toContainText("Private address");
});
test("rename and star persist across a reload", async ({ page }) => {
  await page.getByRole("button", { name: "DiskStation" }).click();
  await page.getByLabel("Name").fill("Backup box"); await page.getByLabel("Name").press("Enter");
  await page.getByRole("button", { name: "Star Backup box" }).first().click();
  await page.reload();
  await page.getByRole("button", { name: "Starred" }).click();
  await expect(page.getByRole("row", { name: /Backup box/ })).toBeVisible();
});
```

- [ ] **Step 2:** `cargo build -p netlantern`, then `npm run build && npm run e2e` in `ui/` → PASS. If a test fails, fix the owning task's code, not the test.
- [ ] **Step 3: Update the spec.** Each change below is tagged "(changed in phase 1)":
  - §6.2 step 2 gets the TCP-probe ruling from Task 10.
  - §6.2 step 4 says mDNS uses one-shot unicast-response queries, never binding 5353.
  - §6.12 adds these rows:
    - TCP probe timeout 800 ms and concurrency 64
    - ARP resolve timeout 3 s
    - ARP poll interval during the sweep 1 s
    - NetBIOS timeout 1 s
    - DNS query timeout 1 s
    - UPnP description timeout 2 s
  - §15 records the interface choice when a VPN is active (Task 2 rule) and the gateway-ignores-ping fallback (the network is identified by the gateway's ARP reply; the scan fails with a message if there is none).
- [ ] **Step 4: Update the README.**
  - Status becomes "Phase 1: real quick scan on Windows; demo mode".
  - Quick start: `netlantern` scans this network, and `netlantern --demo` runs the simulator.
  - What traffic a scan sends: ping, ARP, TCP on 5 ports, mDNS, SSDP, NetBIOS, reverse DNS to the router. Nothing leaves the LAN.
  - Data location `%LOCALAPPDATA%\NetLantern`.
  - `--scan-once` and `--data-dir`.
  - The OUI data source and the refresh command.
- [ ] **Step 5: Run the full local check.**
  - `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
  - Then, in `ui/`: `npm run lint && npm test && npm run build && npm run e2e`
  - Everything must PASS.
- [ ] **Step 6: Commit:** `test: phase 1 e2e journeys; docs: README and spec records for phase 1`

---

## Phase 1 review checkpoint (spec §13)

- [ ] **Demo:** run `netlantern` on the author's home network. Show:
  - devices streaming in during the sweep;
  - vendors, names and types;
  - a rename and a star that survive a restart of the exe;
  - a device turned off showing "Offline · last seen…" after a rescan.
- [ ] **Done-when (spec §12):** a real scan lists devices with vendors, and names persist. Then measure against §2.1 and record the numbers in the spec:
  - **Scan time:** `--scan-once` three times. Median ≤ 20 s; first devices visible ≤ 3 s in the UI.
  - **Coverage:** compare `--scan-once` output with the router's client list. Found / listed ≥ 95%. Count devices found only by ARP.
  - **Identification:** vendor shown for every non-private MAC; type other than unknown for ≥ 70%.
- [ ] **OS-ARP check (spec §6.3).** Record in §6.3:
  - the devices on the router's list that the quick scan missed;
  - whether each answers ping from another machine.
  - If OS ARP misses devices that a raw ARP sweep would find, Npcap + the elevated helper return in phase 4 (§16).
- [ ] **Firewall check:** first run on a fresh user profile, or after removing Net Lantern's rules. Confirm no Windows Firewall prompt appears during a scan. If one does, record it in §16 with the triggering socket.
- [ ] **Manual checklist:**
  - VPN on: Net Lantern still picks Wi-Fi.
  - Ethernet + Wi-Fi together: the lower-metric adapter is picked.
  - Airplane mode: "Not connected".
  - A phone rejoining with a new private MAC keeps its name.
  - Café or hotspot (if available): the isolation message.
- [ ] **Summary** for the author: the Windows APIs used and why there's no admin; how a scan's stages fit together; how device identity works (device vs MAC); where data lives.
- [ ] **Learning check.** Ask the author to explain two or three of the phase 1 goals in their own words:
  - how `IcmpSendEcho` pings without raw sockets;
  - ARP and the neighbour cache (why a ping-silent host still shows up);
  - mDNS, SSDP and NetBIOS in a paragraph each;
  - the locally-administered MAC bit;
  - how the classifier scores a device;
  - what the `// SAFETY:` comments in `win/` promise.

  Walk through any gaps in the code.
- [ ] **Next:** write the Phase 2 plan (diagnosis + Wi-Fi).
