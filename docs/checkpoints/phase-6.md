# Phase 6 checkpoint

Things to run yourself on the home network, with a phone on the same Wi-Fi. Write what happened on each "Result:" line.

Build with a separate target dir so a running `netlantern.exe` is never in the way (do not kill it). Build the UI first, because it is embedded in the binary:

```powershell
cd ui; npm run build; cd ..
$env:CARGO_TARGET_DIR = "C:\Users\vighn\AppData\Local\Temp\nl-target"
cargo build --release -p netlantern
& "$env:CARGO_TARGET_DIR\release\netlantern.exe"
```

1. **`--phone-check` before turning phone access on.** Run `& "$env:CARGO_TARGET_DIR\release\netlantern.exe" --phone-check` on the home Wi-Fi and write down what it says about the network category and the firewall.

Result:

2. **Set the network to Private first, then turn phone access on.** Windows currently marks this Wi-Fi Public, and Net Lantern never changes that. Before turning phone access on, open Settings → Network & internet → Wi-Fi → your network → Network profile type → Private. Then open Net Lantern's Settings and click "Allow phone access", then "Continue". Note the Windows Firewall prompt (or none), what you chose, and the firewall line Settings shows afterwards. If you skip the Private step you should instead see phone access paused with the Public-network explanation, and no QR code; that is also a pass, and item 8 covers it, so then set the network to Private and carry on.

Result:

3. **Scan the QR with the phone.** The dashboard opens, and the phone can browse Devices, Map and Alerts.

Result:

4. **The phone swipe check left over from Phase 5, item 6.** On the phone, open Map. A one-finger swipe over the map scrolls the page.

Result:

5. **No token, no dashboard.** Open the same address without `?t=…` on the phone, in a private tab. It must show the sign-in-needed page.

Result:

6. **Turn phone access off.** The phone's open page stops updating, and a reload shows the sign-in-needed page (or cannot connect at all).

Result:

7. **A dismissed prompt.** Skip this and write "skipped" if you would rather not touch firewall rules.
   1. Open Windows Security → Firewall & network protection → Allow an app through firewall, and untick Private for `netlantern.exe`.
   2. Turn phone access off and on, and confirm Settings explains the block.
   3. Tick it again.

Result:

8. **A Public network.** If you can, join a hotspot or café network that Windows marks Public. Phone access says it is paused and why. Or skip.

Result:

9. **Restart with phone access on.** Quit and start Net Lantern again. Phone access comes back on, a new QR shows, and the old QR no longer works.

Result:

10. **Responsive check on the phone.** Every screen (Dashboard, Devices, Map, Diagnose, Alerts, Settings) fits with no sideways scroll.

Result:

11. **A learning check (spec §13, phase 6).** Explain in your own words: why the phone token is reusable but the launch token is one-time; what `HttpOnly` and `SameSite=Strict` protect against; and what Windows does when you dismiss the firewall prompt, and why Net Lantern can only read that rule, never fix it.

Result:

What to look at:

- The phone listener uses the same port as the main listener on the laptop's Wi-Fi address, or the first free port in 47800–47809. Check the port in the phone URL against what Settings shows.
- While phone access is on, Net Lantern checks the network every 5 s to follow a changed address or a Public network. Watch for it in the task manager or the log; it should stay tiny.
