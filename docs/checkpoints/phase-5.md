# Phase 5 checkpoint

Things to run yourself on the home network. Write what happened on each "Result:" line.

Build with a separate target dir so a running `netlantern.exe` is never in the way (do not kill it). Build the UI first, because it is embedded in the binary:

```powershell
cd ui; npm run build; cd ..
$env:CARGO_TARGET_DIR = "C:\Users\vighn\AppData\Local\Temp\nl-target"
cargo build --release -p netlantern
& "$env:CARGO_TARGET_DIR\release\netlantern.exe"
```

1. **The map after a quick scan.** Run a quick scan on the home network, then open Map. The router is in the center, the internet is above it, and every device from the Devices screen is present.

Result:

2. **Colours and text agree.** The laptop and router are online, and the unknown phone is online or offline as it really is. Check each node's colour against the text in the "On the map" list.

Result:

3. **Details panel.** Click the router and a device on the map. The details panel opens each time. Close it.

Result:

4. **Problems only.** Unplug or switch off a starred device, wait for the offline alert (2 min, or 10 for phones), and confirm the map shows it as a problem without a reload.

Result:

5. **Live refresh.** Zoom in, wait for a monitor tick or run a quick scan, and confirm the view does not jump.

Result:

6. **Phone width.** Open the dashboard on the phone, or use the browser's 375 px device mode, then go to Map. Labels are hidden, nodes are smaller, the page scrolls with one finger over the map, and the zoom buttons work.

Result:

7. **The palette swap.** Set `data-palette="green-red"` on `<html>` in devtools and reload the map. The node colours follow the palette.

Result:

8. **A learning check (spec §13).** Explain in your own words why the map's positions come from a fixed ring layout and not Cytoscape's force layout, and why the canvas has a text list next to it.

Result:

What to look at:

- Cytoscape ships in its own lazy chunk, `NetworkMap-*.js` at 462.58 kB (gzip 148.83 kB), next to the main `index-*.js` at 425.97 kB (gzip 139.52 kB). Check that the other screens do not load it.
- Wheel zoom is off on purpose so the page scrolls over the map; use the Zoom in / Zoom out / Fit map buttons.
