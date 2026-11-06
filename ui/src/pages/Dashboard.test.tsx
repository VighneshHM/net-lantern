import { act, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
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
};

function renderDashboard(routes: MockRoutes = {}) {
  const calls = mockFetch({ "GET /api/network": [200, NETWORK], ...routes });
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

  it("shows a disabled Deep scan button with its phase", () => {
    renderDashboard();
    expect(screen.getByRole("button", { name: /Deep scan/ })).toBeDisabled();
    expect(screen.getByText("Arrives in phase 4")).toBeInTheDocument();
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
});
