# Phase 3 checkpoint

Things to run yourself on the home network. Write what happened on each "Result:" line.

0a. **Toast spike:** run `toast_spike.exe --powershell`, tell Claude whether a toast appeared.

Result:

0b. **Tauri spike:** run the built `apps/desktop/src-tauri/target/release/netlantern-desktop-spike.exe`. Check the dashboard loads logged in, a quick scan works, closing the window ends the process within 2 s. Note the RAM.

Result:

1. **A device going offline.** Scan, star a device you can unplug or switch off (a printer, a TV, a smart plug), and switch it off. Expect an alert and a toast within 2.5 min, and an incident timeline. Switch it back on; the alert resolves.

Result:

2. **A sleeping phone.** Star your phone, leave it idle and locked for 24 h with Net Lantern running. Expect no alert. Count every alert over that day for the false-alert target (at most 1).

Result:

3. **Internet down.** Pull the router's WAN cable for 3 min. Expect "The internet is down", then a resolve.

Result:

4. **Router off.** This only produces the alert when the laptop is on Ethernet, or on Wi-Fi from an access point separate from the router: switch the router off for 3 min while a starred device is on, and expect "Your router isn't answering", with the device alert held back. When the router is also your Wi-Fi access point, switching it off disconnects the laptop, so no alert fires; instead check that the dashboard says monitoring is not connected, then that it recovers by itself once the router is back.

Result:

5. **Sleep.** Sleep the laptop for 10 min. The graph shows a break and no alert fires on wake.

Result:

6. **Footprint.** After 1 h of monitoring, note the CPU average and RAM of `netlantern.exe` (Task Manager, or `Get-Process netlantern | Select CPU,WS`). Targets: under 1% CPU and under 50 MB RAM.

Result:

7. **Still owed from Phase 1:** quick-scan time; coverage against the router's client list; vendor and type rates; the OS-ARP check (spec 6.3).

Result:
