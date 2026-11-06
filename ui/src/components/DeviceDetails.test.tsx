import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Device } from "../api/queries";
import { Devices } from "../pages/Devices";
import { mockFetch, type FetchCall } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";
import { DeviceDetails } from "./DeviceDetails";

function dev(over: Partial<Device> = {}): Device {
  const id = over.id ?? 1;
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

function lastRequestBody(calls: FetchCall[], key: string): unknown {
  const [method, path] = key.split(" ");
  return calls.filter((c) => c.method === method && c.path === path).at(-1)?.body;
}

/** A one-device server whose PATCH really changes the device, so refetches agree with it. */
function statefulDevice(initial: Device): FetchCall[] {
  const state = { ...initial };
  const calls: FetchCall[] = [];
  const json = (status: number, body: unknown) =>
    new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = new URL(String(input), "http://localhost").pathname;
      const method = (init?.method ?? "GET").toUpperCase();
      const body = typeof init?.body === "string" ? JSON.parse(init.body) : undefined;
      calls.push({ method, path, body });
      if (path !== `/api/devices/${state.id}`) {
        return json(404, { code: "not_found", message: "Not found.", hint: null });
      }
      if (method === "PATCH") {
        Object.assign(state, body);
        if ("name" in body) state.display_name = body.name ?? state.hostname ?? state.ip ?? "";
      }
      return json(200, state);
    }),
  );
  return calls;
}

function renderDetails(id: number) {
  return renderWithProviders(<DeviceDetails id={id} onClose={() => {}} />);
}

function Where() {
  const l = useLocation();
  return <output data-testid="where">{l.pathname + l.search}</output>;
}

const user = userEvent.setup();

beforeEach(() => resetLive());

describe("DeviceDetails", () => {
  it("renames, then resets to the default name", async () => {
    const calls = statefulDevice(dev({ id: 7, hostname: "DiskStation", display_name: "DiskStation" }));
    renderDetails(7);
    const input = await screen.findByLabelText("Name");
    expect(input).toHaveAttribute("placeholder", "DiskStation");
    await user.clear(input);
    await user.type(input, "Backup box{Enter}");
    expect(await screen.findByRole("heading", { name: "Backup box" })).toBeInTheDocument();
    expect(lastRequestBody(calls, "PATCH /api/devices/7")).toEqual({ name: "Backup box" });

    await user.click(await screen.findByRole("button", { name: "Use default name" }));
    expect(lastRequestBody(calls, "PATCH /api/devices/7")).toEqual({ name: null });
  });

  it("shows the server's validation message", async () => {
    mockFetch({
      "GET /api/devices/7": [200, dev({ id: 7 })],
      "PATCH /api/devices/7": [
        400,
        { code: "invalid_name", message: "Names can be at most 64 characters.", hint: null },
      ],
    });
    renderDetails(7);
    await user.type(await screen.findByLabelText("Name"), "x{Enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent("Names can be at most 64 characters.");
  });

  it("Escape restores the input and an empty Save uses the default name", async () => {
    const calls = mockFetch({
      "GET /api/devices/7": [200, dev({ id: 7, name: "Old", display_name: "Old" })],
      "PATCH /api/devices/7": [200, dev({ id: 7 })],
    });
    renderDetails(7);
    const input = await screen.findByLabelText("Name");
    expect(input).toHaveValue("Old");
    await user.type(input, "zzz{Escape}");
    expect(input).toHaveValue("Old");
    await user.clear(input);
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(lastRequestBody(calls, "PATCH /api/devices/7")).toEqual({ name: null });
  });

  it("Mark as seen clears the New badge", async () => {
    const calls = statefulDevice(dev({ id: 7, is_new: true }));
    renderDetails(7);
    expect(await screen.findByText("New device")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Mark as seen" }));
    expect(lastRequestBody(calls, "PATCH /api/devices/7")).toEqual({ is_new: false });
    await waitFor(() => expect(screen.queryByText("New device")).not.toBeInTheDocument());
    expect(screen.queryByRole("button", { name: "Mark as seen" })).not.toBeInTheDocument();
  });

  it("stars from the panel", async () => {
    const calls = statefulDevice(dev({ id: 7 }));
    renderDetails(7);
    await user.click(await screen.findByRole("button", { name: /Star/ }));
    expect(lastRequestBody(calls, "PATCH /api/devices/7")).toEqual({ starred: true });
    expect(await screen.findByRole("button", { name: /Unstar/ })).toHaveAttribute("aria-pressed", "true");
  });

  it("shows the facts, with confidence wording and the router copy", async () => {
    mockFetch({
      "GET /api/devices/1": [
        200,
        dev({
          id: 1,
          type: "phone",
          type_confidence: 0.9,
          private_mac: true,
          open_ports: [80, 443],
          services: [{ source: "mdns", value: "_airplay._tcp" }],
        }),
      ],
      "GET /api/devices/2": [200, dev({ id: 2, type: "phone", type_confidence: 0.4 })],
    });
    const { unmount } = renderDetails(1);
    expect(await screen.findByText("Phone · fairly sure")).toBeInTheDocument();
    expect(screen.getByText("Private address — vendor hidden")).toBeInTheDocument();
    expect(screen.getByText("80, 443")).toBeInTheDocument();
    expect(screen.getByText("mdns: _airplay._tcp")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Don't recognise it? Net Lantern can't block devices. Check your router's client list, or change the Wi-Fi password.",
      ),
    ).toBeInTheDocument();
    unmount();
    renderDetails(2);
    expect(await screen.findByText("Phone · best guess")).toBeInTheDocument();
    expect(screen.getByText("none of the common ports")).toBeInTheDocument();
    expect(screen.getByText("Nothing announced")).toBeInTheDocument();
  });

  it("says so when the device does not exist", async () => {
    mockFetch({});
    renderDetails(99);
    expect(await screen.findByText("Device not found.")).toBeInTheDocument();
  });
});

describe("Devices panel wiring", () => {
  it("opens from the table, is addressable by URL, and closes", async () => {
    mockFetch({
      "GET /api/devices": [200, [dev({ id: 1 }), dev({ id: 2 })]],
      "GET /api/devices/1": [200, dev({ id: 1 })],
    });
    renderWithProviders(
      <>
        <Devices />
        <Where />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "192.168.1.1" }));
    expect(screen.getByTestId("where")).toHaveTextContent("?device=1");
    expect(await screen.findByRole("complementary", { name: "Device details" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Back to list" }));
    expect(screen.queryByRole("complementary", { name: "Device details" })).not.toBeInTheDocument();
    expect(screen.getByTestId("where")).not.toHaveTextContent("device=");
  });

  it("switching to another device starts with that device's own name field", async () => {
    mockFetch({
      "GET /api/devices": [200, [dev({ id: 1 }), dev({ id: 2 })]],
      "GET /api/devices/1": [200, dev({ id: 1 })],
      "GET /api/devices/2": [200, dev({ id: 2 })],
    });
    renderWithProviders(<Devices />);
    // Visit both once so each is cached and switching shows it at once.
    await user.click(await screen.findByRole("button", { name: "192.168.1.1" }));
    await screen.findByRole("heading", { name: "192.168.1.1" });
    await user.click(screen.getByRole("button", { name: "192.168.1.2" }));
    await screen.findByRole("heading", { name: "192.168.1.2" });
    await user.click(screen.getByRole("button", { name: "192.168.1.1" }));
    const input = await screen.findByLabelText("Name");
    await user.type(input, "abc");
    expect(input).toHaveValue("abc");
    await user.click(screen.getByRole("button", { name: "192.168.1.2" }));
    await screen.findByRole("heading", { name: "192.168.1.2" });
    expect(screen.getByLabelText("Name")).toHaveValue("");
  });

  it("opens straight from the URL", async () => {
    mockFetch({
      "GET /api/devices": [200, [dev({ id: 1 })]],
      "GET /api/devices/1": [200, dev({ id: 1 })],
    });
    renderWithProviders(<Devices />, "/devices?device=1");
    expect(await screen.findByRole("complementary", { name: "Device details" })).toBeInTheDocument();
  });

  it("ignores a non-numeric id", async () => {
    mockFetch({ "GET /api/devices": [200, [dev({ id: 1 })]] });
    renderWithProviders(<Devices />, "/devices?device=abc");
    await screen.findByRole("button", { name: "192.168.1.1" });
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
  });
});
