import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useLocation } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Device, Network } from "../api/queries";
import type { MapCanvasProps } from "../components/MapCanvas";
import { alert, device, monitorStatus, tick } from "../test/fixtures";
import { mockFetch, type MockRoutes } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";
import { AppRoutes } from "../App";
import { NetworkMap } from "./NetworkMap";

vi.mock("../components/MapCanvas", () => ({
  MapCanvas: ({ map, onSelect, label }: MapCanvasProps) => (
    <div>
      <div role="img" aria-label={label} />
      {map.nodes.map((n) => (
        <button
          key={n.id}
          type="button"
          data-node={n.id}
          onClick={() => {
            if (n.deviceId !== null) onSelect(n);
          }}
        >
          {`canvas ${n.id}`}
        </button>
      ))}
    </div>
  ),
}));

function Where() {
  return <output data-testid="where">{useLocation().search}</output>;
}

const network: Network = {
  ip: "192.168.1.20",
  prefix: 24,
  gateway: "192.168.1.1",
  dns_servers: [],
  kind: "wifi",
  mac: null,
  network_id: 1,
  scan_consent: false,
};

function fleet(): Device[] {
  return [
    device({ id: 9, ip: "192.168.1.1", display_name: "Router box", is_gateway: true }),
    device({ id: 1, ip: "192.168.1.10", display_name: "Laptop" }),
    device({ id: 2, ip: "192.168.1.11", display_name: "Printer" }),
    device({ id: 3, ip: "192.168.1.12", display_name: "Phone", online: false, starred: true }),
  ];
}

function renderMap(devices: Device[] | [number, unknown], extra: MockRoutes = {}, path = "/map") {
  const routes: MockRoutes = {
    "GET /api/devices": Array.isArray(devices) && typeof devices[0] === "number"
      ? (devices as [number, unknown])
      : [200, devices],
    "GET /api/alerts": [200, []],
    "GET /api/monitor": [200, monitorStatus({ last: tick({ internet_rtt_ms: 12 }) })],
    "GET /api/network": [200, network],
    ...extra,
  };
  const calls = mockFetch(routes);
  const view = renderWithProviders(
    <>
      <NetworkMap />
      <Where />
    </>,
    path,
  );
  return { calls, ...view };
}

async function list() {
  return within(await screen.findByRole("region", { name: "Map devices" }));
}

beforeEach(() => {
  resetLive();
});

describe("NetworkMap", () => {
  it("shows the note, filters and every node in the list", async () => {
    renderMap(fleet(), {
      "GET /api/monitor": [200, monitorStatus({ last: { ...monitorStatus().last!, internet_rtt_ms: 12 } })],
    });
    expect(screen.getByRole("heading", { name: "Network map" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "Every device is drawn straight to the router. Switches and mesh points between devices need access to the router, which is planned for later.",
      ),
    ).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "All 3" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Online 2" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Problems 1" })).toBeInTheDocument();
    const rows = (await list()).getAllByRole("listitem");
    expect(rows).toHaveLength(5);
    expect(rows[0]).toHaveTextContent("Internet");
    expect(rows[1]).toHaveTextContent("Router box");
    expect(rows[2]).toHaveTextContent("Phone");
    expect(rows[3]).toHaveTextContent("Laptop");
    expect(rows[4]).toHaveTextContent("Printer");
    expect(within(rows[0]).getByRole("link", { name: "Diagnose" })).toHaveAttribute(
      "href",
      "/diagnose",
    );
  });

  it("problems only shows just the problem devices", async () => {
    const user = userEvent.setup();
    renderMap(fleet());
    await user.click(await screen.findByRole("button", { name: "Problems 1" }));
    expect(screen.getByRole("button", { name: "Problems 1" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    const rows = (await list()).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent("Internet");
    expect(rows[1]).toHaveTextContent("Router box");
    expect(rows[2]).toHaveTextContent("Phone");
    expect(screen.getByRole("img")).toHaveAccessibleName(/1 with a problem$/);
  });

  it("says when there are no problems", async () => {
    const user = userEvent.setup();
    renderMap([
      device({ id: 9, ip: "192.168.1.1", is_gateway: true }),
      device({ id: 1, ip: "192.168.1.10", display_name: "Laptop" }),
    ]);
    await user.click(await screen.findByRole("button", { name: "Problems 0" }));
    expect(screen.getByText("No problems right now.")).toBeInTheDocument();
    expect(screen.getByRole("img")).toBeInTheDocument();
  });

  it("labels the canvas with the device and problem counts", async () => {
    renderMap(fleet());
    expect(await screen.findByRole("img")).toHaveAccessibleName(
      "Network map: the router, the internet and 3 devices; 1 with a problem",
    );
  });

  it("opens the details panel from the list and from the map", async () => {
    const user = userEvent.setup();
    const devices = fleet();
    renderMap(devices, {
      "GET /api/devices/2": [200, devices[2]],
      "GET /api/devices/3": [200, devices[3]],
    });
    await user.click(await (await list()).findByRole("button", { name: "Printer" }));
    expect(await screen.findByRole("heading", { level: 2, name: "Printer" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Device details" })).toBeInTheDocument();
    expect(screen.getByTestId("where")).toHaveTextContent("?device=2");
    await user.click(document.querySelector('[data-node="device:3"]')!);
    expect(await screen.findByRole("heading", { level: 2, name: "Phone" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { level: 2, name: "Printer" })).toBeNull();
    expect(screen.getByTestId("where")).toHaveTextContent("?device=3");
  });

  it("closes the panel by dropping the search parameter", async () => {
    const devices = fleet();
    renderMap(devices, { "GET /api/devices/2": [200, devices[2]] }, "/map?device=2");
    expect(await screen.findByRole("heading", { level: 2, name: "Printer" })).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent(/^$/));
  });

  it("still draws when alerts and monitor fail", async () => {
    renderMap(fleet(), { "GET /api/alerts": 500, "GET /api/monitor": 500 });
    const rows = await (await list()).findAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("Internet");
    expect(rows[0]).toHaveTextContent("Not monitoring");
    expect(rows).toHaveLength(5);
  });

  it("marks alerted devices", async () => {
    renderMap(fleet(), {
      "GET /api/alerts": [
        200,
        [alert({ key: "new_device:1", rule: "new_device", title: "New: Laptop" })],
      ],
    });
    expect(await (await list()).findByText("New: Laptop")).toBeInTheDocument();
  });

  it("says when there are no devices yet", async () => {
    renderMap([]);
    expect(
      await screen.findByText("No devices yet — run a Quick scan from the Dashboard."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("says when devices fail to load", async () => {
    renderMap([500, { code: "storage_error", message: "disk on fire", hint: null }]);
    expect(await screen.findByText(/Couldn't load devices: disk on fire/)).toBeInTheDocument();
    expect(screen.queryByRole("img")).toBeNull();
  });

  it("a router the scan did not find is listed but not clickable", async () => {
    renderMap([device({ id: 1, ip: "192.168.1.10", display_name: "Laptop" })]);
    const label = await (await list()).findByText("Router · 192.168.1.1");
    const row = label.closest("li")!;
    expect(row).toHaveTextContent("Not found by the last scan");
    expect(within(row).queryByRole("button")).toBeNull();
  });

  it("the map route loads the page lazily", async () => {
    mockFetch({
      "GET /api/devices": [200, fleet()],
      "GET /api/alerts": [200, []],
      "GET /api/monitor": [200, monitorStatus()],
      "GET /api/network": [200, network],
    });
    renderWithProviders(<AppRoutes />, "/map");
    expect(await screen.findByRole("heading", { name: "Network map" })).toBeInTheDocument();
  });

  it("does not say there are no problems while the internet is down", async () => {
    const user = userEvent.setup();
    renderMap(
      [
        device({ id: 9, ip: "192.168.1.1", is_gateway: true }),
        device({ id: 1, ip: "192.168.1.10", display_name: "Laptop" }),
      ],
      { "GET /api/alerts": [200, [alert({ key: "internet_down", title: "Internet is down" })]] },
    );
    await user.click(await screen.findByRole("button", { name: "Problems 0" }));
    expect(screen.queryByText("No problems right now.")).toBeNull();
    expect(screen.getByRole("img")).toHaveAccessibleName(/; 1 with a problem$/);
  });

  it("says 0 with a problem when nothing is wrong anywhere", async () => {
    const user = userEvent.setup();
    renderMap([device({ id: 9, ip: "192.168.1.1", is_gateway: true })]);
    await user.click(await screen.findByRole("button", { name: "Problems 0" }));
    expect(screen.getByText("No problems right now.")).toBeInTheDocument();
    expect(screen.getByRole("img")).toHaveAccessibleName(/; 0 with a problem$/);
  });

  it("scrolls the details panel into view when a device is selected", async () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    try {
      const user = userEvent.setup();
      const devices = fleet();
      renderMap(devices, { "GET /api/devices/2": [200, devices[2]] });
      await user.click(await (await list()).findByRole("button", { name: "Printer" }));
      await screen.findByRole("heading", { level: 2, name: "Printer" });
      expect(scroll).toHaveBeenCalledTimes(1);
    } finally {
      // @ts-expect-error jsdom has no scrollIntoView; put it back to absent
      delete Element.prototype.scrollIntoView;
    }
  });

  it("does not scroll when nothing is selected", async () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    try {
      renderMap(fleet());
      await (await list()).findAllByRole("listitem");
      expect(scroll).not.toHaveBeenCalled();
    } finally {
      // @ts-expect-error jsdom has no scrollIntoView; put it back to absent
      delete Element.prototype.scrollIntoView;
    }
  });
});
