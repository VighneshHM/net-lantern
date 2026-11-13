# Phase 4 checkpoint

Things to run yourself on the home network. Write what happened on each "Result:" line.

Build with a separate target dir so a running `netlantern.exe` is never in the way (do not kill it):

```powershell
$env:CARGO_TARGET_DIR = "C:\Users\vighn\AppData\Local\Temp\nl-target"
cargo build --release -p netlantern
& "$env:CARGO_TARGET_DIR\release\netlantern.exe"
```

1. **Consent prompt.** Open the dashboard on the home network and start a deep scan. Expect a one-time "Is this a network you own or have permission to scan?" prompt. Grant it; run a second deep scan and expect no prompt. Run a quick scan on a fresh database (or after revoking consent with `PUT /api/network/consent {"granted": false}`; there is no UI for that until Phase 6) and expect no prompt.

Result:

2. **Deep scan time.** Run a deep scan on the home /24 and time it. There is no target: record it and the number of devices found.

Result:

3. **Ports and banners.** Compare the router's open ports and page title in the device details with its admin page.

Result:

4. **OS guess.** Check the guess on the laptop (Windows), a phone and the router: right, wrong, or none.

Result:

5. **Type rate again.** After the unicast mDNS change, count how many devices now have a type. Compare with 2 of 4 on 2026-11-13. This decides the deferred "best-guess phone" rule.

Result:

6. **Stop button.** Start a deep scan and press Stop mid scan. Expect the scan to end `cancelled` quickly.

Result:

7. **Wi-Fi off mid scan.** Start a scan and turn Wi-Fi off. Expect it to stop with "The connection dropped during the scan."

Result:

8. **Starred router, HTTPS check.** Star the router and add an HTTPS check on port 443. It should pass despite the router's own certificate.

Result:

9. **Quit while monitoring.** Quit the exe while monitoring. It should exit within about 2 s.

Result:

10. **Unelevated.** Run from a normal (non-admin) window; everything above must work.

Result:

11. **`--scan-once --deep`.** Run and paste the output:

```powershell
& "$env:CARGO_TARGET_DIR\release\netlantern.exe" --scan-once --deep
```

Result:

What to look at:

- Phase 4 changed `call_store` in `store/db.rs` to wait on the blocking pool, which adds one extra thread hop per DB call; watch the monitor's CPU and tick timing for any effect.
- A probe error after a link drop is mapped to the connection-dropped outcome, so check in item 7 that the scan says "connection dropped" and not "failed".
