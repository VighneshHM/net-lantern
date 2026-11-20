import { beforeEach, describe, expect, it } from "vitest";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AppRoutes } from "../App";
import type { Network } from "../api/queries";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";

function network(over: Partial<Network> = {}): Network {
  return {
    ip: "192.168.1.20",
    prefix: 24,
    gateway: "192.168.1.1",
    dns_servers: [],
    kind: "wifi",
    mac: null,
    network_id: 1,
    scan_consent: true,
    category: "private",
    ssid: "HomeNet",
    replaced_router: null,
    ...over,
  };
}

const PHONE = {
  state: "off",
  reason: null,
  url: null,
  firewall: null,
  category: "private",
  demo: false,
};

beforeEach(() => {
  resetLive();
});

describe("Settings", () => {
  it("shows the four cards in order", async () => {
    mockFetch({ "GET /api/network": [200, network()], "GET /api/phone-access": [200, PHONE] });
    renderWithProviders(<AppRoutes />, "/settings");
    expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
    const names = (await screen.findAllByRole("region")).map((r) => r.getAttribute("aria-label"));
    expect(names).toEqual(["Phone access", "This network", "Alerts and notifications", "Data kept"]);
    expect(screen.getByRole("link", { name: "Alerts screen" })).toHaveAttribute("href", "/alerts");
    const kept = within(screen.getByRole("region", { name: "Data kept" }));
    expect(kept.getByText(/Raw samples/)).toHaveTextContent("24 hours");
    expect(kept.getByText(/5-minute averages/)).toHaveTextContent("30 days");
    expect(kept.getByText(/Online\/offline history/)).toHaveTextContent("90 days");
    expect(kept.getByText(/Logs/)).toHaveTextContent("7 days");
  });

  it("network_card_shows_category_and_withdraws_consent", async () => {
    const calls = mockFetch({
      "GET /api/network": [200, network({ category: "public" })],
      "GET /api/phone-access": [200, PHONE],
      "PUT /api/network/consent": 204,
    });
    renderWithProviders(<AppRoutes />, "/settings");
    const card = within(await screen.findByRole("region", { name: "This network" }));
    expect(await card.findByText("HomeNet")).toBeInTheDocument();
    expect(card.getByText("192.168.1.20")).toBeInTheDocument();
    expect(card.getByText(/Windows marks it/)).toHaveTextContent("Windows marks it Public");
    expect(card.getByText("Deep scans allowed on this network")).toBeInTheDocument();
    await userEvent.click(card.getByRole("button", { name: "Withdraw permission" }));
    await waitFor(() =>
      expect(calls.filter((c) => c.method === "PUT")).toEqual([
        { method: "PUT", path: "/api/network/consent", body: { granted: false } },
      ]),
    );
  });

  it("network card handles unknown name, unreadable category and no consent", async () => {
    mockFetch({
      "GET /api/network": [200, network({ ssid: null, category: null, scan_consent: false })],
      "GET /api/phone-access": [200, PHONE],
    });
    renderWithProviders(<AppRoutes />, "/settings");
    const card = within(await screen.findByRole("region", { name: "This network" }));
    expect(await card.findByText("Unknown name")).toBeInTheDocument();
    expect(card.getByText("Windows' network type couldn't be read.")).toBeInTheDocument();
    expect(
      card.getByText("Deep scans not allowed yet; you'll be asked before the first one."),
    ).toBeInTheDocument();
    expect(card.queryByRole("button", { name: "Withdraw permission" })).not.toBeInTheDocument();
  });

  it("settings_route_has_no_placeholder", async () => {
    mockFetch({ "GET /api/network": [200, network()], "GET /api/phone-access": [200, PHONE] });
    renderWithProviders(<AppRoutes />, "/settings");
    await screen.findByRole("region", { name: "Data kept" });
    expect(screen.queryByText(/Arrives in phase/)).not.toBeInTheDocument();
  });
});
