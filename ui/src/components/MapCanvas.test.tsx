import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { device, monitorStatus } from "../test/fixtures";
import { buildMap } from "../lib/networkMap";
import { MapCanvas } from "./MapCanvas";

interface Fake {
  opts: Record<string, any>; // eslint-disable-line @typescript-eslint/no-explicit-any
  on: ReturnType<typeof vi.fn>;
  json: ReturnType<typeof vi.fn>;
  elements: ReturnType<typeof vi.fn>;
  getElementById: ReturnType<typeof vi.fn>;
  fit: ReturnType<typeof vi.fn>;
  zoom: ReturnType<typeof vi.fn>;
  width: ReturnType<typeof vi.fn>;
  height: ReturnType<typeof vi.fn>;
  resize: ReturnType<typeof vi.fn>;
  style: ReturnType<typeof vi.fn>;
  userPanningEnabled: ReturnType<typeof vi.fn>;
  destroy: ReturnType<typeof vi.fn>;
}

const h = vi.hoisted(() => ({
  made: [] as unknown[],
  observers: [] as { cb: (e: unknown[]) => void }[],
}));
const made = h.made as Fake[];

vi.mock("cytoscape", () => ({
  default: (opts: Record<string, unknown>) => {
    const fake = {
      opts,
      on: vi.fn(),
      json: vi.fn(),
      elements: vi.fn(() => ({ remove: vi.fn() })),
      getElementById: vi.fn(() => ({ remove: vi.fn() })),
      fit: vi.fn(),
      zoom: vi.fn(() => 2),
      width: vi.fn(() => 600),
      height: vi.fn(() => 400),
      resize: vi.fn(),
      style: vi.fn(),
      userPanningEnabled: vi.fn(),
      destroy: vi.fn(),
    };
    h.made.push(fake);
    return fake;
  },
}));

class FakeResizeObserver {
  cb: (e: unknown[]) => void;
  constructor(cb: (e: unknown[]) => void) {
    this.cb = cb;
    h.observers.push(this);
  }
  observe() {}
  unobserve() {}
  disconnect() {}
}

function resizeTo(width: number) {
  act(() => {
    for (const o of h.observers) o.cb([{ contentRect: { width } }]);
  });
}

const NOW = 10_000;
function mapOf(
  devices = [
    device({ id: 1, ip: "192.168.1.10" }),
    device({ id: 2, ip: "192.168.1.11" }),
  ],
) {
  return buildMap({
    devices,
    alerts: [],
    monitor: monitorStatus(),
    gatewayIp: null,
    filter: "all",
    now: NOW,
  });
}

beforeEach(() => {
  made.length = 0;
  h.observers.length = 0;
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
});

describe("MapCanvas", () => {
  it("creates one instance with preset positions and no wheel zoom", () => {
    const map = mapOf();
    render(
      <MapCanvas map={map} selectedId={null} onSelect={() => {}} label="Map" />,
    );
    expect(made).toHaveLength(1);
    const o = made[0].opts;
    expect(o.layout.name).toBe("preset");
    expect(o.userZoomingEnabled).toBe(false);
    expect(o.autoungrabify).toBe(true);
    const n = map.nodes.find((x) => x.id === "device:1")!;
    const el = o.elements.find(
      (e: { data: { id: string } }) => e.data.id === "device:1",
    );
    expect(el.position).toEqual({ x: n.x, y: n.y });
  });

  it("selecting a device node calls onSelect, the internet node does not", () => {
    const onSelect = vi.fn();
    render(
      <MapCanvas
        map={mapOf()}
        selectedId={null}
        onSelect={onSelect}
        label="Map"
      />,
    );
    const handler = made[0].on.mock.calls.find((c) => c[0] === "tap")![2];
    handler({ target: { id: () => "internet" } });
    expect(onSelect).not.toHaveBeenCalled();
    handler({ target: { id: () => "device:2" } });
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onSelect.mock.calls[0][0].deviceId).toBe(2);
  });

  it("keeps the user's view when only statuses change", () => {
    const a = device({ id: 1, ip: "192.168.1.10" });
    const b = device({ id: 2, ip: "192.168.1.11" });
    const { rerender } = render(
      <MapCanvas
        map={mapOf([a, b])}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    made[0].fit.mockClear();
    made[0].json.mockClear();
    rerender(
      <MapCanvas
        map={mapOf([a, { ...b, online: false }])}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    expect(made[0].fit).not.toHaveBeenCalled();
    const sent = made[0].json.mock.calls.at(-1)![0].elements;
    expect(
      sent.find((e: { data: { id: string } }) => e.data.id === "device:2").data
        .status,
    ).toBe("offline");
    rerender(
      <MapCanvas
        map={mapOf([a, b, device({ id: 3, ip: "192.168.1.12" })])}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    expect(made[0].fit).toHaveBeenCalled();
  });

  it("compact mode hides labels and turns panning off", () => {
    render(
      <MapCanvas
        map={mapOf()}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    resizeTo(375);
    expect(made[0].userPanningEnabled).toHaveBeenLastCalledWith(false);
    const sheet = made[0].style.mock.calls.at(-1)![0] as {
      selector: string;
      style: Record<string, unknown>;
    }[];
    const dev = sheet.find((r) => r.selector === "node.device")!;
    expect(dev.style.label).toBe("");
    resizeTo(900);
    expect(made[0].userPanningEnabled).toHaveBeenLastCalledWith(true);
  });

  it("zoom buttons zoom around the centre and fit", async () => {
    const user = userEvent.setup();
    render(
      <MapCanvas
        map={mapOf()}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    await user.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(made[0].zoom).toHaveBeenCalledWith({
      level: 2 * 1.25,
      renderedPosition: { x: 300, y: 200 },
    });
    await user.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(made[0].zoom).toHaveBeenCalledWith({
      level: 2 * 0.8,
      renderedPosition: { x: 300, y: 200 },
    });
    made[0].fit.mockClear();
    await user.click(screen.getByRole("button", { name: "Fit map" }));
    expect(made[0].fit).toHaveBeenCalled();
  });

  it("destroys the instance on unmount", () => {
    const { unmount } = render(
      <MapCanvas
        map={mapOf()}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    unmount();
    expect(made[0].destroy).toHaveBeenCalled();
  });

  it("exposes the summary as an image label", () => {
    render(
      <MapCanvas
        map={mapOf()}
        selectedId={null}
        onSelect={() => {}}
        label="3 devices"
      />,
    );
    expect(screen.getByRole("img", { name: "3 devices" })).toBeInTheDocument();
  });

  it("keeps the zoom buttons outside the image element", () => {
    render(
      <MapCanvas
        map={mapOf()}
        selectedId={null}
        onSelect={() => {}}
        label="Map"
      />,
    );
    const img = screen.getByRole("img", { name: "Map" });
    for (const name of ["Zoom in", "Zoom out", "Fit map"]) {
      expect(img).not.toContainElement(screen.getByRole("button", { name }));
    }
  });
});
