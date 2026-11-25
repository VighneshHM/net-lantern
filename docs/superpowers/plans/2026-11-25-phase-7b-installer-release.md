# Phase 7b: Installer, Release and README Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A stranger can download an unsigned per-user installer from a GitHub release, get past SmartScreen with clear guidance, install without admin, use the desktop app with a real icon and working start at login, and uninstall it, optionally with all of its data.

**Architecture:** The Tauri desktop crate gets a real bundle: the NSIS per-user installer, with install hooks for data removal, start-at-login cleanup and the firewall note. Start at login moves off `tauri-plugin-autostart` to a small HKCU `Run` key module in `netlantern-platform`. It sits behind a registry trait, so it is tested without touching the real registry. One version number feeds every crate, checked by `xtask`. A tag-triggered GitHub Actions workflow builds, checks size and checksums, and creates a **draft** release for the author to publish. The README is rewritten around the release, with screenshots taken by a Playwright script against the demo.

**Tech Stack:** Tauri v2 bundler (NSIS), `@tauri-apps/cli` v2 (pinned through npm), the `windows` crate (registry), GitHub Actions on `windows-latest` with `gh`, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§2.1 Download target, §6.11 per-user installer, §10.1 Uninstall, §12 phase 7, §13 unsigned releases, §16 SmartScreen risk).

## Global Constraints

- Windows 11 x64 only. The installer is **per-user**: no admin rights, no UAC prompt.
- Installer size **< 15 MB** (15,000,000 bytes), enforced by the release workflow.
- Builds are **unsigned**. SmartScreen guidance goes in the README and in every release's notes.
- Data folder: `%LOCALAPPDATA%\NetLantern` by default, holding the database, its `.bak` copy and `logs\`. The install folder is the Tauri per-user default (`%LOCALAPPDATA%\Net Lantern`), which is a different folder.
- Releases live at `https://github.com/VighneshHM/net-lantern/releases/` (the update check only links there). Tags are `vX.Y.Z` with no pre-release suffix.
- The release workflow creates **drafts only**. The author publishes them. No task pushes a tag or publishes a release.
- **No license** for now. The README says "All rights reserved" and does not add a LICENSE file.
- No test, script or manual step run by an agent writes the real Windows registry or touches the author's installed Net Lantern or its data folder.
- Start at login value name: `Net Lantern`, under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, with the data `"<full exe path>" --hidden` (the path always in double quotes).
- Commits: one plain sentence, no prefix, no body, no Co-Authored-By or other attribution.

## Review Focus

1. **Install path with a space** (`...\Local\Net Lantern\`). Start at login must still launch the app. An unquoted `Run` value breaks or hijacks. Pinned by `run_command_quotes_the_path` (Task 2).
2. **Startup turned off in Task Manager.** Windows keeps the `Run` value and marks it disabled under `StartupApproved\Run`, so Settings must show "off", and turning it on again must re-enable it. Pinned by `a_startup_disabled_in_task_manager_reads_off` and `turning_on_clears_the_task_manager_flag` (Task 2).
3. **Upgrade install over a running copy.** Data, settings and the start-at-login value survive. Only a real uninstall with the box ticked deletes data. Pinned by the update-mode guard in `hooks.nsh` (Task 5) and checkpoint items 4 and 6.
4. **A tag that disagrees with the code's version** (for example a `v0.1.1` tag on a `0.1.0` build). The update check would then compare against the wrong number, so the release must fail before building. Pinned by `a_tag_that_disagrees_fails` (Task 1).
5. **A reinstall that moves the exe.** The stored `Run` value points at a missing file, so the next start rewrites it. Pinned by `repair_rewrites_a_stale_command` (Task 2).

---

### Task 1: One version number, changelog and release notes

**Files:**
- Create: `CHANGELOG.md`
- Modify: `crates/xtask/src/main.rs`, `crates/xtask/Cargo.toml` (add `toml = "0.8"`)
- Modify: `apps/desktop/src-tauri/tauri.conf.json` (remove `"version"`, so Tauri uses the desktop crate's version)
- Test: `crates/xtask/src/main.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `cargo run -p xtask -- version` prints the workspace version (for example `0.1.0`). `cargo run -p xtask -- check-version <tag>` exits 0 or prints why and exits 1. `cargo run -p xtask -- release-notes <version>` prints that version's changelog section.
- Produces (fns): `fn workspace_version(root: &Path) -> Result<String, String>`, `fn check_version(root: &Path, tag: &str) -> Result<String, String>` (returns the version), `fn release_notes(changelog: &str, version: &str) -> Option<String>`.

`check_version` passes only when all four hold. Each failure message names the file and both values:
- the `[workspace.package] version` in `Cargo.toml` equals the `[package] version` in `apps/desktop/src-tauri/Cargo.toml`;
- `apps/desktop/src-tauri/tauri.conf.json` has no top-level `"version"` key;
- `tag == format!("v{version}")`;
- `CHANGELOG.md` has a line exactly `## <version>`.

`release_notes` returns the trimmed text after `## <version>` up to the next line starting `## `, or `None` if there is no such section or it is empty.

- [ ] **Step 1: Write the failing tests.** Use a temp dir holding the three files plus `CHANGELOG.md`.
  - `matching_versions_pass`: returns `Ok("0.1.0")` for tag `v0.1.0`.
  - `a_tag_that_disagrees_fails`: tag `v0.1.1` gives an `Err` naming both `v0.1.1` and `0.1.0`.
  - `a_desktop_crate_that_disagrees_fails`.
  - `a_version_key_in_tauri_conf_fails`.
  - `a_missing_changelog_section_fails`.
  - `release_notes_stop_at_the_next_section`: with `## 0.2.0\nB\n## 0.1.0\nA\n`, version `0.2.0` gives `Some("B")`.
  - `release_notes_of_an_unknown_version_is_none`.
- [ ] **Step 2: Run** `cargo test -p xtask`. Expected: the new tests FAIL (the functions are not defined).
- [ ] **Step 3: Implement** the three fns and the three subcommands beside the existing `oui` one. Remove `"version"` from `tauri.conf.json`.
- [ ] **Step 4: Write `CHANGELOG.md`.** It starts with `# Changelog`, then a `## 0.1.0` section: 6–10 plain-language bullets on what v0.1.0 does (quick and deep scan, device names and stars, layer-by-layer diagnosis, Wi-Fi quality, monitoring and alerts, network map, phone access, desktop app with tray and start at login, update check, debug info, delete all data). Facts only, taken from the spec.
- [ ] **Step 5: Run** `cargo test -p xtask` (PASS) and `cargo run -p xtask -- check-version v0.1.0` (exit 0, prints `0.1.0`).
- [ ] **Step 6: Commit** `Check that the tag, the workspace and the desktop app share one version, and add the changelog the release notes come from.`

---

### Task 2: Start at login through our own HKCU Run value

The plugin wrote the `Run` value without quotes, read HKLM first and ignored Task Manager's "disabled" flag. This replaces it.

**Files:**
- Create: `crates/netlantern-platform/src/win/startup.rs` (the trait, the logic and the HKCU implementation), exported as `netlantern_platform::startup`. The pure parts compile on every target; `HkcuRegistry` is `cfg(windows)`.
- Modify: `crates/netlantern-platform/Cargo.toml` (`windows` features: add `Win32_System_Registry` if missing)
- Modify: `apps/desktop/src-tauri/src/autostart.rs`, `apps/desktop/src-tauri/src/main.rs` (drop the plugin's init, call `repair` at start), `apps/desktop/src-tauri/Cargo.toml` (remove `tauri-plugin-autostart`), and remove the plugin's capability entries if any exist
- Test: `crates/netlantern-platform/src/win/startup.rs` (`#[cfg(test)]`, using a fake registry)

**Interfaces:**
- Produces:
  ```rust
  pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
  pub const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
  pub const VALUE_NAME: &str = "Net Lantern";
  pub fn run_command(exe: &Path, args: &[&str]) -> String;          // "\"C:\\x y\\a.exe\" --hidden"
  pub trait UserRegistry: Send + Sync {                             // all under HKCU; a missing value is Ok(None) / a no-op delete
      fn get_string(&self, key: &str, name: &str) -> io::Result<Option<String>>;
      fn get_binary(&self, key: &str, name: &str) -> io::Result<Option<Vec<u8>>>;
      fn set_string(&self, key: &str, name: &str, value: &str) -> io::Result<()>;
      fn delete(&self, key: &str, name: &str) -> io::Result<()>;
  }
  pub struct StartAtLogin<R: UserRegistry> { /* registry, command */ }
  impl<R: UserRegistry> StartAtLogin<R> {
      pub fn new(registry: R, command: String) -> Self;
      pub fn is_enabled(&self) -> io::Result<bool>;   // Run value present AND not disabled in APPROVED_KEY
      pub fn set_enabled(&self, on: bool) -> io::Result<()>;
      pub fn repair(&self) -> io::Result<()>;          // Run value present but != command -> overwrite; absent -> nothing
  }
  #[cfg(windows)] pub struct HkcuRegistry;           // RegOpenKeyExW/RegCreateKeyExW on HKEY_CURRENT_USER
  ```
- Consumes: the server's `Autostart` trait (`is_enabled() -> Result<bool, String>`, `set_enabled(bool) -> Result<(), String>`), unchanged.

Rules:
- `APPROVED_KEY`'s value is binary. When its first byte is odd (Task Manager writes `03` to disable and `02` to enable), the entry is disabled.
- `set_enabled(true)` writes `RUN_KEY\VALUE_NAME = command` and deletes the `APPROVED_KEY` value.
- `set_enabled(false)` deletes both values.
- `command` is `run_command(&current_exe, autostart::ARGS)`. The desktop app calls `repair()` once at start and logs a failure with `tracing::warn!`, without the path.

- [ ] **Step 1: Write the failing tests.** Use a `FakeRegistry` (a `Mutex<HashMap<(String, String), Value>>`).
  - `run_command_quotes_the_path`: `run_command(Path::new(r"C:\Users\a b\AppData\Local\Net Lantern\Net Lantern.exe"), &["--hidden"])` == `"\"C:\\Users\\a b\\AppData\\Local\\Net Lantern\\Net Lantern.exe\" --hidden"`.
  - `off_when_no_value`.
  - `on_after_enable_and_writes_the_command`.
  - `off_after_disable_and_both_values_are_gone`.
  - `a_startup_disabled_in_task_manager_reads_off`: Run value set, approved value `[3,0,0,0,0,0,0,0,0,0,0,0]` → `false`.
  - `an_approved_entry_marked_enabled_reads_on`: first byte `2` → `true`.
  - `turning_on_clears_the_task_manager_flag`.
  - `repair_rewrites_a_stale_command`.
  - `repair_leaves_a_missing_value_missing`.
  - `a_registry_error_is_returned_not_swallowed`: the fake returns `Err` on `get_string`, so `is_enabled` returns `Err`.
- [ ] **Step 2: Run** `cargo test -p netlantern-platform startup`. Expected: FAIL (not defined).
- [ ] **Step 3: Implement** `startup.rs`. Then implement `HkcuRegistry`: `RegCreateKeyExW` for writes (creating `RUN_KEY` if it is somehow missing), `RegOpenKeyExW`+`RegQueryValueExW` for reads, `RegDeleteKeyValueW` for deletes. `ERROR_FILE_NOT_FOUND` maps to `Ok(None)` / `Ok(())`. **Do not run anything that calls `HkcuRegistry` against the real registry**: the author checks it in the checkpoint.
- [ ] **Step 4: Switch the desktop app over.** `autostart.rs` becomes `DesktopAutostart(StartAtLogin<HkcuRegistry>)`, implementing `Autostart` with `io::Error` mapped to its `to_string()`. Remove the plugin, its builder call and any `autostart:` permissions in `apps/desktop/src-tauri/capabilities/`.
- [ ] **Step 5: Run** `cargo test -p netlantern-platform` (PASS), `cargo clippy --workspace --all-targets -- -D warnings`, and in `apps/desktop/src-tauri`: `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`.
- [ ] **Step 6: Commit** `Start at login through our own quoted HKCU Run value that respects Task Manager's switch, in place of the autostart plugin.`

---

### Task 3: Loose ends carried from Phase 7a

**Files:**
- Modify: `apps/desktop/src-tauri/src/main.rs` (`shell_open`)
- Modify: `crates/netlantern-platform/src/types.rs` (`MacAddr`'s `Debug`)
- Modify: `crates/netlantern-server/src/debug_info.rs` (`Redactor::apply`)
- Create: `crates/netlantern-server/tests/refused_log.rs`. Move `a_foreign_host_is_logged_once` there from `tests/api.rs`. If the helpers it uses (`Server`, `browser`, `send`, `Captured`) live in `api.rs`, move them to `tests/common/mod.rs` and `mod common;` from both files.

**Interfaces:** none new. `Redactor::apply(&self, text: &str) -> String` keeps its signature.

**ShellExecute.** Microsoft requires COM on the calling thread. The spawned thread calls `CoInitializeEx(null, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)` (raw `ole32` extern like the existing `shell32` one) before `ShellExecuteW`, and `CoUninitialize` after it only if init returned `S_OK` or `S_FALSE`.

**MacAddr Debug.** Hand-write `Debug` as `MacAddr(AA:BB:CC:DD:EE:FF)` (the `Display` form inside), so a MAC that reaches a log through `{:?}` is caught by the debug-info MAC rule.

**Redaction by spans.** The current two passes replace names first, which cuts an address open when a name overlaps its edge. For example, the name `20 Hub` in `192.168.1.20 Hub` leaves `192.168.1.` behind. Change `apply` to work on spans:
1. Find every match of every rule (names, MAC, IPv4 except the allowed three, IPv6, tokens, profile folder) **on the original text**.
2. Merge spans that overlap or touch.
3. Replace each merged span with one placeholder. If an address rule took part, use that rule's placeholder (in the order token > MAC > IPv6 > IPv4); otherwise use `[name]`.

Existing tests keep passing.

**Flaky test.** In its own binary, call `tracing::callsite::rebuild_interest_cache()` right after `set_default`, so no callsite cached as "never" by another test's subscriber can hide the log line.

- [ ] **Step 1: Write the failing tests.**
  - `debug_info.rs`: `a_name_overlapping_an_address_edge_hides_both`: names `["20 Hub"]`, text `"seen 192.168.1.20 Hub"`. The output contains none of `192.168`, `.20`, `Hub`.
  - `debug_info.rs`: `a_hex_like_name_inside_a_mac_hides_the_whole_mac`: names `["aa:bb"]`, text `"mac 11:22:aa:bb:33:44"`. The output contains none of `11:22`, `33:44`, `aa:bb`. If `aa:bb` is under the short-name minimum, use the shortest name the minimum allows, such as `aa:bb:33`.
  - `types.rs`: `debug_shows_the_colon_form`: `format!("{:?}", MacAddr([0xaa,0xbb,0xcc,0xdd,0xee,0xff]))` == `"MacAddr(AA:BB:CC:DD:EE:FF)"`.
- [ ] **Step 2: Run** `cargo test -p netlantern-server debug_info` and `cargo test -p netlantern-platform types`. Expected: the three new tests FAIL.
- [ ] **Step 3: Implement** the span merge, the `Debug` impl, the COM init and the test move.
- [ ] **Step 4: Run** `cargo test --workspace` (PASS). Then run the moved test 30 times: `for i in $(seq 30); do cargo test -p netlantern-server --test refused_log -q || break; done`. Expected: 30 passes.
- [ ] **Step 5: Run** clippy for the workspace and the desktop crate.
- [ ] **Step 6: Commit** `Redact overlapping names and addresses as one span, print MAC addresses in their redactable form, initialise COM before opening release links, and give the refused-host log test its own binary so it stops flaking.`

---

### Task 4: The amber lantern icon and the pinned Tauri CLI

**Files:**
- Create: `apps/desktop/icons/lantern.svg` (the source)
- Create: `apps/desktop/package.json`, `apps/desktop/package-lock.json`
- Replace/create: `apps/desktop/src-tauri/icons/*` (generated)
- Create: `ui/public/favicon.svg` (a copy of the source); modify `ui/index.html` to link it
- Modify: `apps/desktop/src-tauri/tauri.conf.json` (`bundle.icon` lists the generated `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.ico`)
- Test: `apps/desktop/src-tauri/src/main.rs` or a new `tests/icon.rs` in the desktop crate

**Interfaces:**
- Produces: `npm run tauri -- <args>` in `apps/desktop` runs the pinned CLI. Task 5 and Task 6 use `npm run tauri -- build`.

**The icon.**
- A lantern seen from the front, on a 1024×1024 viewBox with a transparent background.
- A dark body (`#1F2937`) with a handle ring, a cap and a base, and a glowing amber glass panel (`#F59E0B`, with a lighter `#FCD34D` core).
- No text and no thin strokes: it must still read as a lantern at 16×16 in the tray. Check the 16 px PNG by eye before committing, and thicken shapes if it turns into a blob.
- Amber is deliberately independent of the undecided landing-site palette.

**The CLI.** `package.json` is private, has no dependencies except the dev dependency `@tauri-apps/cli`, and has the script `"tauri": "tauri"`. Pin an exact version (no `^`) whose minor equals the `tauri` crate's minor in `apps/desktop/src-tauri/Cargo.lock`, taking the latest patch of that minor.

- [ ] **Step 1: Write the failing test** `the_icon_has_every_tray_and_shell_size`. Read `icons/icon.ico`, parse the ICONDIR header (u16 reserved = 0, u16 type = 1, u16 count, then 16-byte entries where a width byte of 0 means 256), and assert that the widths include 16, 24, 32, 48 and 256.
- [ ] **Step 2: Run** `cargo test` in `apps/desktop/src-tauri`. Expected: FAIL, because the placeholder lacks some sizes. If the placeholder happens to pass, note that and carry on.
- [ ] **Step 3: Draw the SVG.** In `apps/desktop`, run `npm install` (creates the lock) and then `npm run tauri -- icon icons/lantern.svg -o src-tauri/icons`. If the CLI rejects SVG input, render a 1024 px PNG from the SVG with Playwright's Chromium (already a UI dev dependency) in a scratch script outside the repo, and feed that PNG in. Remove generated mobile/Store assets that a Windows-only app does not use (`android/`, `ios/`, `Square*Logo.png`, `StoreLogo.png`).
- [ ] **Step 4: Add the favicon** and link it from `ui/index.html`. Then run `npm run lint`, `npm test` and `npm run build` in `ui`.
- [ ] **Step 5: Run** `cargo test` in `apps/desktop/src-tauri` (PASS) and `cargo build --release` there.
- [ ] **Step 6: Commit** `Give Net Lantern an amber lantern icon in every size, use it as the dashboard favicon, and pin the Tauri CLI for bundling.`

---

### Task 5: The per-user NSIS installer and uninstaller

**Files:**
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Create: `apps/desktop/src-tauri/windows/hooks.nsh`
- Create (only if needed for the checkbox label): `apps/desktop/src-tauri/windows/English.nsh`, wired in through `bundle.windows.nsis.customLanguageFiles`
- Modify: `apps/desktop/src-tauri/Cargo.toml` (add `serde_json` as a dev-dependency if it is not reachable)
- Test: `apps/desktop/src-tauri/tests/bundle_config.rs`

**Interfaces:**
- Consumes: `VALUE_NAME`, `RUN_KEY`, `APPROVED_KEY` from Task 2 (copied as literals into `hooks.nsh`). Consumes `npm run tauri -- build` from Task 4.
- Produces: `apps/desktop/src-tauri/target/release/bundle/nsis/Net Lantern_<version>_x64-setup.exe`. Task 6 relies on this path pattern; if the real name differs, Task 6 globs `bundle/nsis/*-setup.exe`.

**Config.** Set:
- `bundle.active: true` and `bundle.targets: ["nsis"]`;
- `bundle.shortDescription: "See what's on your Wi-Fi and why the internet broke."`;
- `bundle.publisher: "Net Lantern"`;
- `bundle.windows.webviewInstallMode: { "type": "downloadBootstrapper" }`, because Windows 11 ships WebView2, which keeps the installer small;
- `bundle.windows.nsis`: `{ "installMode": "currentUser", "installerHooks": "windows/hooks.nsh", "displayLanguageSelector": false }`.

If `tauri build` demands a `build.frontendDist`, add the smallest setting that builds. The window keeps loading the loopback URL exactly as in 7a.

**Before writing `hooks.nsh`, read the installed `tauri-bundler` NSIS template** (`installer.nsi` in the cargo registry source, for the version in `Cargo.lock`). Record in the commit's task report:
- the hook macro names;
- the variable that holds the "delete app data" checkbox state;
- how the template detects an update install, so that it skips deleting data then;
- whether it sets the Start menu shortcut's AppUserModelID to the bundle identifier `dev.netlantern.app`, which toasts need.

If the template does **not** set the AppUserModelID, set it on the shortcut in `NSIS_HOOK_POSTINSTALL` using the template's own macro, or the `ApplicationID::Set` approach the template uses for its other shortcuts.

**`hooks.nsh`, post-uninstall, skipped entirely on an update install:**
1. `DeleteRegValue HKCU "<RUN_KEY>" "Net Lantern"` and `DeleteRegValue HKCU "<APPROVED_KEY>" "Net Lantern"`.
2. If the delete-data checkbox was ticked: `RMDir /r "$LOCALAPPDATA\NetLantern"`. This removes the database, the `.bak` copy and `logs\`. A custom `--data-dir` / `NETLANTERN_DATA_DIR` folder is not touched; the README says so.
3. Unless silent, a `MessageBox MB_OK|MB_ICONINFORMATION` with exactly:
   `If you turned on phone access, Windows Firewall may still have rules that allow Net Lantern. Removing them needs an administrator: open Windows Security, then Firewall & network protection, Advanced settings, Inbound Rules, and delete the rules for Net Lantern.`

The checkbox label reads `Also delete Net Lantern's data (devices, history, settings and logs)`. Override the template's English string only if its default label does not say this.

- [ ] **Step 1: Write the failing test** `bundle_config_is_a_per_user_nsis_installer`. Parse `tauri.conf.json` and assert:
  - `bundle.active == true` and `bundle.targets == ["nsis"]`;
  - `bundle.windows.nsis.installMode == "currentUser"`;
  - `installerHooks == "windows/hooks.nsh"`, and that file exists;
  - `webviewInstallMode.type == "downloadBootstrapper"`;
  - `identifier == "dev.netlantern.app"`;
  - there is no top-level `version`.

  Write a second test, `the_hooks_skip_updates_and_name_the_right_value`. Read `hooks.nsh` and assert that it contains `"Net Lantern"`, `$LOCALAPPDATA\NetLantern` and the update-mode guard expression recorded above.
- [ ] **Step 2: Run** `cargo test --test bundle_config` in `apps/desktop/src-tauri`. Expected: FAIL.
- [ ] **Step 3: Implement** the config and the hooks.
- [ ] **Step 4: Build.** Run `cd ui && npm run build`, then `cd apps/desktop && npm ci && npm run tauri -- build`. Expected: one `*-setup.exe` under `bundle/nsis/`. Print its size in bytes; it must be under 15,000,000. **Do not run the installer**: the author installs it in the checkpoint.
- [ ] **Step 5: Run** `cargo test` and `cargo clippy --all-targets -- -D warnings` in `apps/desktop/src-tauri`.
- [ ] **Step 6: Commit** `Bundle the desktop app as a per-user installer whose uninstaller can delete Net Lantern's data, removes start at login and explains the leftover firewall rules.`

---

### Task 6: CI for the desktop crate and the release workflow

**Files:**
- Modify: `.github/workflows/ci.yml` (new job `desktop`)
- Create: `.github/workflows/release.yml`
- Create: `.github/release-notes-footer.md`

**Interfaces:**
- Consumes: `xtask version`, `check-version`, `release-notes` (Task 1); `npm run tauri -- build` and the setup exe path (Task 5).
- Produces: release assets `NetLantern-<v>-setup.exe`, `netlantern-<v>-x64.exe` (the standalone exe that opens the dashboard in a browser) and `SHA256SUMS.txt` (`sha256sum` format, both files).

**CI `desktop` job** (`windows-latest`, bash):
1. Checkout, then Node 22, then `ui` `npm ci` and `npm run build`. The desktop crate embeds `ui/dist` through `netlantern`.
2. Set up the Rust toolchain with `rustfmt` and `clippy`, and `Swatinem/rust-cache@v2` with `workspaces: apps/desktop/src-tauri -> target`.
3. In `apps/desktop/src-tauri`, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`.

No bundling in CI.

**`release.yml`:**
- Triggers: `push: tags: ['v*']` and `workflow_dispatch`.
- Permissions: `contents: write`. `timeout-minutes: 60`. One job on `windows-latest`, bash.
- Steps:
  1. Checkout, then Node and the Rust toolchain (with caches as in CI).
  2. `TAG` is `$GITHUB_REF_NAME` on a tag push, and `v$(cargo run -q -p xtask -- version)` on dispatch.
  3. `cargo run -q -p xtask -- check-version "$TAG"`. This must be the first real step, so a mismatch fails within minutes.
  4. In `ui`: `npm ci`, `npm run lint`, `npm test`, `npm run build`. Then `cargo test --workspace` and `cargo build --release -p netlantern`.
  5. In `apps/desktop`: `npm ci` and `npm run tauri -- build`.
  6. Copy the setup exe and `target/release/netlantern.exe` into `release/` under the asset names. Fail with a clear message if the setup exe is ≥ 15,000,000 bytes. Write `SHA256SUMS.txt` with `cd release && sha256sum *.exe > SHA256SUMS.txt`.
  7. `notes.md` is `xtask release-notes <v>`, a blank line, then `.github/release-notes-footer.md`.
  8. Always `actions/upload-artifact@v4` the `release/` folder.
  9. On a tag push only: `gh release create "$TAG" --draft --verify-tag --title "Net Lantern <v>" --notes-file notes.md release/*`, with `GH_TOKEN: ${{ github.token }}`.

**`release-notes-footer.md`:**
- `## Installing`: download `NetLantern-<v>-setup.exe`; Windows 11 only; no admin needed.
- The SmartScreen paragraph (exact copy below).
- How to check the checksum: `Get-FileHash .\NetLantern-<v>-setup.exe -Algorithm SHA256`, compared with `SHA256SUMS.txt`.

SmartScreen copy, used verbatim here and in the README:

> Net Lantern isn't code-signed yet, so Windows SmartScreen may say "Windows protected your PC" the first time you run the installer. Check the file against SHA256SUMS.txt (below), then click **More info** and **Run anyway**.

- [ ] **Step 1: Validate the YAML.** Run `npx --yes yaml-lint .github/workflows/*.yml`, or, if that is unavailable, `python -c "import yaml,sys;[yaml.safe_load(open(f)) for f in sys.argv[1:]]" .github/workflows/*.yml`. Expected: no errors.
- [ ] **Step 2: Dry-run the release steps locally in bash, without `gh`.** Run steps 3–7 from a scratch copy of the commands, then confirm that `release/` holds the three files and that `sha256sum -c SHA256SUMS.txt` passes inside it. Delete `release/` afterwards. Add `release/` and `notes.md` to `.gitignore`.
- [ ] **Step 3: Commit** `Check the desktop crate in CI and add a tag-triggered workflow that builds the installer and standalone exe and drafts a GitHub release with checksums and SmartScreen guidance.`

---

### Task 7: Screenshots, README, spec and checkpoint

**Files:**
- Create: `ui/playwright.screenshots.config.ts`, `ui/e2e-screenshots/screenshots.spec.ts`; modify `ui/package.json` (script `"screenshots": "playwright test -c playwright.screenshots.config.ts"`)
- Create: `docs/screenshots/*.png`
- Rewrite: `README.md`
- Modify: `docs/superpowers/specs/2026-10-31-netlantern-design.md`
- Create: `docs/checkpoints/phase-7b.md`

**Screenshots.**
- The config reuses `e2e/globalSetup` (demo, its own pipe) with `testDir: "./e2e-screenshots"`, so `npm run e2e` never runs it.
- Each shot waits for its screen's content. Hide any running clock or spinner via the page's own state, not CSS hacks.
- Each shot is a 1280×800 viewport in the app's default theme, written to `docs/screenshots/`:
  - `dashboard.png`, after one quick scan has finished;
  - `devices.png`, with a device's details open;
  - `diagnose.png`, after one diagnosis;
  - `map.png`;
  - `alerts.png`;
  - `settings.png`;
  - `phone.png`, the dashboard at 390×844.
- Each PNG must be under 400 KB.

**README sections, in this order:**
1. Title, the one-line pitch, a status line (`Version 0.1.0. Windows 11.`) and `dashboard.png`.
2. **Download and install**: the latest release link, per-user with no admin, the SmartScreen paragraph (verbatim from Task 6) and the `Get-FileHash` check.
3. **What it does**: one short paragraph each for scan, diagnose, Wi-Fi quality, monitoring and alerts, map and phone access, with screenshots.
4. **Privacy and what Net Lantern sends**: keep the existing "What a scan sends", "Deep scan" and "What a diagnosis sends" text, and add the update check (off until you say yes; one GET to GitHub; no scan data).
5. **Phone access**: the Private-network requirement and the one-time firewall prompt.
6. **Start at login, logs and debug info**: where the logs are, and that Copy debug info redacts addresses and names.
7. **Uninstall**: Settings → Apps; the data checkbox; a custom data folder is not removed; the firewall rule steps (same text as the uninstaller).
8. **Build from source**: the existing quick start, CLI flags and demo scenarios, plus `npm run tauri -- build` in `apps/desktop`.
9. **Releasing** (for the author): bump `version` in both Cargo.toml files, add a `## <version>` changelog section, merge, then `git tag vX.Y.Z` and `git push origin vX.Y.Z`; review and publish the draft.
10. A footer: `Copyright © 2026 Net Lantern's author. All rights reserved.` and the "networks you own or have permission to scan" line.

**Spec.** Add *(Phase 7b: …)* notes, in the style of the 7a notes, to:
- §10.1 Uninstall: the per-user NSIS installer, the checkbox, `Run` value removal, the firewall message, and no data deletion on an update;
- §12, the phase 7 row: the 7b scope;
- §13: draft releases built by `release.yml`;
- §15: the icon decision;
- §2.1: the measured installer size from Task 5.

**`docs/checkpoints/phase-7b.md`**, in the format of `phase-7a.md`, with these items for the author:
1. Run the release workflow by hand (Actions → Release → Run workflow) and download the artifact.
2. SmartScreen appears; More info and Run anyway work.
3. The install needs no UAC. A Start menu entry and the lantern icon appear in the window, the taskbar and the tray (16 px is legible).
4. Install again over a running copy: the installer asks to close it; devices, settings and start at login survive.
5. Start at login: turn it on, sign out and in, and the app starts hidden in the tray. Turn it off in Task Manager → Startup and Settings shows off.
6. A toast shows "Net Lantern" as its source.
7. Uninstall without the box: `%LOCALAPPDATA%\NetLantern` remains. Reinstall, then uninstall with the box: the folder (and `logs\`) is gone, the `Run` value is gone, and the firewall message appears.
8. The installer is under 15 MB.
9. The dashboard is reachable < 2 s after launch.
10. The README renders on GitHub with its screenshots.

- [ ] **Step 1: Write the screenshot spec and config.** Then run `cargo build -p netlantern` and `cd ui && npm run screenshots`. Expected: 7 PNGs, each < 400 KB. Look at each one.
- [ ] **Step 2: Run** `cd ui && npm run lint && npm run e2e`. Expected: lint clean and e2e unchanged (20/20).
- [ ] **Step 3: Rewrite the README, update the spec and write the checkpoint.** Check every README command against the code; for example `--data-dir`, the scenario names and the `npm run tauri` script exist.
- [ ] **Step 4: Commit** `Add the release README with screenshots, SmartScreen and uninstall guidance, the Phase 7b spec notes and checkpoint.`
