# Phase 7b checkpoint

Things to run yourself at the screen, on a real Windows 11 laptop. Write what happened on each "Result:" line. The installer, SmartScreen, the tray icon, start at login, toasts and uninstall cannot be tested without a real desktop, so they are all here.

Nothing here needs a build on your machine: the installer comes from the release workflow. Quit Net Lantern from the tray first if one is running, and note that the installed app and `target\` builds share one data folder (`%LOCALAPPDATA%\NetLantern`) and one single-instance lock. To keep your real data safe, copy `%LOCALAPPDATA%\NetLantern` somewhere else before you start (only after Quit), and put it back afterwards.

1. **Run the release workflow by hand.** On GitHub open Actions, then Release, then Run workflow on the branch you want. When it finishes green, download the `release` artifact from the run page and unzip it. It holds `NetLantern-<version>-setup.exe`, `netlantern-<version>-x64.exe`, `SHA256SUMS.txt` and `notes.md`. A by-hand run drafts no GitHub release: only a pushed `vX.Y.Z` tag does.

Result:

2. **SmartScreen.** Double-click the setup exe. Windows shows "Windows protected your PC". Click **More info**, then **Run anyway**, and the installer opens. Also run `Get-FileHash .\NetLantern-<version>-setup.exe -Algorithm SHA256` and compare it with `SHA256SUMS.txt`.

Result:

3. **Install, no UAC, icon.** The install finishes without a User Account Control (administrator) prompt. There is a "Net Lantern" Start menu entry. Launch it and look at the lantern icon in the window title bar, the taskbar and the tray. At 16 px (the tray, and the title bar at 100% scale) the lantern must still be legible. Write down what you see.

Result:

4. **Install again over a running copy.** Add a custom name and a star to a device, and turn on start at login (item 5 below does the check). With Net Lantern running, run the same setup exe again. The installer asks you to close the running app (or offers to). Then it shows the old uninstaller's confirm page with the checkbox "Also delete Net Lantern's data (devices, history, settings and logs)". Leave it UNTICKED. After the install, launch the app: the devices, names, stars and settings are all still there, and start at login is still on.

Result:

5. **Start at login.** In Settings, turn on "Start Net Lantern when you sign in to Windows", sign out and in: the app starts hidden in the tray with no window. Then open Task Manager, Startup apps, and turn "Net Lantern" off there. Open Net Lantern's Settings: it must show start at login as off. Turn it on in Settings: Task Manager shows it enabled again.

Result:

6. **Toast source.** Star a device and switch it off. The Windows toast must show "Net Lantern" as its source, not Windows PowerShell. Record whether it appeared and what the source line says.

Result:

7. **Uninstall, both ways.** Open Settings, Apps, Installed apps, Net Lantern, Uninstall. First time, leave the box UNTICKED: after it finishes, `%LOCALAPPDATA%\NetLantern` is still there. Reinstall, then uninstall again with the box TICKED: the whole `%LOCALAPPDATA%\NetLantern` folder (including `logs\`) is gone, and a message explains how to remove leftover firewall rules.

Result:

8. **Installer size.** The setup exe is under 15 MB (spec section 2.1). The workflow builds it at about 5 MB. Write the size from File Explorer properties.

Result:

9. **Startup.** After launching the installed app, the Dashboard is reachable in under 2 s (spec section 2.1). Write down the time.

Result:

10. **README on GitHub.** Open the repository page on GitHub. The README renders, all seven screenshots show, the download link goes to the latest release, and the SmartScreen paragraph and the `Get-FileHash` command are readable.

Result:

11. **Upgrade keeps start at login and shows no firewall dialog.** With start at login ON, run a newer setup exe over the installed one (item 4's setup again is fine). While it runs, no "Windows Security Alert" or firewall dialog may appear and no "firewall rules" message box may appear. After the install, Settings still shows start at login on, and Task Manager, Startup apps, still lists Net Lantern as enabled. This is what shows the installer recognised the upgrade (it checks that the old uninstaller is running as `uninstall.exe`), which could not be tested without running an installer. Note that this protection is only in builds that contain it, so the check needs two builds that both have it.

Result:

12. **The checkbox label fits.** On the uninstall page (Settings, Apps, Uninstall, or during an upgrade), the label "Also delete Net Lantern's data (devices, history, settings and logs)" must fit inside its box and not be cut off at the right edge. If it is cut off, tell me and the label gets shortened.

Result:

13. **Uninstall removes start at login and shows the firewall note.** Turn on start at login, then uninstall from Settings, Apps. Afterwards, Task Manager, Startup apps, no longer lists Net Lantern, and in `regedit` the value "Net Lantern" is gone from `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`. The message about Windows Firewall rules and an administrator appears. (A silent uninstall shows no message, which is expected.)

Result:

14. **A dev build does not touch the installed entry.** With the installed app's start at login ON, run any other build, for example `target\debug\netlantern-desktop.exe` or a `cargo build --release` copy of the desktop app, quit it, and look at the `Run` value "Net Lantern" in `regedit` (or run `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v "Net Lantern"`). It must still point at the installed exe, not at the build you ran. Only an entry whose exe has gone is repaired.

Result:

15. **The learning check (spec section 13, phase 7).** Explain in your own words: what a per-user installer is and what it writes (and why no administrator is needed); how the installer tells an upgrade from a real uninstall and why that matters for start at login; why unsigned code triggers SmartScreen and what signing would change.

Result:

What to look at:

- The release workflow's notes (`notes.md` in the artifact) hold the changelog section for the version, the SmartScreen paragraph and the checksum steps.
- The installer never touches a custom `--data-dir` folder, even with the box ticked.
- Phone-access firewall rules always need an administrator to remove, on every uninstall.
