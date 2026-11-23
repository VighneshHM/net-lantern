# Phase 7a checkpoint

Things to run yourself at the screen, on the laptop (phone access items use a phone on the same Wi-Fi). Write what happened on each "Result:" line. The tray, Quit, start at login, notifications and opening links in the browser cannot be tested without a real desktop, so they are all here.

Build with separate target dirs so a running Net Lantern is never in the way (do not kill it). Build the UI first, because it is embedded in both programs:

```powershell
cd ui; npm run build; cd ..

# The exe (still PowerShell toasts)
$env:CARGO_TARGET_DIR = "C:\Users\vighn\AppData\Local\Temp\nl-target"
cargo build --release -p netlantern
& "$env:CARGO_TARGET_DIR\release\netlantern.exe"

# The desktop app
cd apps\desktop\src-tauri
$env:CARGO_TARGET_DIR = "C:\Users\vighn\AppData\Local\Temp\nl-desktop-target"
cargo build --release
& "$env:CARGO_TARGET_DIR\release\netlantern-desktop.exe"
```

The exe and the desktop app share one data folder and one single-instance lock, so run only one at a time. To keep your real data safe, add `--data-dir C:\Temp\nl-test` (an absolute path) to either command for the throwaway items.

1. **Launch the desktop app.** The window opens already signed in, and the Dashboard is reachable in under 2 s (spec §2.1). Write down the time.

Result:

2. **Close the window.** The one-time tray notification ("still running in the tray") shows, the app keeps running (monitor ticks continue; reopen and check the Dashboard numbers moved), and closing again shows no second hint.

Result:

3. **Tray.** "Open Net Lantern" shows the window. Launching the app again while it is hidden focuses the existing window, and no second `netlantern-desktop.exe` stays in Task Manager.

Result:

4. **Quit from the tray** during a quick scan, and again with phone access on: the app exits within about 3 s, and no `netlantern-desktop.exe` remains in Task Manager.

Result:

5. **Start at login.** Turn on "Start Net Lantern when you sign in to Windows" in Settings, sign out and in: the app starts in the tray with no window. Then turn it off again and confirm it stays off after the next sign-in.

Result:

6. **Desktop notification.** Star a device and switch it off. The alert appears as a Windows notification titled "Net Lantern: …". Windows may show nothing for a build run from outside `target\debug` or `target\release` until the 7b installer registers the app id: record whether the toast appeared, and from which folder you ran the app.

Result:

7. **Footprint.** With the window open after 10 minutes of monitoring, Task Manager memory for the app and its WebView2 processes should be under 200 MB in total (spec §2.1). Write the numbers.

Result:

8. **Logs.** `%LOCALAPPDATA%\NetLantern\logs` has today's `netlantern.log.*`, and it contains no `?t=`.

Result:

9. **Copy debug info.** Paste it into Notepad and check that no IP (other than 1.1.1.1 and 8.8.8.8), MAC, Wi-Fi name, device name or your Windows username appears.

Result:

10. **Update check.** Answer the Dashboard question. In Settings, turn it on and see "You have version 0.1.0." There is no banner until a newer release exists in 7b. To see the banner now, run the DESKTOP app in demo mode from the repository root (its release is v9.9.9; demo data stays in memory, and the desktop app ignores `--data-dir` in demo mode silently, with no message):

    ```powershell
    $env:NETLANTERN_PIPE_NAME = "\\.\pipe\netlantern-demo-check"   # so a running Net Lantern is not asked to open instead
    & "C:\Users\vighn\AppData\Local\Temp\nl-desktop-target\release\netlantern-desktop.exe" --demo=scenarios\update-available.json
    ```

    Answer "Yes, check for updates". The Dashboard shows the update banner and Settings shows "Version 9.9.9 is available" with "See what's new". Click both links: the release page must open in your DEFAULT BROWSER, and the app window must stay on the dashboard. Quit the demo from the tray when done. The exe with the same scenario (`netlantern.exe --demo=scenarios\update-available.json`) only checks the banner text in a browser tab; it cannot test the app window's link handling.

Result:

11. **Location retest (spec §16 risk).** Does the desktop app show Wi-Fi details where the unpackaged exe needed both location switches? The answer is expected to be the same until the 7b installer gives a package identity. Record it. The link in the location message must open Windows Settings while the app window stays on the dashboard.

Result:

12. **"Delete all data" on a throwaway copy.** Back up `%LOCALAPPDATA%\NetLantern\netlantern.db` first, or run with `--data-dir`. Confirm the app starts fresh (the first-run card shows again), then restore the backup. Deleting data also removes the pre-upgrade backup copy (`netlantern.db.bak`), so copy that too if you want to keep it. Also try it from the phone with phone access on: the page may stop responding because the phone listener closes. That is expected; reload on the laptop.

Result:

13. **The learning check (spec §13, phase 7).** Explain in your own words: how Tauri hosts the server in-process (one process, the webview talks HTTP to 127.0.0.1); the tray lifecycle (close hides, Quit stops); and why the release in 7b is unsigned and what SmartScreen will say.

Result:

What to look at:

- The exe and the desktop app share one data folder (`%LOCALAPPDATA%\NetLantern`), so names, stars and settings carry over between them.
- The exe still sends PowerShell toasts (route B); only the desktop app uses the Tauri notification plugin.
