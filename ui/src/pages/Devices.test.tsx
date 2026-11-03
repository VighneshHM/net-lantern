import { screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { Device } from "../api/queries";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";
import { Devices } from "./Devices";

function dev(ip: string, over: Partial<Device> = {}): Device {
  return {
    ip,
    mac: "aa:bb:cc:00:00:01",
    private_mac: false,
    answers_ping: true,
    rtt_ms: null,
    is_self: false,
    is_gateway: false,
    ...over,
  };
}

function renderDevices(devices: Device[]) {
  mockFetch({ "GET /api/devices": [200, devices] });
  renderWithProviders(<Devices />);
}

function row(ip: string): HTMLElement {
  // The IP appears in the Name and IP columns, so find the row by its second cell.
  const cell = screen.getAllByRole("cell", { name: ip })[0];
  return cell.closest("tr")!;
}

beforeEach(() => {
  resetLive();
});

describe("Devices", () => {
  it("lists devices with text status and badges", async () => {
    renderDevices([
      dev("192.168.1.1", { is_gateway: true, rtt_ms: 2 }),
      dev("192.168.1.88", { answers_ping: false }),
    ]);
    expect(await screen.findByText("Online · doesn't answer ping")).toBeInTheDocument();
    expect(screen.getByText("Router")).toBeInTheDocument();
    expect(within(row("192.168.1.1")).getByText("Online")).toBeInTheDocument();
    expect(within(row("192.168.1.88")).getByText("Online · doesn't answer ping")).toBeInTheDocument();
  });

  it("has Name, IP, MAC, Latency and Status columns", async () => {
    renderDevices([dev("192.168.1.5")]);
    await screen.findByRole("table");
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toEqual([
      "Name",
      "IP",
      "MAC",
      "Latency",
      "Status",
    ]);
  });

  it("shows Private address for a private MAC, the MAC otherwise, and a dash when unknown", async () => {
    renderDevices([
      dev("192.168.1.44", { private_mac: true, mac: "da:a1:19:00:00:44" }),
      dev("192.168.1.45", { mac: "aa:bb:cc:dd:ee:45" }),
      dev("192.168.1.46", { mac: null }),
    ]);
    await screen.findByRole("table");
    expect(within(row("192.168.1.44")).getByText("Private address")).toBeInTheDocument();
    expect(within(row("192.168.1.44")).queryByText("da:a1:19:00:00:44")).toBeNull();
    expect(within(row("192.168.1.45")).getByText("aa:bb:cc:dd:ee:45")).toBeInTheDocument();
    expect(within(row("192.168.1.46")).getByText("—", { selector: "td:nth-child(3)" })).toBeInTheDocument();
  });

  it("shows latency in ms or a dash", async () => {
    renderDevices([dev("192.168.1.1", { rtt_ms: 12 }), dev("192.168.1.2", { rtt_ms: null })]);
    await screen.findByRole("table");
    expect(within(row("192.168.1.1")).getByText("12 ms")).toBeInTheDocument();
    expect(within(row("192.168.1.2")).getByText("—", { selector: "td:nth-child(4)" })).toBeInTheDocument();
  });

  it("marks this device", async () => {
    renderDevices([dev("192.168.1.23", { is_self: true })]);
    expect(await screen.findByText("This device")).toBeInTheDocument();
  });

  it("sorts rows by IP numerically", async () => {
    renderDevices([dev("192.168.1.100"), dev("192.168.1.9"), dev("192.168.1.20")]);
    await screen.findByRole("table");
    const ips = screen
      .getAllByRole("row")
      .slice(1)
      .map((r) => within(r).getAllByRole("cell")[1].textContent);
    expect(ips).toEqual(["192.168.1.9", "192.168.1.20", "192.168.1.100"]);
  });

  it("shows an empty state when there are no devices", async () => {
    renderDevices([]);
    expect(
      await screen.findByText("No devices yet — run a Quick scan from the Dashboard."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });
});
