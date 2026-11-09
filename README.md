# Net Lantern

Net Lantern shows what's on your Wi-Fi and, when the internet breaks, which layer broke and what to try — all on your own laptop, no account, nothing uploaded.

**Status: Phase 2: diagnosis and Wi-Fi quality.** `netlantern` scans the network you are on and lists devices with vendors, names and types; renames and stars are kept between runs. The Diagnose screen checks your connection layer by layer (Wi-Fi link, router, internet, DNS, website), says which layer broke and what to try, and shows your Wi-Fi quality, nearby networks and channel congestion. `--demo` runs a simulated 13-device home network instead. Windows 11 only, no admin rights.

Net Lantern is for networks you own or have permission to scan.

## Quick start

You need Node 22+ and a Rust toolchain.

```
cd ui
npm ci
npm run build
cd ..
cargo run -p netlantern
```

The UI is embedded into `netlantern.exe` at compile time, so build it before `cargo`. Net Lantern prints a link like `Net Lantern running at http://127.0.0.1:<port>/?t=<token>` and opens it in your browser. The `t` token works once; after that the browser stays signed in with a cookie, so reloading the page is fine. Only one Net Lantern runs per Windows user: launching it again opens the running instance's dashboard.

`netlantern` scans this network. `netlantern --demo` runs the simulator instead (in-memory data, nothing saved). Click **Quick scan**, then open **Devices**.

Other flags: `--scan-once` runs one scan, prints the devices found and exits. `--diagnose-once` runs one diagnosis, prints the verdict and exits. `--wifi-once` prints the Wi-Fi connection and nearby networks and exits. `--data-dir <DIR>` keeps the database in `<DIR>` instead of the default.

`--demo` takes an optional scenario file, for example `netlantern --demo=scenarios/cafe.json`. The files in `scenarios/` are home (the default), weak-wifi, dns-failure, provider-down, cafe, tls-intercept, router-down, router-silent, icmp-blocked, location-denied, offline and isolated.

### What a scan sends

ICMP pings and ARP requests to each address in the subnet, TCP connects to ports 80, 443, 445, 22 and 62078 on every device found present (whether it answered a ping or only ARP), mDNS and SSDP queries to their multicast groups, an HTTP GET of the UPnP description a device advertises (sent only to that device's own address), NetBIOS name queries to found devices, and reverse DNS lookups to your router. Everything stays on your LAN; nothing goes to the internet.

### What a diagnosis sends

Only when you run a diagnosis (the button, `POST /api/diagnose` or `--diagnose-once`), and never on a schedule, Net Lantern contacts:

- ICMP echo to `1.1.1.1` and `8.8.8.8`;
- a TCP connect to `1.1.1.1:443`, closed straight away;
- a plain-HTTP GET of `http://www.msftconnecttest.com/connecttest.txt` (redirects not followed), which detects sign-in pages on public Wi-Fi;
- DNS A queries for `www.microsoft.com` and `www.cloudflare.com`, sent to your system DNS servers and to `1.1.1.1`;
- an HTTPS GET of `https://www.microsoft.com/` (redirects not followed).

Every HTTP check sends the User-Agent `netlantern/<version>`, which carries no personal data. No scan data, device data or identifiers are sent. A nearby-network radio scan runs only when you open or refresh the Diagnose screen, or during a diagnosis.

### Wi-Fi details and location permission

Windows hides Wi-Fi names and nearby networks from apps that lack location permission. If Diagnose says it needs location permission, open Windows Settings > Privacy & security > Location, turn on Location services and "Let desktop apps access your location", then refresh. Without it everything else still works and the link layer still passes.

### Where data lives

`%LOCALAPPDATA%\NetLantern\netlantern.db` (demo mode never touches it).

## Dev loop

Run the server and the Vite dev server side by side:

```
cargo run -p netlantern -- --demo --no-browser --port 47800 --dev-origin http://localhost:5173
```

```
cd ui
npm run dev
```

Net Lantern prints its link on port 47800. Open it with the host and port swapped for the dev server's, for example `http://localhost:5173/?t=<token>`. Vite proxies `/api` (including the WebSocket) to port 47800, so the token is redeemed and the cookie is set on `localhost:5173`.

Vendor names come from the IEEE MA-L registry (`https://standards-oui.ieee.org/oui/oui.csv`), embedded as `crates/netlantern-core/data/oui.tsv`. To refresh it, download the CSV by hand and run `cargo run -p xtask -- oui <path/to/oui.csv>`. Blocks the IEEE Registration Authority sub-assigns (MA-M/MA-S) are left out, so those devices show no vendor.

## Checks

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd ui
npm run lint
npm test
npm run build
npm run e2e
```

`npm run e2e` (Playwright, Chromium) starts `target/debug/netlantern.exe --demo` on port 47850, so run `cargo build -p netlantern` first and close any other running Net Lantern. The first time, install the browser with `npx playwright install chromium`. CI runs all of this on `windows-latest`.

## Repository layout

```
crates/netlantern/            the netlantern.exe binary (CLI, demo mode, single instance)
crates/netlantern-server/     axum REST API, WebSocket events, auth, embedded UI
crates/netlantern-core/       scan pipeline
crates/netlantern-platform/   Platform and NetIo traits, plus the simulated network
ui/                       React + TypeScript dashboard
scenarios/                simulated-network scenario files
docs/                     design spec and plans
```

The full design is in [docs/superpowers/specs/2026-10-31-netlantern-design.md](docs/superpowers/specs/2026-10-31-netlantern-design.md).
