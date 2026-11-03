# NetBot

NetBot shows what's on your Wi-Fi and, when the internet breaks, which layer broke and what to try — all on your own laptop, no account, nothing uploaded.

**Status: Phase 0, demo mode only.** The Rust server, the embedded React dashboard and a simulated 13-device home network work end to end. Scanning a real network is not built yet (Phase 1), so `--demo` is currently the only way to run it. Windows 11 only.

NetBot is for networks you own or have permission to scan.

## Quick start

You need Node 22+ and a Rust toolchain.

```
cd ui
npm ci
npm run build
cd ..
cargo run -p netbot -- --demo
```

The UI is embedded into `netbot.exe` at compile time, so build it before `cargo`. NetBot prints a link like `NetBot running at http://127.0.0.1:<port>/?t=<token>` and opens it in your browser. The `t` token works once; after that the browser stays signed in with a cookie, so reloading the page is fine. Only one NetBot runs per Windows user: launching it again opens the running instance's dashboard.

Click **Quick scan** to see the simulated network, then open **Devices**.

## Dev loop

Run the server and the Vite dev server side by side:

```
cargo run -p netbot -- --demo --no-browser --port 47800 --dev-origin http://localhost:5173
```

```
cd ui
npm run dev
```

NetBot prints its link on port 47800. Open it with the host and port swapped for the dev server's, for example `http://localhost:5173/?t=<token>`. Vite proxies `/api` (including the WebSocket) to port 47800, so the token is redeemed and the cookie is set on `localhost:5173`.

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

`npm run e2e` (Playwright, Chromium) starts `target/debug/netbot.exe --demo` on port 47850, so run `cargo build -p netbot` first and close any other running NetBot. The first time, install the browser with `npx playwright install chromium`. CI runs all of this on `windows-latest`.

## Repository layout

```
crates/netbot/            the netbot.exe binary (CLI, demo mode, single instance)
crates/netbot-server/     axum REST API, WebSocket events, auth, embedded UI
crates/netbot-core/       scan pipeline
crates/netbot-platform/   Platform and NetIo traits, plus the simulated network
ui/                       React + TypeScript dashboard
scenarios/                simulated-network scenario files
docs/                     design spec and plans
```

The full design is in [docs/superpowers/specs/2026-10-31-netbot-design.md](docs/superpowers/specs/2026-10-31-netbot-design.md).
