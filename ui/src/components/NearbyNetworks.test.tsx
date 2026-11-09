import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { ChannelLoad, NearbyScan } from "../api/wifi";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { NearbyNetworks } from "./NearbyNetworks";

const ch = (channel: number, networks: number): ChannelLoad => ({ band: "2.4ghz", channel, networks });

const SCAN: NearbyScan = {
  scanned_at: 1,
  networks: [
    { ssid: "Lantern Home", bssid: "aa:bb:cc:dd:ee:01", rssi_dbm: -52, channel: 6, band: "2.4ghz", width_mhz: 20 },
    { ssid: "", bssid: "aa:bb:cc:dd:ee:02", rssi_dbm: -75, channel: 11, band: "2.4ghz", width_mhz: 20 },
    { ssid: "Neighbour", bssid: "aa:bb:cc:dd:ee:03", rssi_dbm: -60, channel: 6, band: "2.4ghz", width_mhz: 20 },
  ],
  congestion: {
    channels: [ch(1, 1), ch(6, 9), ch(11, 1)],
    current: {
      band: "2.4ghz",
      channel: 6,
      overlapping: 9,
      crowded: true,
      own_5ghz_visible: false,
      suggested_channel: 1,
    },
    advice: "Channel 6 is crowded. Try channel 1.",
  },
};
const CONNECTED = {
  interface_kind: "wifi",
  connection: {
    ssid: "Lantern Home",
    bssid: "aa:bb:cc:dd:ee:01",
    rssi_dbm: -52,
    channel: 6,
    band: "2.4ghz",
    width_mhz: 20,
    phy: "802.11n",
    rx_rate_mbps: null,
    tx_rate_mbps: null,
    security: "WPA2",
  },
};

describe("NearbyNetworks", () => {
  it("lists nearby networks, marks yours and hidden ones, and shows the advice", async () => {
    mockFetch({ "POST /api/wifi/scan": [200, SCAN], "GET /api/wifi": [200, CONNECTED] });
    renderWithProviders(<NearbyNetworks />);
    expect(await screen.findByText("(hidden network)")).toBeInTheDocument();
    const table = screen.getByRole("table");
    expect(within(table).getAllByRole("row")).toHaveLength(4);
    expect(await within(table).findByText("(you)")).toBeInTheDocument();
    expect(screen.getByText("Neighbour")).toBeInTheDocument();
    expect(screen.getByText("Channel 6 is crowded. Try channel 1.")).toBeInTheDocument();
  });

  it("labels the congestion chart for screen readers and marks your channel", async () => {
    mockFetch({ "POST /api/wifi/scan": [200, SCAN], "GET /api/wifi": [200, CONNECTED] });
    renderWithProviders(<NearbyNetworks />);
    const chart = await screen.findByRole("img", { name: /^2.4 GHz channels: .*channel 6 has 9 networks/ });
    expect(chart).toHaveAttribute("aria-label", expect.stringContaining("channel 1 has 1 network,"));
    expect(within(chart).getByText("Your channel")).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: /5 GHz channels/ })).toBeNull();
  });

  it("refresh scans again", async () => {
    const user = userEvent.setup();
    const calls = mockFetch({ "POST /api/wifi/scan": [200, SCAN], "GET /api/wifi": [200, CONNECTED] });
    renderWithProviders(<NearbyNetworks />);
    await screen.findByText("Neighbour");
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await screen.findByRole("button", { name: "Refresh" });
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(2);
  });

  it("no adapter shows the server message", async () => {
    mockFetch({
      "POST /api/wifi/scan": [503, { code: "wifi_unavailable", message: "No Wi-Fi adapter found.", hint: null }],
    });
    renderWithProviders(<NearbyNetworks />);
    expect(await screen.findByText("No Wi-Fi adapter found.")).toBeInTheDocument();
  });

  it("points to location access on 403", async () => {
    mockFetch({
      "POST /api/wifi/scan": [403, { code: "location_permission", message: "Hidden.", hint: null }],
    });
    renderWithProviders(<NearbyNetworks />);
    expect(
      await screen.findByText("Turn on location access (above) to see nearby networks."),
    ).toBeInTheDocument();
  });
});
