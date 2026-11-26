# Phase 7b checkpoint

Things to run yourself at the screen, on a real Windows 11 laptop. Write what happened on each "Result:" line. The installer, SmartScreen, the tray icon, start at login, toasts and uninstall cannot be tested without a real desktop, so they are all here.

You need one installer, from either route in item 1; everything after item 1 starts from whichever setup exe you have. Quit Net Lantern from the tray first if one is running, and note that the installed app and `target\` builds share one data folder (`%LOCALAPPDATA%\NetLantern`) and one single-instance lock. To keep your real data safe, copy `%LOCALAPPDATA%\NetLantern` somewhere else before you start (only after Quit), and put it back afterwards.

1. **Get an installer.** GitHub offers "Run workflow" only for workflows that are on the default branch, so the release workflow can be run by hand only after this branch is merged to `main`.
   - Before merging (local route): from the repository root run `cd ui && npm ci && npm run build`, then `cd ../apps/desktop && npm ci && npm run tauri -- build`. The setup exe is in `apps/desktop/src-tauri/target/release/bundle/nsis/`, named `Net Lantern_<version>_x64-setup.exe`. There is no `SHA256SUMS.txt` on this route, so skip the hash check in item 2.
   - After merging (workflow route): on GitHub open Actions, then Release, then Run workflow on `main`. When it finishes green, download the `release` artifact from the run page and unzip it. It holds `NetLantern-<version>-setup.exe`, `netlantern-<version>-x64.exe`, `SHA256SUMS.txt` and `notes.md`. A by-hand run drafts no GitHub release: only a pushed `vX.Y.Z` tag does.

Result:

2. **SmartScreen.** Double-click the setup exe. Windows shows "Windows protected your PC". Click **More info**, then **Run anyway**, and the installer opens. With the workflow's files, also run `Get-FileHash .\NetLantern-<version>-setup.exe -Algorithm SHA256` and compare it with `SHA256SUMS.txt` (the letter case doesn't matter).

Result:

3. **Install, no UAC, icon.** The install finishes without a User Account Control (administrator) prompt. There is a "Net Lantern" Start menu entry. Launch it and look at the lantern icon in the window title bar, the taskbar and the tray. At 16 px (the tray, and the title bar at 100% scale) the lantern must still be legible. Write down what you see.

Result:

4. **Install again over a running copy.** Add a custom name and a star to a device, and turn on start at login (item 5 below does the check). With Net Lantern running, run the same setup exe again. It shows an "Already Installed" page whose default choice for the same version is "Add/Reinstall components", which skips the uninstaller: pick **Uninstall Net Lantern** instead and click Next. That runs the old `uninstall.exe` in place (`_?=`), the same way a real upgrade does, so one build is enough (a build with a higher version defaults to "Uninstall before installing"). The uninstaller asks you to close the running app (or offers to). Then it shows its confirm page with the checkbox "Also delete Net Lantern's data (devices, settings, logs)". Leave it UNTICKED, and let the installer carry on with the install. After the install, launch the app: the devices, names, stars and settings are all still there, and start at login is still on.

Result:

5. **Start at login.** In Settings, turn on "Start Net Lantern when you sign in to Windows", sign out and in: the app starts hidden in the tray with no window. Then open Task Manager, Startup apps, and turn "Net Lantern" off there. Open Net Lantern's Settings: it must show start at login as off. Turn it on in Settings: Task Manager shows it enabled again.

Result:

6. **Toast source.** Star a device and switch it off. The Windows toast must show "Net Lantern" as its source, not Windows PowerShell. Record whether it appeared and what the source line says.

Result:

7. **Uninstall, both ways.** Back up `%LOCALAPPDATA%\NetLantern` first, or use a throwaway Windows profile: the second uninstall deletes it. Open Settings, Apps, Installed apps, Net Lantern, Uninstall. First time, leave the box UNTICKED: after it finishes, `%LOCALAPPDATA%\NetLantern` is still there. Reinstall, then uninstall again with the box TICKED: the whole `%LOCALAPPDATA%\NetLantern` folder (including `logs\`) is gone, and a message explains how to remove leftover firewall rules.

Result:

8. **Installer size.** The setup exe is under 15 MB (spec section 2.1). Either route builds it at about 5 MB. Write the size from File Explorer properties.

Result:

9. **Startup.** After launching the installed app, the Dashboard is reachable in under 2 s (spec section 2.1). Write down the time.

Result:

10. **README on GitHub.** Open the repository page on GitHub. The README renders, all seven screenshots show, the download link goes to the latest release, and the SmartScreen paragraph and the `Get-FileHash` command are readable.

Result:

11. **Upgrade keeps start at login and shows no firewall dialog.** With start at login ON, run the same setup exe again over the installed one and pick **Uninstall Net Lantern** on the "Already Installed" page (not "Add/Reinstall components", which never runs the uninstaller and so tests nothing here). That runs the old `uninstall.exe` in place with `_?=`, the same mechanism as a real upgrade, so one build is enough; a build with a higher version would default to "Uninstall before installing". Leave the delete-data box unticked. While it runs, no "Windows Security Alert" or firewall dialog may appear and no "firewall rules" message box may appear. After the install, Settings still shows start at login on, and Task Manager, Startup apps, still lists Net Lantern as enabled. This is what shows the installer recognised the upgrade (it checks that the old uninstaller is running as `uninstall.exe`), which could not be tested without running an installer.

Result:

12. **The checkbox label fits.** On the uninstall page (Settings, Apps, Uninstall, or during an upgrade), the label "Also delete Net Lantern's data (devices, settings, logs)" must fit inside its box and not be cut off at the right edge. If it is cut off, tell me and the label gets shortened.

Result:

13. **Uninstall removes start at login and shows the firewall note.** Turn on start at login, then uninstall from Settings, Apps, with the delete-data box UNTICKED (item 7 covers the ticked case). Afterwards, Task Manager, Startup apps, no longer lists Net Lantern, and in `regedit` the value "Net Lantern" is gone from `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`. The message about Windows Firewall rules and an administrator appears. (A silent uninstall shows no message, which is expected.)

Result:

14. **A dev build does not touch the installed entry.** With the installed app's start at login ON, run any other build, for example `target\debug\netlantern-desktop.exe` or a `cargo build --release` copy of the desktop app, quit it, and look at the `Run` value "Net Lantern" in `regedit` (or run `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v "Net Lantern"`). It must still point at the installed exe, not at the build you ran. Only an entry whose exe has gone is repaired.

Result:

15. **The learning check (spec section 13, phase 7).** Explain in your own words: what a per-user installer is and what it writes (and why no administrator is needed); how the installer tells an upgrade from a real uninstall and why that matters for start at login; why unsigned code triggers SmartScreen and what signing would change.

Result:

What to look at:

- The release workflow's notes (`notes.md` in the artifact) hold the changelog section for the version, the SmartScreen paragraph and the checksum steps.
- The installer never touches a custom `--data-dir` folder, even with the box ticked.
- Phone-access firewall rules always need an administrator to remove, on every uninstall.
