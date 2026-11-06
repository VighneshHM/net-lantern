import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Device } from "../api/queries";
import { mockFetch, type MockRoutes } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";
import { Devices } from "./Devices";

let nextId = 1;
function dev(over: Partial<Device> = {}): Device {
  const id = over.id ?? nextId++;
  const ip = over.ip === undefined ? `192.168.1.${id}` : over.ip;
  return {
    id,
    ip,
    mac: "aa:bb:cc:00:00:01",
    private_mac: false,
    vendor: null,
    hostname: null,
    name: null,
    display_name: ip ?? `Device ${id}`,
    type: "unknown",
    type_confidence: 0,
    starred: false,
    is_new: false,
    online: true,
    answers_ping: true,
    rtt_ms: null,
    is_self: false,
    is_gateway: false,
    first_seen: 0,
    last_seen: Date.now(),
    services: [],
    open_ports: [],
    ...over,
  };
}

function renderDevices(devices: Device[], extra: MockRoutes = {}) {
  const calls = mockFetch({ "GET /api/devices": [200, devices], ...extra });
  renderWithProviders(<Devices />);
  return calls;
}

/** A tiny server that remembers stars; `accept(id)` decides whether a PATCH succeeds. */
function statefulServer(devices: Device[], accept: (id: number) => boolean) {
  const state = devices.map((d) => ({ ...d }));
  const calls: { method: string; path: string; body: unknown }[] = [];
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(input), "http://localhost").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
      calls.push({ method, path, body });
      if (method === "GET" && path === "/api/devices") return json(200, state);
      const m = /^\/api\/devices\/(\d+)$/.exec(path);
      const d = m && state.find((x) => x.id === Number(m[1]));
      if (!d) return json(404, { code: "not_found", message: "Not found.", hint: null });
      if (method === "GET") return json(200, d);
      await new Promise((r) => setTimeout(r, 20));
      if (!accept(d.id)) return json(500, { code: "storage_error", message: "x", hint: null });
      Object.assign(d, body);
      return json(200, d);
    }),
  );
  return { calls };
}

function row(ip: string): HTMLElement {
  const cell = screen.getAllByRole("cell", { name: ip })[0];
  return cell.closest("tr")!;
}

beforeEach(() => {
  resetLive();
});

describe("Devices", () => {
  it("lists devices with text status and badges", async () => {
    renderDevices([
      dev({ ip: "192.168.1.1", is_gateway: true, rtt_ms: 2 }),
      dev({ ip: "192.168.1.88", answers_ping: false }),
    ]);
    expect(await screen.findByText("Online · doesn't answer ping")).toBeInTheDocument();
    expect(screen.getByText("Router")).toBeInTheDocument();
    expect(within(row("192.168.1.1")).getByText("Online")).toBeInTheDocument();
    expect(within(row("192.168.1.88")).getByText("Online · doesn't answer ping")).toBeInTheDocument();
  });

  it("has Star, Name, IP, MAC, Type, Latency and Status columns", async () => {
    renderDevices([dev({ ip: "192.168.1.5" })]);
    await screen.findByRole("table");
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toEqual([
      "Star",
      "Name",
      "IP",
      "MAC",
      "Type",
      "Latency",
      "Status",
    ]);
  });

  it("shows Private address for a private MAC, the MAC otherwise, and a dash when unknown", async () => {
    renderDevices([
      dev({ ip: "192.168.1.44", private_mac: true, mac: "da:a1:19:00:00:44" }),
      dev({ ip: "192.168.1.45", mac: "aa:bb:cc:dd:ee:45" }),
      dev({ ip: "192.168.1.46", mac: null }),
    ]);
    await screen.findByRole("table");
    expect(within(row("192.168.1.44")).getByText("Private address")).toBeInTheDocument();
    expect(within(row("192.168.1.44")).queryByText("da:a1:19:00:00:44")).toBeNull();
    expect(within(row("192.168.1.45")).getByText("aa:bb:cc:dd:ee:45")).toBeInTheDocument();
    expect(within(row("192.168.1.46")).getByText("—", { selector: "td:nth-child(4)" })).toBeInTheDocument();
  });

  it("shows latency in ms or a dash", async () => {
    renderDevices([dev({ ip: "192.168.1.1", rtt_ms: 12 }), dev({ ip: "192.168.1.2", rtt_ms: null })]);
    await screen.findByRole("table");
    expect(within(row("192.168.1.1")).getByText("12 ms")).toBeInTheDocument();
    expect(within(row("192.168.1.2")).getByText("—", { selector: "td:nth-child(6)" })).toBeInTheDocument();
  });

  it("marks this device", async () => {
    renderDevices([dev({ ip: "192.168.1.23", is_self: true })]);
    expect(await screen.findByText("This device")).toBeInTheDocument();
  });

  it("sorts rows by IP numerically", async () => {
    renderDevices([dev({ ip: "192.168.1.100" }), dev({ ip: "192.168.1.9" }), dev({ ip: "192.168.1.20" })]);
    await screen.findByRole("table");
    const ips = screen
      .getAllByRole("row")
      .slice(1)
      .map((r) => within(r).getAllByRole("cell")[2].textContent);
    expect(ips).toEqual(["192.168.1.9", "192.168.1.20", "192.168.1.100"]);
  });

  it("shows an empty state when there are no devices", async () => {
    renderDevices([]);
    expect(
      await screen.findByText("No devices yet — run a Quick scan from the Dashboard."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("filters by search and by Starred", async () => {
    const user = userEvent.setup();
    renderDevices([
      dev({ id: 1, display_name: "DiskStation", vendor: "Synology", starred: true }),
      dev({ id: 2, display_name: "Pixel-7", ip: "192.168.1.44" }),
    ]);
    await user.type(await screen.findByLabelText("Search devices"), "synol");
    expect(screen.getAllByRole("row")).toHaveLength(2);
    await user.clear(screen.getByLabelText("Search devices"));
    await user.click(screen.getByRole("button", { name: /Starred/ }));
    expect(screen.getByText("DiskStation")).toBeInTheDocument();
    expect(screen.queryByText("Pixel-7")).toBeNull();
  });

  it("searches IP, MAC and type label, and says when nothing matches", async () => {
    const user = userEvent.setup();
    renderDevices([
      dev({ id: 1, display_name: "A", ip: "10.0.0.5", mac: "11:22:33:44:55:66", type: "printer" }),
      dev({ id: 2, display_name: "B", ip: "10.0.0.6" }),
    ]);
    const box = await screen.findByLabelText("Search devices");
    await user.type(box, "PRINT");
    expect(screen.getAllByRole("row")).toHaveLength(2);
    await user.clear(box);
    await user.type(box, "10.0.0.6");
    expect(screen.getByText("B")).toBeInTheDocument();
    expect(screen.queryByText("A")).toBeNull();
    await user.clear(box);
    await user.type(box, "44:55");
    expect(screen.getByText("A")).toBeInTheDocument();
    await user.clear(box);
    await user.type(box, "zzz");
    expect(screen.getByText("No devices match “zzz”.")).toBeInTheDocument();
  });

  it("shows counts on the filters and filters Online and New", async () => {
    const user = userEvent.setup();
    renderDevices([
      dev({ id: 1, display_name: "On" }),
      dev({ id: 2, display_name: "Off", online: false }),
      dev({ id: 3, display_name: "Fresh", is_new: true }),
    ]);
    await screen.findByRole("table");
    expect(screen.getByRole("button", { name: "All 3" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Online 2" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Starred 0" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^New/ }));
    expect(screen.getByRole("button", { name: /^New/ })).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByText("On")).toBeNull();
    expect(screen.getByText("Fresh")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Online/ }));
    expect(screen.queryByText("Off")).toBeNull();
    expect(screen.getByText("On")).toBeInTheDocument();
  });

  it("stars a device optimistically and rolls back on error", async () => {
    const user = userEvent.setup();
    renderDevices([dev({ id: 2, display_name: "Pixel-7" })], {
      "PATCH /api/devices/2": [500, { code: "storage_error", message: "x", hint: null }],
    });
    await user.click(await screen.findByRole("button", { name: "Star Pixel-7" }));
    expect(await screen.findByRole("button", { name: "Star Pixel-7" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByRole("alert")).toHaveTextContent("x");
  });

  it("keeps the star after the server accepts it", async () => {
    const user = userEvent.setup();
    const server = statefulServer([dev({ id: 2, display_name: "Pixel-7" })], () => true);
    renderWithProviders(<Devices />);
    await user.click(await screen.findByRole("button", { name: "Star Pixel-7" }));
    expect(await screen.findByRole("button", { name: "Unstar Pixel-7" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(server.calls.find((c) => c.method === "PATCH")?.body).toEqual({ starred: true });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("rolls back only the failed device when two stars are in flight", async () => {
    const user = userEvent.setup();
    const server = statefulServer(
      [dev({ id: 1, display_name: "A-dev" }), dev({ id: 2, display_name: "B-dev" })],
      (id) => id === 2,
    );
    renderWithProviders(<Devices />);
    await user.click(await screen.findByRole("button", { name: "Star A-dev" }));
    await user.click(screen.getByRole("button", { name: "Star B-dev" }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Unstar B-dev" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Star A-dev" })).toHaveAttribute("aria-pressed", "false");
    expect(server.calls.filter((c) => c.method === "PATCH")).toHaveLength(2);
  });

  it("shows offline devices with last seen", async () => {
    renderDevices([dev({ ip: "192.168.1.9", online: false, last_seen: Date.now() - 5 * 60_000 })]);
    expect(await screen.findByText("Offline · last seen 5 min ago")).toBeInTheDocument();
  });

  it("sorts online first, then by IP, with devices without an IP last", async () => {
    renderDevices([
      dev({ id: 1, display_name: "NoIp", ip: null }),
      dev({ id: 2, display_name: "OffLow", ip: "192.168.1.2", online: false }),
      dev({ id: 3, display_name: "OnHigh", ip: "192.168.1.200" }),
      dev({ id: 4, display_name: "OnLow", ip: "192.168.1.3" }),
    ]);
    await screen.findByRole("table");
    const names = screen
      .getAllByRole("row")
      .slice(1)
      .map((r) => within(r).getAllByRole("cell")[1].querySelector("button")!.textContent);
    expect(names).toEqual(["OnLow", "OnHigh", "NoIp", "OffLow"]);
    expect(within(screen.getAllByRole("row")[3]).getAllByText("—").length).toBeGreaterThan(0);
  });

  it("shows name, badges, vendor and type", async () => {
    renderDevices([
      dev({ id: 1, display_name: "DiskStation", vendor: "Synology", type: "nas", is_new: true }),
    ]);
    const r = (await screen.findByText("DiskStation")).closest("tr")!;
    expect(within(r).getByText("New")).toBeInTheDocument();
    expect(within(r).getByText("Synology")).toBeInTheDocument();
    expect(within(r).getByText("NAS")).toBeInTheDocument();
    expect(within(r).getByRole("button", { name: "DiskStation" })).toBeInTheDocument();
  });
});
