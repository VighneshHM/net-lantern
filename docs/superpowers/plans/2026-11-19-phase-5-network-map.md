# Phase 5: Network Map Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Network map screen: the router at the center, the internet above it, every known device around it colored by status. A "problems only" filter, click for details, and a layout that works at phone width.

**Architecture:** This is a UI-only phase. The server already serves everything the map needs: `GET /api/devices`, `GET /api/alerts?state=active`, `GET /api/monitor` and `GET /api/network`. Live events already invalidate those queries. The work splits into three parts:
- A pure model (`lib/networkMap.ts`) turns those four responses into nodes, edges, statuses and fixed positions. It is fully unit-tested.
- A thin `MapCanvas` component hands the model to Cytoscape.js. It is tested against a mocked `cytoscape`.
- The `NetworkMap` page adds filters, an accessible text list of the same nodes, the existing `DeviceDetails` panel and the honest note. The page is lazy-loaded so Cytoscape stays out of the main bundle.

**Tech Stack:** React 19, TypeScript, TanStack Query, Tailwind 4, Vitest + Testing Library, Playwright, and Cytoscape.js (new).

**Spec:** `docs/superpowers/specs/2026-10-31-netlantern-design.md`. The relevant sections are §6.10 (network map), §9 (UI: stack, look, responsive rules), §12 (phase 5 row) and §13 (checkpoint).

**Branch:** `phase-5-network-map`, created from `phase-4-deep-scan`. Phase 4 is not merged yet, so this branch is stacked on it.

## Global Constraints

- Spec §6.10: "Router at the center, internet above, devices around it colored by status; click for details; 'problems only' filter. Rendered with Cytoscape.js; works on phone widths (labels hide, nodes shrink). Honest note in the UI: switches/mesh points between devices need router access, planned later."
- Spec §9: "colors defined as design tokens (CSS variables) so the palette can be swapped without touching components. Responsive down to phone width; touch targets ≥44 px; status always shown with text, not color alone."
- No raw colours in components or the Cytoscape stylesheet. Read `--ok`, `--warn`, `--bad`, `--text-muted`, `--border-strong`, `--accent`, `--text`, `--surface` and `--bg` from `document.documentElement` at mount.
- No server or API change in this phase. If a task seems to need one, stop and report it.
- Done when (spec §12): "Map usable on desktop and phone width."
- Footprint (spec §2.1): the installer must stay under 15 MB. Cytoscape is loaded only on the map route, through `React.lazy`.
- Commits are one plain sentence with no prefix, no body and no Co-Authored-By or Claude attribution (author's rule).
- Every new UI string is plain English. The status text follows the existing Devices screen: "Online", "Online · doesn't answer ping", "Offline · last seen 5 min ago".
- Verification at the end of every task: from `ui/`, run `npm run lint`, `npx tsc -b` and `npm test`. All must be clean. Rust is untouched, so `cargo test` is needed only in Task 4.

## Review Focus

1. **A live refresh while the user is zoomed in.** A presence event or a scan changes a status while the user has panned or zoomed. Expected: colours update in place and the view does not jump back to "fit". It refits only when the set of node ids changes. Test: Task 2, `keeps the user's view when only statuses change`.
2. **A big network** (a /22 with 250 devices). Expected: no two nodes overlap, and the internet node sits clear of every ring. Test: Task 1, `ringPositions keeps 250 devices apart`.
3. **Phone width (375 px).** Expected:
   - labels hidden and nodes smaller;
   - a one-finger swipe over the map scrolls the page instead of being trapped by the canvas (panning off when compact);
   - no horizontal page scroll.

   Tests: Task 2, `compact mode hides labels and turns panning off`, and Task 4, the e2e test at a 375×812 viewport.
4. **No router among the devices.** The last scan missed it, or there are no devices yet but the network has a gateway. Expected: a "Router" node is still drawn at the center, labelled with the gateway IP or "Router", with the status text "Not found by the last scan". No crash, and the node is not clickable. Test: Task 1, `draws a router node when no device is the gateway`.
5. **Odd alert data.** Cases: alerts about a device id that is not in the list (deleted or merged), several alerts about one device, unknown key formats. Expected: unknown or orphan keys are ignored, and a device with several alerts shows the first alert's title. Test: Task 1, `ignores alerts about unknown devices and unknown keys`.

---

## File Structure

| File | Responsibility |
|---|---|
| `ui/src/lib/deviceStatus.ts` (new) | The presence status text shared by Devices and the map (moved out of `pages/Devices.tsx`). |
| `ui/src/lib/networkMap.ts` (new) | A pure model: statuses, problem detection from alert keys, filtering, ring layout, node and edge lists. |
| `ui/src/lib/networkMap.test.ts` (new) | Unit tests for the model. |
| `ui/src/components/MapCanvas.tsx` (new) | Wraps Cytoscape: token colours, compact mode, zoom buttons, tap to select, in-place updates. |
| `ui/src/components/MapCanvas.test.tsx` (new) | Tests against a mocked `cytoscape`. |
| `ui/src/pages/NetworkMap.tsx` (new) | The page: data hooks, filters, node list, details panel, note, empty and error states. |
| `ui/src/pages/NetworkMap.test.tsx` (new) | Page tests with `mockFetch` and a mocked `MapCanvas`. |
| `ui/src/App.tsx` (modify) | The `map` route lazy-loads `NetworkMap`. |
| `ui/src/pages/Devices.tsx` (modify) | Uses `statusText` from `lib/deviceStatus.ts`. |
| `ui/e2e/demo.spec.ts` (modify) | Map tests on desktop and phone viewports. |
| `docs/checkpoints/phase-5.md` (new) | The manual checklist for the author. |
| `docs/superpowers/specs/2026-10-31-netlantern-design.md` (modify) | §6.10 phase-5 notes and §6.12 map rows. |

---

### Task 1: Map model and shared status text

**Files:**
- Create: `ui/src/lib/deviceStatus.ts`, `ui/src/lib/networkMap.ts`, `ui/src/lib/networkMap.test.ts`
- Modify: `ui/src/pages/Devices.tsx` (delete its local `statusText` and import the shared one)
- Test: `ui/src/lib/networkMap.test.ts`, plus the existing `ui/src/pages/Devices.test.tsx` must still pass unchanged

**Interfaces:**
- Consumes:
  - `Device` from `api/queries.ts`;
  - `Alert` from `api/alerts.ts`;
  - `MonitorStatus` from `api/monitor.ts`;
  - `compareIp` from `lib/ip.ts`;
  - `formatAgo` from `lib/time.ts`.
- Produces:

```ts
// lib/deviceStatus.ts
export function statusText(d: Pick<Device, "online" | "answers_ping" | "last_seen">, now: number): string;

// lib/networkMap.ts
export type NodeStatus = "ok" | "warn" | "problem" | "offline" | "unknown";
export type MapFilter = "all" | "online" | "problems";
export interface MapNode {
  id: string;               // "internet" | "router" | `device:${id}`
  kind: "internet" | "router" | "device";
  deviceId: number | null;  // null for the internet node and for a router with no device
  label: string;
  status: NodeStatus;
  statusText: string;
  isNew: boolean;
  starred: boolean;
  ip: string | null;
  x: number;
  y: number;
}
export interface MapEdge { id: string; source: string; target: string; status: NodeStatus } // status = target's
export interface NetworkMap { nodes: MapNode[]; edges: MapEdge[]; counts: Record<MapFilter, number> }
export interface MapInput {
  devices: Device[];
  alerts: Alert[];          // the "active" list (open + acked)
  monitor: MonitorStatus | null;
  gatewayIp: string | null; // Network.gateway
  filter: MapFilter;
  now: number;
}
export function buildMap(input: MapInput): NetworkMap;
export function ringPositions(count: number): { x: number; y: number }[];
export function isProblem(n: MapNode): boolean; // status "problem" or "warn"
export const LAYOUT: { FIRST_RING: 130; RING_STEP: 110; SPACING: 72; GAP_DEG: 24; INTERNET_CLEARANCE: 120 };
```

**Rules that `buildMap` implements.** These are the decisions; the implementer writes the code.

- **Alerts per device.** Parse each active alert's `key`.
  - `starred_offline:<n>` and `packet_loss:device:<n>` mark device n as **problem**.
  - `new_device:<n>` marks device n as **warn**.
  - `router_down` and `packet_loss:gateway` mark the router as **problem**.
  - `internet_down` marks the internet as **problem**.
  - `latency_high` and `packet_loss:internet` mark the internet as **warn**.
  - Problem beats warn. The status text of an alerted node is the title of the first such alert, in the order the list gives. Any other key, or an id not in `devices`, is ignored.
- **Device status without an alert:**
  - starred and offline → problem, with `statusText(d, now)`;
  - offline → offline;
  - online → ok, whether or not it answers ping. The text says "doesn't answer ping", which is normal for phones, so it is not coloured as a warning.
- **The router node** is the device with `is_gateway`. If there is none, it is a node with `deviceId: null`, label `Router` (plus ` · <gatewayIp>` when one is known), status `unknown` and text "Not found by the last scan". A router alert still overrides the status.
- **The internet node:**
  - `monitor` is null or its `state` is not `"monitoring"` → unknown, "Not monitoring";
  - else an alert, as above;
  - else `last?.internet_rtt_ms` null → warn, "No reply";
  - else ok, `Online · ${ms} ms`.
- **Labels.** Device label = `display_name`. The `is_self` device's label = `This laptop`.
- **Filtering.** The router and internet nodes are always kept. Filters apply to device nodes only:
  - `online` keeps `online`;
  - `problems` keeps `isProblem`.

  `counts` are device-node counts for each filter, computed before filtering.
- **Order and positions.** Kept device nodes are ordered by `compareIp`, and ip-less devices go last by id. This order keeps positions stable when statuses change. They get `ringPositions(kept.length)`. The router is at (0,0). The internet is at (0, −(outermost ring radius + 120)); with no devices it uses `FIRST_RING` as the radius.
- **Edges.** `internet→router` with id `e:internet`, and `router→device:n` with id `e:device:n`.

**The `ringPositions` algorithm.** The y axis points down, so −90° is straight up.
- Ring k (k = 0, 1, …) has radius `r = 130 + 110k`.
- Its usable arc leaves a gap of `GAP_DEG = 24°` on each side of straight up, so the internet edge has a clear lane: the arc spans from −90°+24° to 270°−24°.
- Capacity is `floor(r × arcRadians / 72)`.
- Rings are filled in order. The n nodes in a ring sit at angles `start + (i + 0.5) × span / n`.
- Coordinates are rounded to whole numbers.

- [ ] **Step 1: Write the failing tests** in `ui/src/lib/networkMap.test.ts`. Use the `device`, `alert` and `monitorStatus` fixtures from `src/test/fixtures.ts`. The tests (names and assertions):
  - `puts the router at the center and the internet straight above`: with 3 devices, the router node is at (0,0). The internet node has x = 0 and y < every device's y minus 100.
  - `colors devices by presence`: an online device is ok with "Online". `answers_ping: false` is ok with "Online · doesn't answer ping". An offline device is offline. A starred offline device is problem.
  - `marks devices named by active alerts`: `starred_offline:2` → device 2 is problem, with the alert's title as text. `packet_loss:device:3` → problem. `new_device:4` → warn. `isProblem` is true for all three.
  - `marks the router and the internet from their alerts`: `router_down` → router problem. `latency_high` → internet warn. `internet_down` together with `latency_high` → internet problem.
  - `ignores alerts about unknown devices and unknown keys`: `starred_offline:999`, `packet_loss:dns` and `new_device:x` change nothing, and no node is added.
  - `draws a router node when no device is the gateway`: with `gatewayIp: "192.168.1.1"` and no `is_gateway` device, the router has `deviceId` null, label "Router · 192.168.1.1", status unknown and text "Not found by the last scan". With `gatewayIp: null` the label is "Router". With an empty device list there is still a router node and an internet node, and `counts.all` is 0.
  - `reads the internet from the monitor`: null monitor → unknown "Not monitoring". `state: "waiting_for_scan"` → unknown. `internet_rtt_ms: 18` → ok "Online · 18 ms". `internet_rtt_ms: null` → warn "No reply".
  - `filters devices but always keeps router and internet`: with a gateway device, 2 online devices and 1 starred offline device:
    - `all` keeps 3 device nodes, because the gateway is the router node and is never counted as a device;
    - `online` keeps 2;
    - `problems` keeps 1;
    - router and internet are present every time;
    - `counts` equals `{ all: 3, online: 2, problems: 1 }` whichever filter is active.
  - `keeps positions when only statuses change`: building twice with the same devices but different `online` values gives identical x/y for every node.
  - `calls the laptop "This laptop" and links edges from the router`: an `is_self` device is labelled "This laptop". There is edge `e:internet` from internet to router, and edge `e:device:7` from router to `device:7` carrying the device's status.
  - `ringPositions keeps 250 devices apart`: every pair of the 250 points is at least 72 × 0.9 apart, and no point is within 24° of straight up. That means for each (x, y), `Math.abs(Math.atan2(x, -y)) >= 24° − 0.5°` in radians.
  - `ringPositions fills the first ring first`: capacity of ring 0 = `floor(130 × (312 × π/180) / 72)` = 9. So `ringPositions(9)` has every radius equal to 130, and `ringPositions(10)` has one point at radius 240.
- [ ] **Step 2: Run them to make sure they fail.** Run (from `ui/`) `npx vitest run src/lib/networkMap.test.ts`. Expected: FAIL, because the module was not found.
- [ ] **Step 3: Move the status text out of Devices.** Create `lib/deviceStatus.ts` with `statusText(d, now)`. It returns the exact strings `pages/Devices.tsx` builds today, where `now` replaces the `Date.now()` call. Then delete the local copy in Devices and call `statusText(d, Date.now())`.
- [ ] **Step 4: Implement `lib/networkMap.ts`** following the rules above.
- [ ] **Step 5: Run the tests.** Run `npx vitest run src/lib/networkMap.test.ts src/pages/Devices.test.tsx`. Expected: PASS. Then run `npm run lint`, `npx tsc -b` and `npm test`, all clean.
- [ ] **Step 6: Commit.**

```bash
git add ui/src/lib/deviceStatus.ts ui/src/lib/networkMap.ts ui/src/lib/networkMap.test.ts ui/src/pages/Devices.tsx
git commit -m "Add the network map model that places the router, internet and devices and colours them from presence and active alerts."
```

---

### Task 2: Cytoscape canvas

**Files:**
- Modify: `ui/package.json` and `ui/package-lock.json` (add `cytoscape`)
- Create: `ui/src/components/MapCanvas.tsx`, `ui/src/components/MapCanvas.test.tsx`

**Interfaces:**
- Consumes: `NetworkMap`, `MapNode` and `NodeStatus` from Task 1.
- Produces:

```ts
export interface MapCanvasProps {
  map: NetworkMap;
  selectedId: string | null;          // a MapNode id
  onSelect: (node: MapNode) => void;  // fired only for nodes whose deviceId is not null
  label: string;                      // accessible summary, put on the canvas wrapper
}
export function MapCanvas(props: MapCanvasProps): JSX.Element;
export const COMPACT_BELOW = 480;     // px of container width
```

**Decisions:**

- **Install.** Run `npm install cytoscape@3` from `ui/` and keep the caret range npm writes. If `node_modules/cytoscape/package.json` has no `types` field, also run `npm install -D @types/cytoscape`. Say which in the report.
- **Creation.**
  - Create the instance once per mount with `cytoscape({ container, elements, style, layout: { name: "preset" }, minZoom: 0.3, maxZoom: 3 })`.
  - Also pass `userZoomingEnabled: false`, so wheel and pinch never trap page scroll, and `boxSelectionEnabled: false` and `autoungrabify: true`, because positions come from the model.
  - `userPanningEnabled` is `!compact`.
  - Destroy the instance on unmount.
- **Elements.**
  - A node's `data` is `{ id, label, status, isNew, selected }` and its `position` is `{ x, y }`, where `selected` is a boolean that is true when `id === selectedId`.
  - Edge data is `{ id, source, target, status }`.
  - Node `classes` are the node's `kind` (`internet`, `router` or `device`).
- **Style** is built from tokens read with `getComputedStyle(document.documentElement).getPropertyValue("--ok").trim()`, and so on.
  - In jsdom the values are empty, so fall back to the hex values in `styles/tokens.css`. The fallbacks are defined in one map in this file.
  - Fill colour by status: ok→`--ok`, warn→`--warn`, problem→`--bad`, offline→`--text-muted`, unknown→`--border-strong`.
  - `isNew` adds a 3 px `--accent` border.
  - `selected` adds a 3 px `--text` border, which wins over `isNew`.
  - Edges are 1.5 px `--border-strong`. Edges to offline nodes are `line-style: dashed`, and edges to problem nodes are coloured `--bad`.
  - Labels: the text is `--text` at 11 px, sits below the node and is wrapped to 120 px with ellipsis. The text background is `--bg` at 0.7 opacity.
- **Sizes:**

  | | Device | Router / internet | Labels |
  |---|---|---|---|
  | Desktop | 28 | 44 | shown |
  | Compact | 16 | 26 | hidden (`label: ""`), selected node's label still shown |

- **Compact** means the container is narrower than `COMPACT_BELOW`, measured by a `ResizeObserver` on the wrapper. On a change, re-apply the style and `userPanningEnabled`, call `cy.resize()`, then fit.
- **Updates.** When `map` or `selectedId` changes, call `cy.json({ elements })` with the new list and remove any element whose id is gone. Call `cy.fit(undefined, 24)` only when the set of node ids differs from the previous render. A status-only change must not move the view.
- **Taps.** Bind `cy.on("tap", "node", handler)` once. It looks the tapped id up in the latest `map` through a ref and calls `onSelect` only when `deviceId !== null`.
- **Zoom buttons** sit over the canvas, top-right. They are real `<button>`s, each at least 44 px, labelled "Zoom in", "Zoom out" and "Fit map".
  - Zoom in and out multiply `cy.zoom()` by 1.25 or 0.8 around the viewport centre, using `renderedPosition` = half of `cy.width()` and `cy.height()`.
  - Fit calls `cy.fit(undefined, 24)`.
- **The wrapper** has `role="img"` and `aria-label={label}`. Its height is `h-[420px]` at 760 px and above, and `h-[320px]` below, with `w-full`. It uses token classes only (`bg-surface`, `border-border`, `rounded-xl`).

- [ ] **Step 1: Write the failing tests** in `ui/src/components/MapCanvas.test.tsx`.
  - Use `vi.mock("cytoscape", …)` returning a factory that records its options and returns a fake with `on`, `json`, `elements` (with `remove`), `getElementById`, `fit`, `zoom`, `width`, `height`, `resize`, `style`, `userPanningEnabled` and `destroy`, all as `vi.fn`s.
  - Stub `ResizeObserver` on `globalThis`, and let tests fire it with a chosen `contentRect.width`.
  - Build maps with `buildMap` from Task 1.

  The tests:
  - `creates one instance with preset positions and no wheel zoom`: the factory is called once. Its options have `layout.name === "preset"`, `userZoomingEnabled === false` and `autoungrabify === true`. A device element carries the model's `position`.
  - `selecting a device node calls onSelect, the internet node does not`: invoke the recorded `tap` handler with `{ target: { id: () => "device:2" } }` → `onSelect` is called with the node whose `deviceId` is 2. With `"internet"` → not called.
  - `keeps the user's view when only statuses change`: re-render with the same devices but one set offline → `fit` was not called again and `json` was called with that node's `status: "offline"`. Then re-render with one device added → `fit` was called again.
  - `compact mode hides labels and turns panning off`: fire the observer with width 375 → `userPanningEnabled` was called with `false`, and the style passed to `style()` has the device node label set to `""`. Fire it with 900 → `userPanningEnabled(true)`.
  - `zoom buttons zoom around the centre and fit`: clicking "Zoom in" calls `zoom` with `{ level: current × 1.25, renderedPosition: { x: w/2, y: h/2 } }`. "Fit map" calls `fit`.
  - `destroys the instance on unmount`.
  - `exposes the summary as an image label`: `getByRole("img", { name: label })`.
- [ ] **Step 2: Run them to make sure they fail.** Run `npx vitest run src/components/MapCanvas.test.tsx`. Expected: FAIL, because the module was not found.
- [ ] **Step 3: Install Cytoscape and implement `components/MapCanvas.tsx`** with the decisions above.
- [ ] **Step 4: Run the tests.** Run `npx vitest run src/components/MapCanvas.test.tsx`. Expected: PASS. Then run `npm run lint`, `npx tsc -b` and `npm test`, all clean.
- [ ] **Step 5: Commit.**

```bash
git add ui/package.json ui/package-lock.json ui/src/components/MapCanvas.tsx ui/src/components/MapCanvas.test.tsx
git commit -m "Draw the network map with Cytoscape using the design tokens, a compact phone mode, zoom buttons and in-place status updates."
```

---

### Task 3: Network map page

**Files:**
- Create: `ui/src/pages/NetworkMap.tsx`, `ui/src/pages/NetworkMap.test.tsx`
- Modify: `ui/src/App.tsx` (the `map` route), `ui/src/App.test.tsx` (only if an existing route test needs `findBy…` for the lazy page)

**Interfaces:**
- Consumes:
  - `buildMap`, `isProblem`, `MapFilter` and `MapNode` from Task 1;
  - `MapCanvas` from Task 2;
  - `useDevices` and `useNetwork` from `api/queries.ts`;
  - `useAlerts("active")` from `api/alerts.ts`;
  - `useMonitor` from `api/monitor.ts`;
  - `DeviceDetails` from `components/DeviceDetails.tsx` (`{ id, onClose }`);
  - `OutdatedBanner`.
- Produces: `export function NetworkMap(): JSX.Element`, the default for the lazy import via `export default NetworkMap`.

**Decisions:**

- **Route.** In `App.tsx`, `const NetworkMap = lazy(() => import("./pages/NetworkMap"))`, and the `map` route renders `<Suspense fallback={<p className="m-0 text-text-muted">Loading the map…</p>}><NetworkMap /></Suspense>`. `Placeholder` stays for Settings.
- **Heading.** `<h1>` "Network map", then `<OutdatedBanner />`.
- **Honest note** (exact copy), a muted paragraph under the heading: "Every device is drawn straight to the router. Switches and mesh points between devices need access to the router, which is planned for later."
- **Data.**
  - Devices are required.
  - Alerts, monitor and network are optional. While they are loading or have failed, pass `[]`, `null` and `null`, so the map still draws.
  - `now` = `Date.now()` at render.
- **States:**
  - devices pending → "Loading devices…";
  - devices error → "Couldn't load devices: {message}";
  - an empty device list → "No devices yet — run a Quick scan from the Dashboard." The canvas is not shown.
- **Filters.** Three toggle buttons with `aria-pressed`: "All", "Online" and "Problems", each followed by its count from `map.counts`, in the same style as the Devices filter buttons. They live in local state, and the default is `all`. Under `problems` with a count of 0, show "No problems right now." in place of the list's device entries. The canvas still shows the router and the internet.
- **Selection.**
  - `?device=<id>`, the same search parameter and parsing as `pages/Devices.tsx`.
  - Selecting a node or a list entry sets it.
  - The `DeviceDetails` panel closes by deleting it.
  - The selected node id is `device:<id>`, or `router` when that id is the gateway device's.
- **Layout** mirrors Devices:
  - At 760 px and above, a row with the map and the list stacked on the left (`flex-1 min-w-0`) and `DeviceDetails` on the right.
  - Below that, the details panel goes first (`order-1`), then the map, then the list.
- **The canvas summary label**: `Network map: the router, the internet and ${n} ${n === 1 ? "device" : "devices"}; ${p} with a problem`. Here n is the number of device nodes shown, and p is `map.counts.problems`.
- **The node list** (`<section aria-label="Map devices">` with `<h2>` "On the map"):
  - One `<ul>` row per node: internet first, router second, then problems, then the rest in map order.
  - Each row shows a status dot (`aria-hidden`, token colour class per status: `bg-ok`, `bg-warn`, `bg-bad`, `bg-text-muted`, `bg-border-strong`), the label, the `ip` in mono when present, a "New" badge when `isNew`, and the `statusText`.
  - A device or router row with a `deviceId` is a `<button>` (min-h-11) that selects it. A router row without one is plain text.
  - The internet row has a `Link` to `/diagnose` with the text "Diagnose".

- [ ] **Step 1: Write the failing tests** in `ui/src/pages/NetworkMap.test.tsx`.
  - `vi.mock("../components/MapCanvas", …)` with a stub that renders `<div role="img" aria-label={label}>` and one `<button>` per node with `data-node={id}` that calls `onSelect(node)` when `deviceId !== null`.
  - Use `mockFetch` for `/api/devices`, `/api/alerts?state=active`, `/api/monitor` and `/api/network`, and `renderWithProviders(<NetworkMap />, "/map")`.

  The tests:
  - `shows the note, filters and every node in the list`: use the fixture from Task 1's filter test (a gateway device, 2 online devices and 1 starred offline device). Expect the heading, the exact note text, the buttons "All 3", "Online 2" and "Problems 1", and list rows for Internet, the router and 3 devices.
  - `problems only shows just the problem devices`: click "Problems" → the list holds Internet, Router and the starred offline device only, and the canvas label ends `1 with a problem`.
  - `says when there are no problems`: with no alerts and every device online, clicking "Problems" shows "No problems right now."
  - `opens the details panel from the list and from the map`: clicking the list button for a device shows the `DeviceDetails` panel (mock `/api/devices/2`), and the URL search has `device=2`. Clicking the stub map node `device:3` switches it to 3.
  - `still draws when alerts and monitor fail`: alerts and monitor return 500 → the list shows Internet with "Not monitoring", and the device rows render.
  - `says when there are no devices yet` and `says when devices fail to load`: the exact copy, and no `img` role.
  - `a router the scan did not find is listed but not clickable`: no `is_gateway` device, and the network gateway is `192.168.1.1` → the row "Router · 192.168.1.1" with "Not found by the last scan" exists, and it has no button.
  - In `App.test.tsx` (or here), `the map route loads the page lazily`: render `AppRoutes` at `/map` and `findByRole("heading", { name: "Network map" })`.
- [ ] **Step 2: Run them to make sure they fail.** Run `npx vitest run src/pages/NetworkMap.test.tsx`. Expected: FAIL, because the module was not found.
- [ ] **Step 3: Implement `pages/NetworkMap.tsx`** and switch the route in `App.tsx`.
- [ ] **Step 4: Run the tests.** Run `npx vitest run src/pages/NetworkMap.test.tsx src/App.test.tsx`. Expected: PASS. Then run `npm run lint`, `npx tsc -b`, `npm test` and `npm run build`, all clean. In the build output, Cytoscape must be in a separate chunk, not in the main `index-*.js`. Record both chunk sizes in the report.
- [ ] **Step 5: Commit.**

```bash
git add ui/src/pages/NetworkMap.tsx ui/src/pages/NetworkMap.test.tsx ui/src/App.tsx ui/src/App.test.tsx
git commit -m "Add the Network map screen with filters, a text list of every node, the device details panel and the note about switches and mesh points."
```

---

### Task 4: End-to-end tests, checkpoint and spec notes

**Files:**
- Modify: `ui/e2e/demo.spec.ts`
- Create: `docs/checkpoints/phase-5.md`
- Modify: `docs/superpowers/specs/2026-10-31-netlantern-design.md` (§6.10 and §6.12)

**Interfaces:**
- Consumes: the page from Task 3. The demo scenario `scenarios/home.json` has 13 devices once a quick scan has run, which the earlier tests in the file already do.

- [ ] **Step 1: Add the e2e tests** to the existing serial `describe` in `ui/e2e/demo.spec.ts`, after the diagnose test, reusing `page`.
  - `the map draws the demo network and opens device details`:
    - click the "Map" link;
    - expect the heading "Network map", and expect the `img` whose name starts "Network map: the router, the internet and 12 devices" to be visible. The demo finds 13 devices, and one of them is the router node;
    - in the "Map devices" list, click the button containing "Backup box" (renamed by an earlier test) and expect the details panel's Name field to have the value "Backup box";
    - click "Problems" and expect either "No problems right now." or a list without "Backup box" (the demo may have no active alerts).
  - `the map fits a phone screen`:
    - `page.setViewportSize({ width: 375, height: 812 })` and reload;
    - expect the map `img` to be visible and `document.documentElement.scrollWidth <= 375`;
    - expect the "Zoom in" button's bounding box height to be at least 44;
    - reset the viewport to 1280×800 afterwards.
- [ ] **Step 2: Run them.** Build the exe with the separate target dir, then run `npm run e2e` from `ui/`, the way the existing e2e is run (see `e2e/global-setup.ts`). Expected: all tests pass, old and new.
- [ ] **Step 3: Write the checkpoint** `docs/checkpoints/phase-5.md`. Use the same shape as `docs/checkpoints/phase-4.md`: the same build block with `CARGO_TARGET_DIR`, numbered items and blank "Result:" lines. The items:
  1. The map after a quick scan on the home network: the router is in the center, the internet is above it, and every device from the Devices screen is present.
  2. Colours and text agree: the laptop and router are online, and the unknown phone is online or offline as it really is.
  3. Click the router and a device on the map: the details panel opens. Close it.
  4. Problems only: unplug or switch off a starred device, wait for the offline alert (2 min, or 10 for phones), and confirm the map shows it as a problem without a reload.
  5. Live refresh: zoom in, wait for a monitor tick or run a quick scan, and confirm the view does not jump.
  6. Phone width: open the dashboard on the phone, or use the browser's 375 px device mode, then go to Map. Labels are hidden, nodes are smaller, the page scrolls with one finger over the map, and the zoom buttons work.
  7. The palette swap: set `data-palette="green-red"` on `<html>` in devtools and reload the map. The node colours follow the palette.
  8. A learning check (spec §13): explain in your own words why the map's positions come from a fixed ring layout and not Cytoscape's force layout, and why the canvas has a text list next to it.

  End with a "What to look at" note: Cytoscape's chunk size from Task 3, and that wheel zoom is off on purpose so the page scrolls.
- [ ] **Step 4: Update the spec.**
  - In §6.10, append: *(Phase 5: positions come from a fixed ring layout around the router, ordered by IP, so a status change never moves a node; a text list beside the map carries every node's status for screen readers and phones; wheel and pinch zoom are off so the page scrolls, with Zoom in / Zoom out / Fit map buttons instead; the internet node links to Diagnose; problems are devices named by an active alert or starred and offline.)*
  - In §6.12, add a "Network map" group with these rows, each marked *(changed in phase 5)*:
    - first ring radius / ring step / node spacing: 130 / 110 / 72;
    - gap above the router: 24° each side;
    - compact below: 480 px container width;
    - node size desktop / compact: 28 / 16, router and internet 44 / 26;
    - zoom range: 0.3 – 3.
- [ ] **Step 5: Final verification.**
  - From `ui/`: run `npm run lint`, `npx tsc -b`, `npm test`, `npm run build` and `npm run e2e`, all clean.
  - From the repo root: `cargo test --workspace` must be unchanged and passing, because the UI is embedded in the binary.
- [ ] **Step 6: Commit.**

```bash
git add ui/e2e/demo.spec.ts docs/checkpoints/phase-5.md docs/superpowers/specs/2026-10-31-netlantern-design.md
git commit -m "Add map end-to-end tests for desktop and phone width, the Phase 5 checkpoint, and the map notes and defaults in the spec."
```
