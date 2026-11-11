import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { Incident } from "../api/alerts";
import { alert } from "../test/fixtures";
import { mockFetch, type MockRoutes } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { Alerts } from "./Alerts";

const RULES = {
  starred_offline: true,
  internet_down: true,
  latency_high: true,
  packet_loss: true,
  new_device: true,
  notifications: true,
};

function base(extra: MockRoutes = {}): MockRoutes {
  return { "GET /api/rules": [200, RULES], ...extra };
}

describe("Alerts", () => {
  it("lists active alerts with severity text and acknowledges one", async () => {
    const now = Date.now();
    const open = alert({
      id: 1,
      title: "Internet is down",
      detail: "No reply",
      opened_at: now - 120_000,
    });
    const routes = base({
      "GET /api/alerts": [200, [open]],
      "POST /api/alerts/1/ack": [200, { ...open, state: "acked", acked_at: now }],
    });
    const calls = mockFetch(routes);
    renderWithProviders(<Alerts />);
    expect(await screen.findByRole("button", { name: "Internet is down" })).toBeInTheDocument();
    expect(screen.getByText("Critical")).toBeInTheDocument();
    expect(screen.getAllByText("No reply").length).toBeGreaterThan(0);
    routes["GET /api/alerts"] = [200, [{ ...open, state: "acked", acked_at: now }]];
    await userEvent.click(screen.getByRole("button", { name: "Acknowledge" }));
    expect(await screen.findByText(/· acknowledged/)).toBeInTheDocument();
    expect(calls.some((c) => c.method === "POST" && c.path === "/api/alerts/1/ack")).toBe(true);
    expect(screen.queryByRole("button", { name: "Acknowledge" })).toBeNull();
  });

  it("shows held-back alerts as such", async () => {
    mockFetch(base({ "GET /api/alerts": [200, [alert({ suppressed: true })]] }));
    renderWithProviders(<Alerts />);
    expect(await screen.findByText("Held back while the router is down")).toBeInTheDocument();
  });

  it("opens an incident timeline in order with labels", async () => {
    const root = alert({ id: 1, title: "Router is down" });
    const other = alert({ id: 2, title: "Printer offline", rule: "starred_offline" });
    const incident: Incident = {
      id: 1,
      opened_at: 1_000,
      resolved_at: null,
      root,
      alerts: [root, other],
      timeline: [
        { ts: 1_000, kind: "last_good", text: "Router answered", alert_id: null },
        { ts: 2_000, kind: "detected", text: "Two failed checks", alert_id: null },
        { ts: 3_000, kind: "alerted", text: "Alert opened", alert_id: 1 },
        { ts: 4_000, kind: "recovered", text: "Back online", alert_id: null },
      ],
    };
    mockFetch(base({ "GET /api/alerts": [200, [root]], "GET /api/incidents/1": [200, incident] }));
    renderWithProviders(<Alerts />);
    await userEvent.click(await screen.findByRole("button", { name: "Router is down" }));
    const list = await screen.findByRole("list", { name: "Incident timeline" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(4);
    expect(items[0]).toHaveTextContent("Last good check");
    expect(items[0]).toHaveTextContent("Router answered");
    expect(items[1]).toHaveTextContent("Detected");
    expect(items[2]).toHaveTextContent("Alert raised");
    expect(items[3]).toHaveTextContent("Recovered");
    expect(screen.getByRole("heading", { name: "Incident" })).toBeInTheDocument();
    expect(screen.getByText("Also affected")).toBeInTheDocument();
    expect(screen.getByText("Printer offline")).toBeInTheDocument();
  });

  it("says already resolved on a 409", async () => {
    mockFetch(
      base({
        "GET /api/alerts": [200, [alert()]],
        "POST /api/alerts/1/ack": [
          409,
          { code: "alert_resolved", message: "Alert already resolved.", hint: null },
        ],
      }),
    );
    renderWithProviders(<Alerts />);
    await userEvent.click(await screen.findByRole("button", { name: "Acknowledge" }));
    expect(await screen.findByText("Already resolved.")).toBeInTheDocument();
  });

  it("switches filter tabs", async () => {
    mockFetch(base({ "GET /api/alerts": [200, []] }));
    renderWithProviders(<Alerts />);
    expect(screen.getByRole("button", { name: "Active" })).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "Resolved" }));
    expect(screen.getByRole("button", { name: "Resolved" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Active" })).toHaveAttribute("aria-pressed", "false");
  });
});
