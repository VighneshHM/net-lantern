import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLive } from "../state/live";
import { mockFetch, type MockRoutes } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";
import { Dashboard } from "./Dashboard";

const NETWORK = {
  ip: "192.168.1.23",
  prefix: 24,
  gateway: "192.168.1.1",
  dns_servers: ["192.168.1.1"],
  kind: "wifi",
  mac: "aa:bb:cc:dd:ee:ff",
  network_id: 1,
  scan_consent: true,
  category: "private",
  ssid: null,
  replaced_router: null,
};

const LATEST_SCAN = {
  id: 1,
  mode: "quick",
  state: "done",
  stage: "merge",
  done: 1,
  total: 1,
  devices_found: 2,
  error: null,
  isolated: false,
};

const NEW_ROUTER = "This looks like a network you know with a new router.";
const NEEDED = { code: "network_decision_needed", message: NEW_ROUTER, hint: null };
const NO_SCANS: [number, unknown] = [404, { code: "not_found", message: "No scans.", hint: null }];

function renderDashboard(routes: MockRoutes = {}) {
  const calls = mockFetch({
    "GET /api/network": [200, NETWORK],
    "GET /api/scans/latest": [200, LATEST_SCAN],
    ...routes,
  });
  renderWithProviders(<Dashboard />);
  return calls;
}

function progressEvent(scan_id: number, done: number, total: number, stage: string, seq = 1) {
  return { seq, api_version: 1, type: "scan.progress", data: { scan_id, done, total, stage } };
}

beforeEach(() => {
  resetLive();
});

describe("Dashboard", () => {
  it("shows the network card", async () => {
    renderDashboard();
    expect(
      await screen.findByText("192.168.1.23 · 192.168.1.0/24 · gateway 192.168.1.1"),
    ).toBeInTheDocument();
  });

  it("shows the Wi-Fi name", async () => {
    renderDashboard({
      "GET /api/wifi": [
        200,
        {
          interface_kind: "wifi",
          connection: {
            ssid: "Lantern Home",
            bssid: "x",
            rssi_dbm: -50,
            channel: 1,
            band: "2.4ghz",
            width_mhz: null,
            phy: "n",
            rx_rate_mbps: null,
            tx_rate_mbps: null,
            security: "WPA2",
          },
        },
      ],
    });
    expect(await screen.findByText("Wi-Fi: Lantern Home")).toBeInTheDocument();
  });

  it("adds no Wi-Fi line when Wi-Fi details are unavailable", async () => {
    renderDashboard({
      "GET /api/wifi": [403, { code: "location_permission", message: "Hidden.", hint: null }],
    });
    await screen.findByText("192.168.1.23 · 192.168.1.0/24 · gateway 192.168.1.1");
    expect(screen.queryByText(/Wi-Fi:/)).toBeNull();
  });

  it("shows Not connected, with the server's hint, on 503", async () => {
    renderDashboard({
      "GET /api/network": [
        503,
        { code: "not_connected", message: "No network.", hint: "Connect to Wi-Fi and try again." },
      ],
    });
    expect(await screen.findByText("Not connected")).toBeInTheDocument();
    expect(screen.getByText("Connect to Wi-Fi and try again.")).toBeInTheDocument();
  });

  it("Quick scan attaches to the running scan on 409, without an error", async () => {
    const user = userEvent.setup();
    renderDashboard({
      "POST /api/scans": [
        409,
        { code: "scan_in_progress", message: "A scan is running.", hint: null, scan_id: 7 },
      ],
    });
    await user.click(screen.getByRole("button", { name: "Quick scan" }));
    expect(useLive.getState().scan?.id).toBe(7);
    expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("Quick scan posts mode quick and attaches to the new scan on 202", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({ "POST /api/scans": [202, { scan_id: 3 }] });
    await user.click(screen.getByRole("button", { name: "Quick scan" }));
    expect(useLive.getState().scan?.id).toBe(3);
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ mode: "quick" });
    expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();
  });

  it("shows the server's message in an alert when the scan cannot start", async () => {
    const user = userEvent.setup();
    renderDashboard({
      "POST /api/scans": [503, { code: "not_connected", message: "No network.", hint: null }],
    });
    await user.click(screen.getByRole("button", { name: "Quick scan" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("No network.");
    expect(screen.getByRole("button", { name: "Quick scan" })).toBeEnabled();
    expect(useLive.getState().scan).toBeNull();
  });

  it("deep scan asks for consent first, then starts", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({
      "GET /api/network": [200, { ...NETWORK, scan_consent: false }],
      "PUT /api/network/consent": [200, { network_id: 1, scan_consent: true }],
      "POST /api/scans": [202, { scan_id: 8 }],
    });
    await screen.findByText(/192\.168\.1\.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    const dialog = screen.getByRole("dialog", {
      name: "Is this a network you own or have permission to scan?",
    });
    expect(dialog).toHaveTextContent("A deep scan checks 100 common ports on every device");
    expect(calls.some((c) => c.method === "POST" && c.path === "/api/scans")).toBe(false);

    await user.click(screen.getByRole("button", { name: "Yes, I have permission" }));
    await waitFor(() => expect(useLive.getState().scan?.id).toBe(8));
    const writes = calls.filter((c) => c.method !== "GET");
    expect(writes[0]).toMatchObject({ method: "PUT", path: "/api/network/consent", body: { granted: true } });
    expect(writes[1]).toMatchObject({ method: "POST", path: "/api/scans", body: { mode: "deep" } });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("\"Not now\" closes the prompt without scanning", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({
      "GET /api/network": [200, { ...NETWORK, scan_consent: false }],
    });
    await screen.findByText(/192\.168\.1\.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.some((c) => c.method !== "GET")).toBe(false);
  });

  it("deep scan with consent starts straight away", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({ "POST /api/scans": [202, { scan_id: 9 }] });
    await screen.findByText(/192\.168\.1\.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    await waitFor(() => expect(useLive.getState().scan?.id).toBe(9));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ mode: "deep" });
  });

  it("a consent_needed answer shows the prompt", async () => {
    const user = userEvent.setup();
    renderDashboard({
      "POST /api/scans": [
        403,
        { code: "consent_needed", message: "Confirm you may scan this network.", hint: null },
      ],
    });
    await screen.findByText(/192\.168\.1\.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("moves focus into the consent prompt, and back to Deep scan when it closes", async () => {
    const user = userEvent.setup();
    renderDashboard({ "GET /api/network": [200, { ...NETWORK, scan_consent: false }] });
    await screen.findByText(/192\.168\.1\.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    const dialog = screen.getByRole("dialog");
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
    await user.click(screen.getByRole("button", { name: "Not now" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.getByRole("button", { name: "Deep scan" })).toHaveFocus();
  });

  it("reopens the prompt when a deep scan finds it needs permission for this network", async () => {
    const calls = renderDashboard();
    await screen.findByText(/192\.168\.1\.23/);
    const networkReads = () =>
      calls.filter((c) => c.method === "GET" && c.path === "/api/network").length;
    const before = networkReads();
    act(() => {
      useLive.getState().applyEvent(progressEvent(6, 1, 10, "sweep"));
      useLive.getState().applyEvent({
        seq: 2,
        api_version: 1,
        type: "scan.done",
        data: {
          scan_id: 6,
          state: "failed",
          devices_found: 0,
          error: "Deep scan needs your permission for this network.",
        },
      });
    });
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    await waitFor(() => expect(networkReads()).toBeGreaterThan(before));
  });

  it("other failures do not open the prompt", async () => {
    renderDashboard();
    await screen.findByText(/192\.168\.1\.23/);
    act(() =>
      useLive.getState().setScan({
        id: 6, state: "failed", stage: "sweep", done: 0, total: 0, devicesFound: 0,
        error: "No route", isolated: false,
      }),
    );
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("stop clears an earlier error", async () => {
    const user = userEvent.setup();
    renderDashboard({
      "POST /api/scans": [503, { code: "not_connected", message: "No network.", hint: null }],
      "POST /api/scans/7/cancel": 202,
    });
    await user.click(screen.getByRole("button", { name: "Quick scan" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("No network.");
    act(() => useLive.getState().applyEvent(progressEvent(7, 1, 10, "port_scan")));
    await user.click(screen.getByRole("button", { name: "Stop scan" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("stop scan cancels the running scan", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({ "POST /api/scans/7/cancel": 202 });
    act(() => useLive.getState().applyEvent(progressEvent(7, 1, 10, "port_scan")));
    await user.click(screen.getByRole("button", { name: "Stop scan" }));
    await waitFor(() =>
      expect(calls.some((c) => c.method === "POST" && c.path === "/api/scans/7/cancel")).toBe(true),
    );
    expect(screen.getByRole("button", { name: "Stopping…" })).toBeDisabled();
    act(() =>
      useLive.getState().applyEvent({
        seq: 3,
        api_version: 1,
        type: "scan.done",
        data: { scan_id: 7, state: "cancelled", devices_found: 0 },
      }),
    );
    expect(screen.queryByRole("button", { name: "Stopping…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Stop scan" })).toBeNull();
  });

  it("port scan progress shows probe counts", () => {
    renderDashboard();
    act(() => useLive.getState().applyEvent(progressEvent(4, 640, 2000, "port_scan")));
    expect(screen.getByText("Scanning ports · 640 of 2000")).toBeInTheDocument();
    act(() => useLive.getState().applyEvent(progressEvent(4, 3, 40, "banners", 2)));
    expect(screen.getByText("Reading what open ports say · 3 of 40")).toBeInTheDocument();
  });

  it("a network change shows its reason", () => {
    renderDashboard();
    act(() =>
      useLive.getState().setScan({
        id: 4, state: "cancelled", stage: "port_scan", done: 1, total: 2, devicesFound: 0,
        error: "The network changed during the scan.", isolated: false,
      }),
    );
    expect(
      screen.getByText("The scan stopped: The network changed during the scan."),
    ).toBeInTheDocument();
    act(() => {
      useLive.setState({ scan: null });
      useLive.getState().setScan({
        id: 5, state: "cancelled", stage: "sweep", done: 0, total: 0, devicesFound: 0, error: null, isolated: false,
      });
    });
    expect(screen.getByText("The scan was cancelled.")).toBeInTheDocument();
  });

  it("shows no progress bar before a scan", () => {
    renderDashboard();
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("follows live progress, then shows the device count and re-enables Quick scan", async () => {
    renderDashboard();
    act(() => useLive.getState().applyEvent(progressEvent(4, 50, 200, "sweep")));
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "25");
    expect(bar).toHaveAttribute("aria-valuemin", "0");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
    expect(screen.getByText(/Pinging addresses/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();

    act(() =>
      useLive.getState().applyEvent({
        seq: 2,
        api_version: 1,
        type: "scan.done",
        data: { scan_id: 4, state: "done", devices_found: 13 },
      }),
    );
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "100");
    expect(screen.getByText("13 devices found")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Quick scan" })).toBeEnabled();
  });

  it("says when a scan failed", () => {
    renderDashboard();
    act(() => {
      useLive.getState().applyEvent(progressEvent(4, 1, 10, "sweep"));
      useLive.getState().applyEvent({
        seq: 2,
        api_version: 1,
        type: "scan.done",
        data: { scan_id: 4, state: "failed", devices_found: 0 },
      });
    });
    expect(screen.getByText("The scan failed.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Quick scan" })).toBeEnabled();
  });

  it("shows the new stage labels and the count found so far", () => {
    renderDashboard();
    act(() => {
      useLive.getState().applyEvent(progressEvent(4, 1, 5, "ports"));
      useLive.getState().applyEvent({ seq: 2, api_version: 1, type: "scan.device", data: { scan_id: 4 } });
    });
    expect(screen.getByText(/Checking common ports/)).toBeInTheDocument();
    expect(screen.getByText(/1 found so far/)).toBeInTheDocument();
    act(() => useLive.getState().applyEvent(progressEvent(4, 1, 5, "discover", 3)));
    expect(screen.getByText(/Asking devices their names/)).toBeInTheDocument();
    act(() => useLive.getState().applyEvent(progressEvent(4, 1, 5, "merge", 4)));
    expect(screen.getByText(/Identifying devices/)).toBeInTheDocument();
  });

  it("shows isolation and failure reasons", () => {
    renderDashboard();
    act(() => {
      useLive.getState().setScan({
        id: 4, state: "done", stage: "merge", done: 1, total: 1, devicesFound: 1, error: null, isolated: true,
      });
    });
    expect(screen.getByText("1 device found")).toBeInTheDocument();
    expect(
      screen.getByText(
        "This network appears to isolate devices — only the router answered, so other devices can't be seen from here.",
      ),
    ).toBeInTheDocument();
    act(() => {
      useLive.setState({ scan: null });
      useLive.getState().setScan({
        id: 5, state: "failed", stage: "sweep", done: 0, total: 0, devicesFound: 0, error: "No route", isolated: false,
      });
    });
    expect(screen.getByText("The scan failed: No route")).toBeInTheDocument();
    expect(screen.queryByText(/isolate devices/)).toBeNull();
  });

  it("shows the out-of-date banner only when outdated", () => {
    renderDashboard();
    expect(screen.queryByText("This dashboard is out of date — reload")).toBeNull();
    act(() => useLive.setState({ outdated: true }));
    expect(screen.getByText("This dashboard is out of date — reload")).toBeInTheDocument();
  });

  it("asks for a scan before monitoring", async () => {
    renderDashboard({
      "GET /api/monitor": [
        200,
        { state: "waiting_for_scan", network_id: null, session_started_at: null, last: null, checks: [] },
      ],
    });
    expect(
      await screen.findByText("Run a quick scan to start monitoring this network."),
    ).toBeInTheDocument();
  });

  it("says since when it is monitoring", async () => {
    const since = new Date(2026, 0, 1, 9, 5).getTime();
    renderDashboard({
      "GET /api/monitor": [
        200,
        { state: "monitoring", network_id: 1, session_started_at: since, last: null, checks: [] },
      ],
    });
    const t = new Date(since).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    expect(await screen.findByText(`Monitoring since ${t}`)).toBeInTheDocument();
  });

  it("says when it is not connected", async () => {
    renderDashboard({
      "GET /api/monitor": [
        200,
        { state: "not_connected", network_id: null, session_started_at: null, last: null, checks: [] },
      ],
    });
    expect(await screen.findByText("Not monitoring: not connected.")).toBeInTheDocument();
  });
});

describe("first run and the replaced router", () => {
  it("first_run_shows_for_a_network_with_no_scans_and_starts_a_quick_scan", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({
      "GET /api/scans/latest": NO_SCANS,
      "GET /api/network": [200, { ...NETWORK, ssid: "Lantern Home" }],
      "POST /api/scans": [202, { scan_id: 3 }],
    });
    const card = await screen.findByRole("region", { name: "Network detected" });
    expect(card).toHaveTextContent("Lantern Home");
    expect(card).toHaveTextContent("Windows marks this network Private");
    expect(card).toHaveTextContent("Run your first scan to see the devices on this network.");
    expect(card).toHaveTextContent("A deep scan asks for your permission first.");
    // No duplicate scan buttons while the card shows.
    expect(screen.getAllByRole("button", { name: /quick scan/i })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Deep scan" })).toBeNull();

    await user.click(screen.getByRole("button", { name: "Run quick scan" }));
    expect(calls.find((c) => c.method === "POST")?.body).toEqual({ mode: "quick" });
    await waitFor(() => expect(useLive.getState().scan?.id).toBe(3));
  });

  it("first_run_shows_without_a_saved_network_and_names_the_subnet", async () => {
    renderDashboard({ "GET /api/network": [200, { ...NETWORK, network_id: null }] });
    const card = await screen.findByRole("region", { name: "Network detected" });
    expect(card).toHaveTextContent("192.168.1.0/24");
  });

  it("first_run_mentions_public_networks_only_when_public", async () => {
    renderDashboard({
      "GET /api/scans/latest": NO_SCANS,
      "GET /api/network": [200, { ...NETWORK, category: "public" }],
    });
    const card = await screen.findByRole("region", { name: "Network detected" });
    expect(card).toHaveTextContent("Windows marks this network Public");
    expect(card).toHaveTextContent("Some features, like phone access, stay off on Public networks.");
  });

  it("first_run_omits_the_public_sentence_on_a_private_network", async () => {
    renderDashboard({ "GET /api/scans/latest": NO_SCANS });
    const card = await screen.findByRole("region", { name: "Network detected" });
    expect(card).not.toHaveTextContent("stay off on Public networks");
  });

  it("first_run_hides_once_a_scan_runs", async () => {
    renderDashboard({ "GET /api/scans/latest": NO_SCANS });
    await screen.findByRole("region", { name: "Network detected" });
    act(() => useLive.getState().applyEvent(progressEvent(4, 1, 10, "sweep")));
    expect(screen.queryByRole("region", { name: "Network detected" })).toBeNull();
    expect(screen.getByRole("button", { name: "Scanning…" })).toBeDisabled();
  });

  it("no_first_run_card_when_the_network_has_a_scan", async () => {
    renderDashboard();
    await screen.findByText(/192\.168\.1\.23/);
    await screen.findByRole("button", { name: "Quick scan" });
    expect(screen.queryByRole("region", { name: "Network detected" })).toBeNull();
  });

  it("router_prompt_from_network_field_keep_and_decline_post_the_answer", async () => {
    const user = userEvent.setup();
    const replaced = { network_id: 2, label: "Lantern Home" };
    const calls = renderDashboard({
      "GET /api/network": [200, { ...NETWORK, replaced_router: replaced }],
      "POST /api/network/replaced-router": [200, { network_id: 2 }],
    });
    const prompt = await screen.findByRole("alertdialog");
    expect(prompt).toHaveTextContent(
      "This looks like Lantern Home with a new router. Keep your devices and names?",
    );
    expect(prompt).toContainElement(document.activeElement as HTMLElement);
    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    await user.click(screen.getByRole("button", { name: "It's a different network" }));
    const posts = calls.filter((c) => c.method === "POST");
    expect(posts.map((c) => c.body)).toEqual([{ keep: true }, { keep: false }]);
    expect(posts.every((c) => c.path === "/api/network/replaced-router")).toBe(true);
  });

  it("blocked_scan_start_shows_the_prompt_and_retries_the_same_mode_after_keep", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({
      "POST /api/network/replaced-router": [200, { network_id: 2 }],
      "POST /api/scans": [202, { scan_id: 12 }],
    });
    const base = globalThis.fetch;
    let blocked = false;
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
      const isStart =
        (init?.method ?? "GET") === "POST" &&
        new URL(String(input), "http://localhost").pathname === "/api/scans";
      if (isStart && !blocked) {
        blocked = true;
        return Promise.resolve(
          new Response(JSON.stringify(NEEDED), {
            status: 409,
            headers: { "content-type": "application/json" },
          }),
        );
      }
      return base(input, init);
    });
    await screen.findByText(/192\.168\.1\.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    const prompt = await screen.findByRole("alertdialog");
    expect(prompt).toHaveTextContent("with a new router");
    expect(screen.queryByRole("alert")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    await waitFor(() => expect(useLive.getState().scan?.id).toBe(12));
    const writes = calls.filter((c) => c.method !== "GET");
    expect(writes[0]).toMatchObject({ path: "/api/network/replaced-router", body: { keep: true } });
    expect(writes[1]).toMatchObject({ path: "/api/scans", body: { mode: "deep" } });
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("scan_failed_with_new_router_text_shows_the_prompt_once", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({
      "POST /api/network/replaced-router": [200, { network_id: 2 }],
    });
    await screen.findByText(/192\.168\.1\.23/);
    const failed = {
      id: 6, state: "failed", stage: "sweep", done: 0, total: 0, devicesFound: 0,
      error: NEW_ROUTER, isolated: false,
    };
    act(() => useLive.getState().setScan(failed));
    expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "It's a different network" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls.filter((c) => c.method === "POST").map((c) => c.body)).toEqual([{ keep: false }]);
    // The same failed scan arriving again must not reopen it.
    act(() => useLive.getState().setScan({ ...failed, done: 1 }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  function blockFirstStart() {
    const base = globalThis.fetch;
    let blocked = false;
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
      const isStart =
        (init?.method ?? "GET") === "POST" &&
        new URL(String(input), "http://localhost").pathname === "/api/scans";
      if (isStart && !blocked) {
        blocked = true;
        return Promise.resolve(
          new Response(JSON.stringify(NEEDED), {
            status: 409,
            headers: { "content-type": "application/json" },
          }),
        );
      }
      return base(input, init);
    });
  }

  it("answer_404_no_decision_pending_closes_the_prompt_and_restarts_the_blocked_scan", async () => {
    const user = userEvent.setup();
    const calls = renderDashboard({
      "POST /api/network/replaced-router": [
        404,
        { code: "no_decision_pending", message: "Nothing to decide.", hint: null },
      ],
      "POST /api/scans": [202, { scan_id: 15 }],
    });
    blockFirstStart();
    await screen.findByText(/192.168.1.23/);
    await user.click(screen.getByRole("button", { name: "Deep scan" }));
    await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    await waitFor(() => expect(useLive.getState().scan?.id).toBe(15));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls.filter((c) => c.method === "POST").at(-1)).toMatchObject({
      path: "/api/scans",
      body: { mode: "deep" },
    });
  });

  it("a failed answer keeps the prompt with an alert inside it, and a retry works", async () => {
    const user = userEvent.setup();
    const replaced = { network_id: 2, label: "Lantern Home" };
    let fail = true;
    let answered = false;
    renderDashboard({
      "GET /api/network": [200, { ...NETWORK, replaced_router: replaced }],
    });
    const base = globalThis.fetch;
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
      if ((init?.method ?? "GET") === "GET" && String(input).endsWith("/api/network") && answered) {
        return Promise.resolve(
          new Response(JSON.stringify(NETWORK), {
            status: 200,
            headers: { "content-type": "application/json" },
          }),
        );
      }
      if ((init?.method ?? "GET") === "POST" && String(input).includes("replaced-router")) {
        const ok = !fail;
        answered = ok;
        return Promise.resolve(
          new Response(
            JSON.stringify(ok ? { network_id: 2 } : { code: "internal", message: "Disk full.", hint: null }),
            { status: ok ? 200 : 500, headers: { "content-type": "application/json" } },
          ),
        );
      }
      return base(input, init);
    });
    const prompt = await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    expect(await within(prompt).findByRole("alert")).toHaveTextContent("Disk full.");
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });

  it("returns focus to the Quick scan button after the prompt is answered", async () => {
    const user = userEvent.setup();
    let answered = false;
    renderDashboard({
      "GET /api/network": [200, { ...NETWORK, replaced_router: { network_id: 2, label: "Home" } }],
      "POST /api/network/replaced-router": [200, { network_id: 2 }],
    });
    const base = globalThis.fetch;
    vi.stubGlobal("fetch", (input: RequestInfo | URL, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      if (method === "GET" && String(input).endsWith("/api/network") && answered) {
        return Promise.resolve(
          new Response(JSON.stringify(NETWORK), {
            status: 200,
            headers: { "content-type": "application/json" },
          }),
        );
      }
      if (method === "POST") answered = true;
      return base(input, init);
    });
    await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Quick scan" })).toHaveFocus();
  });

  it("answering a failed-scan prompt clears the obsolete new-router message", async () => {
    const user = userEvent.setup();
    renderDashboard({ "POST /api/network/replaced-router": [200, { network_id: 2 }] });
    await screen.findByText(/192.168.1.23/);
    act(() =>
      useLive.getState().setScan({
        id: 6, state: "failed", stage: "sweep", done: 0, total: 0, devicesFound: 0,
        error: NEW_ROUTER, isolated: false,
      }),
    );
    await screen.findByRole("alertdialog");
    await user.click(screen.getByRole("button", { name: "Keep my devices" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(screen.queryByText(/with a new router/)).toBeNull();
    expect(useLive.getState().scan).toBeNull();
    expect(screen.getByRole("button", { name: "Quick scan" })).toHaveFocus();
  });

  it("refreshes the latest scan when a scan ends", async () => {
    const calls = renderDashboard();
    await screen.findByText(/192.168.1.23/);
    const reads = () => calls.filter((c) => c.path === "/api/scans/latest").length;
    const before = reads();
    act(() => {
      useLive.getState().applyEvent(progressEvent(4, 1, 10, "sweep"));
      useLive.getState().applyEvent({
        seq: 2, api_version: 1, type: "scan.done",
        data: { scan_id: 4, state: "done", devices_found: 1 },
      });
    });
    await waitFor(() => expect(reads()).toBeGreaterThan(before));
  });
});
