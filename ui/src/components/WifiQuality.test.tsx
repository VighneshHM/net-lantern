import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { WifiConnection } from "../api/wifi";
import { mockFetch, type FetchCall } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { WifiQuality } from "./WifiQuality";

function conn(over: Partial<WifiConnection> = {}): WifiConnection {
  return {
    ssid: "Lantern Home",
    bssid: "aa:bb:cc:dd:ee:01",
    rssi_dbm: -52,
    channel: 36,
    band: "5ghz",
    width_mhz: 80,
    phy: "802.11ax",
    rx_rate_mbps: 866,
    tx_rate_mbps: 700,
    security: "WPA2-Personal",
    ...over,
  };
}

const count = (calls: FetchCall[], method: string, path: string) =>
  calls.filter((c) => c.method === method && c.path === path).length;

describe("WifiQuality", () => {
  it("shows the current connection with a signal label", async () => {
    mockFetch({ "GET /api/wifi": [200, { interface_kind: "wifi", connection: conn() }] });
    renderWithProviders(<WifiQuality />);
    expect(await screen.findByText("Lantern Home")).toBeInTheDocument();
    expect(screen.getByText("-52 dBm · Good")).toBeInTheDocument();
    expect(screen.getByText("36 · 80 MHz wide")).toBeInTheDocument();
    expect(screen.getByText("866 Mbps down · 700 Mbps up")).toBeInTheDocument();
    expect(screen.getByText("5 GHz")).toBeInTheDocument();
  });

  it("leaves out an unknown width and link speed", async () => {
    mockFetch({
      "GET /api/wifi": [
        200,
        { interface_kind: "wifi", connection: conn({ width_mhz: null, rx_rate_mbps: null }) },
      ],
    });
    renderWithProviders(<WifiQuality />);
    expect(await screen.findByText("36")).toBeInTheDocument();
    expect(screen.getByText("700 Mbps up")).toBeInTheDocument();
  });

  it("walks the user through location permission", async () => {
    const user = userEvent.setup();
    const calls = mockFetch({
      "GET /api/wifi": [
        403,
        { code: "location_permission", message: "Windows is hiding Wi-Fi details from Net Lantern.", hint: "x" },
      ],
    });
    renderWithProviders(<WifiQuality />);
    expect(
      await screen.findByRole("heading", { name: "Windows is hiding Wi-Fi details" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open location settings" })).toHaveAttribute(
      "href",
      "ms-settings:privacy-location",
    );
    expect(screen.getByText("Turn on Location services.")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Check again" }));
    await screen.findByRole("button", { name: "Check again" });
    expect(count(calls, "GET", "/api/wifi")).toBe(2);
  });

  it("explains Ethernet", async () => {
    mockFetch({ "GET /api/wifi": [200, { interface_kind: "ethernet", connection: null }] });
    renderWithProviders(<WifiQuality />);
    expect(
      await screen.findByText("Connected by cable — Wi-Fi details don't apply."),
    ).toBeInTheDocument();
  });

  it("says when not connected to Wi-Fi", async () => {
    mockFetch({ "GET /api/wifi": [200, { interface_kind: "wifi", connection: null }] });
    renderWithProviders(<WifiQuality />);
    expect(await screen.findByText("Not connected to Wi-Fi.")).toBeInTheDocument();
  });

  it("shows the server message for other errors", async () => {
    mockFetch({
      "GET /api/wifi": [503, { code: "wifi_unavailable", message: "No Wi-Fi adapter found.", hint: null }],
    });
    renderWithProviders(<WifiQuality />);
    expect(await screen.findByText("No Wi-Fi adapter found.")).toBeInTheDocument();
  });
});
