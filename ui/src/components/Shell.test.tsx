import { beforeEach, describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { AppRoutes } from "../App";
import { alert } from "../test/fixtures";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";

function renderAt(path: string) {
  return renderWithProviders(<AppRoutes />, path);
}

beforeEach(() => {
  resetLive();
  mockFetch({ "GET /api/network": 200, "GET /api/devices": [200, []] });
});

describe("Shell", () => {
  it("shows six nav links and marks the active one", () => {
    renderAt("/devices");
    expect(screen.getAllByRole("link")).toHaveLength(6);
    expect(screen.getByRole("link", { name: "Devices" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Dashboard" })).not.toHaveAttribute("aria-current");
  });

  it("renders the page for the current route", () => {
    renderAt("/devices");
    expect(screen.getByRole("heading", { level: 1, name: "Devices" })).toBeInTheDocument();
  });

  it("shows the phase for placeholder pages", () => {
    renderAt("/map");
    expect(screen.getByRole("heading", { level: 1, name: "Map" })).toBeInTheDocument();
    expect(screen.getByText("Arrives in phase 5")).toBeInTheDocument();
  });

  it("does not make the brand a link", () => {
    renderAt("/");
    expect(screen.getByText("Net Lantern")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Dashboard" })).toHaveAttribute("aria-current", "page");
  });

  it("shows the active alert count in the nav", async () => {
    mockFetch({
      "GET /api/network": 200,
      "GET /api/devices": [200, []],
      "GET /api/alerts": [200, [alert({ id: 1 }), alert({ id: 2 })]],
    });
    renderAt("/devices");
    const link = await screen.findByRole("link", { name: "Alerts, 2 active" });
    expect(link).toHaveTextContent("Alerts (2)");
  });

  it("hides the count when there are no active alerts", async () => {
    mockFetch({
      "GET /api/network": 200,
      "GET /api/devices": [200, []],
      "GET /api/alerts": [200, []],
    });
    renderAt("/devices");
    expect(await screen.findByRole("link", { name: "Alerts" })).toHaveTextContent(/^Alerts$/);
  });
});
