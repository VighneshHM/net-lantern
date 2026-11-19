import { describe, expect, it } from "vitest";
import { alert, device, monitorStatus, tick } from "../test/fixtures";
import { buildMap, isProblem, ringPositions, type MapInput } from "./networkMap";

const NOW = 10_000;

function input(over: Partial<MapInput> = {}): MapInput {
  return {
    devices: [],
    alerts: [],
    monitor: monitorStatus(),
    gatewayIp: null,
    filter: "all",
    now: NOW,
    ...over,
  };
}

function node(m: ReturnType<typeof buildMap>, id: string) {
  const n = m.nodes.find((x) => x.id === id);
  if (!n) throw new Error(`no node ${id}`);
  return n;
}

const gw = () =>
  device({
    id: 100,
    ip: "192.168.1.1",
    display_name: "Router",
    is_gateway: true,
  });

describe("buildMap", () => {
  it("colours the router from alerts keyed by the gateway device's own id", () => {
    const m = buildMap(
      input({
        devices: [device({ id: 5, ip: "192.168.1.1", is_gateway: true })],
        alerts: [alert({ id: 1, key: "packet_loss:device:5", title: "Router is losing packets" })],
      }),
    );
    expect(node(m, "router")).toMatchObject({
      status: "problem",
      statusText: "Router is losing packets",
    });
  });

  it("keeps the first alert's title when two alerts hit one device", () => {
    const m = buildMap(
      input({
        devices: [device({ id: 2, ip: "192.168.1.11" })],
        alerts: [
          alert({ id: 1, key: "starred_offline:2", title: "First" }),
          alert({ id: 2, key: "packet_loss:device:2", title: "Second" }),
        ],
      }),
    );
    expect(node(m, "device:2")).toMatchObject({ status: "problem", statusText: "First" });
  });

  it("puts the router at the center and the internet straight above", () => {
    const m = buildMap(
      input({
        devices: [
          gw(),
          device({ id: 1, ip: "192.168.1.10" }),
          device({ id: 2, ip: "192.168.1.11" }),
          device({ id: 3, ip: "192.168.1.12" }),
        ],
      }),
    );
    const r = node(m, "router");
    expect([r.x, r.y]).toEqual([0, 0]);
    const net = node(m, "internet");
    expect(net.x).toBe(0);
    for (const n of m.nodes.filter((x) => x.kind === "device")) {
      expect(net.y).toBeLessThan(n.y - 60);
    }
  });

  it("hangs the internet 60 above the outermost ring", () => {
    const devices = Array.from({ length: 12 }, (_, i) => device({ id: i + 1, ip: `192.168.1.${i + 10}` }));
    const m = buildMap(input({ devices }));
    const outer = Math.max(
      ...m.nodes.filter((n) => n.kind === "device").map((n) => Math.round(Math.hypot(n.x, n.y))),
    );
    expect(outer).toBe(240);
    expect(node(m, "internet").y).toBe(-(240 + 60));
  });

  it("colors devices by presence", () => {
    const m = buildMap(
      input({
        devices: [
          device({ id: 1, ip: "192.168.1.10" }),
          device({ id: 2, ip: "192.168.1.11", answers_ping: false }),
          device({ id: 3, ip: "192.168.1.12", online: false }),
          device({ id: 4, ip: "192.168.1.13", online: false, starred: true }),
        ],
      }),
    );
    expect(node(m, "device:1")).toMatchObject({
      status: "ok",
      statusText: "Online",
    });
    expect(node(m, "device:2")).toMatchObject({
      status: "ok",
      statusText: "Online · doesn't answer ping",
    });
    expect(node(m, "device:3").status).toBe("offline");
    expect(node(m, "device:4").status).toBe("problem");
  });

  it("marks devices named by active alerts", () => {
    const m = buildMap(
      input({
        devices: [
          device({ id: 2, ip: "192.168.1.11" }),
          device({ id: 3, ip: "192.168.1.12" }),
          device({ id: 4, ip: "192.168.1.13" }),
        ],
        alerts: [
          alert({ id: 1, key: "starred_offline:2", title: "Phone is offline" }),
          alert({
            id: 2,
            key: "packet_loss:device:3",
            title: "TV is losing packets",
          }),
          alert({ id: 3, key: "new_device:4", title: "New device joined" }),
        ],
      }),
    );
    expect(node(m, "device:2")).toMatchObject({
      status: "problem",
      statusText: "Phone is offline",
    });
    expect(node(m, "device:3")).toMatchObject({
      status: "problem",
      statusText: "TV is losing packets",
    });
    expect(node(m, "device:4")).toMatchObject({
      status: "warn",
      statusText: "New device joined",
    });
    for (const id of ["device:2", "device:3", "device:4"])
      expect(isProblem(node(m, id))).toBe(true);
  });

  it("marks the router and the internet from their alerts", () => {
    const base = { devices: [gw()] };
    expect(
      node(buildMap(input({ ...base, alerts: [alert({ key: "router_down" })] })), "router").status,
    ).toBe("problem");
    expect(
      node(buildMap(input({ ...base, alerts: [alert({ key: "latency_high" })] })), "internet")
        .status,
    ).toBe("warn");
    expect(
      node(
        buildMap(
          input({
            ...base,
            alerts: [alert({ id: 1, key: "latency_high" }), alert({ id: 2, key: "internet_down" })],
          }),
        ),
        "internet",
      ).status,
    ).toBe("problem");
  });

  it("ignores alerts about unknown devices and unknown keys", () => {
    const devices = [device({ id: 1, ip: "192.168.1.10" })];
    const before = buildMap(input({ devices }));
    const after = buildMap(
      input({
        devices,
        alerts: [
          alert({ id: 1, key: "starred_offline:999" }),
          alert({ id: 2, key: "packet_loss:dns" }),
          alert({ id: 3, key: "new_device:x" }),
        ],
      }),
    );
    expect(after).toEqual(before);
    expect(after.nodes).toHaveLength(3);
  });

  it("draws a router node when no device is the gateway", () => {
    const withIp = node(buildMap(input({ gatewayIp: "192.168.1.1" })), "router");
    expect(withIp).toMatchObject({
      deviceId: null,
      label: "Router · 192.168.1.1",
      status: "unknown",
      statusText: "Not found by the last scan",
    });
    expect(node(buildMap(input()), "router").label).toBe("Router");
    const empty = buildMap(input({ devices: [] }));
    expect(empty.nodes.map((n) => n.id).sort()).toEqual(["internet", "router"]);
    expect(empty.counts.all).toBe(0);
  });

  it("reads the internet from the monitor", () => {
    const net = (monitor: MapInput["monitor"]) => node(buildMap(input({ monitor })), "internet");
    expect(net(null)).toMatchObject({
      status: "unknown",
      statusText: "Not monitoring",
    });
    expect(net(monitorStatus({ state: "waiting_for_scan" })).status).toBe("unknown");
    expect(net(monitorStatus({ last: tick({ internet_rtt_ms: 18 }) }))).toMatchObject({
      status: "ok",
      statusText: "Online · 18 ms",
    });
    expect(net(monitorStatus({ last: tick({ internet_rtt_ms: null }) }))).toMatchObject({
      status: "warn",
      statusText: "No reply",
    });
  });

  it("filters devices but always keeps router and internet", () => {
    const devices = [
      gw(),
      device({ id: 1, ip: "192.168.1.10" }),
      device({ id: 2, ip: "192.168.1.11" }),
      device({ id: 3, ip: "192.168.1.12", online: false, starred: true }),
    ];
    const deviceNodes = (filter: MapInput["filter"]) => {
      const m = buildMap(input({ devices, filter }));
      expect(m.nodes.some((n) => n.id === "router")).toBe(true);
      expect(m.nodes.some((n) => n.id === "internet")).toBe(true);
      expect(m.counts).toEqual({ all: 3, online: 2, problems: 1 });
      return m.nodes.filter((n) => n.kind === "device").length;
    };
    expect(deviceNodes("all")).toBe(3);
    expect(deviceNodes("online")).toBe(2);
    expect(deviceNodes("problems")).toBe(1);
  });

  it("keeps positions when only statuses change", () => {
    const make = (online: boolean) =>
      buildMap(
        input({
          devices: [
            device({ id: 1, ip: "192.168.1.30", online }),
            device({ id: 2, ip: "192.168.1.5", online: !online }),
            device({ id: 3, ip: null as unknown as string, online }),
          ],
        }),
      );
    const a = make(true);
    const b = make(false);
    expect(a.nodes.map((n) => [n.id, n.x, n.y])).toEqual(b.nodes.map((n) => [n.id, n.x, n.y]));
  });

  it("labels this machine by its display name, flags it, and links edges from the router", () => {
    const m = buildMap(
      input({
        devices: [
          device({
            id: 7,
            ip: "192.168.1.7",
            is_self: true,
            online: false,
            display_name: "DESKTOP-1",
          }),
          device({ id: 8, ip: "192.168.1.8" }),
        ],
      }),
    );
    expect(node(m, "device:7")).toMatchObject({ label: "DESKTOP-1", isSelf: true });
    expect(node(m, "device:8").isSelf).toBe(false);
    expect(node(m, "internet").isSelf).toBe(false);
    expect(m.edges).toContainEqual({
      id: "e:internet",
      source: "internet",
      target: "router",
      status: node(m, "router").status,
    });
    expect(m.edges).toContainEqual({
      id: "e:device:7",
      source: "router",
      target: "device:7",
      status: "offline",
    });
  });
});

describe("ringPositions", () => {
  it("keeps 250 devices apart", () => {
    const pts = ringPositions(250);
    expect(pts).toHaveLength(250);
    for (let i = 0; i < pts.length; i++) {
      const { x, y } = pts[i];
      expect(Math.abs(Math.atan2(x, -y))).toBeGreaterThanOrEqual(((24 - 0.5) * Math.PI) / 180);
      for (let j = i + 1; j < pts.length; j++) {
        expect(Math.hypot(x - pts[j].x, y - pts[j].y)).toBeGreaterThanOrEqual(72 * 0.9);
      }
    }
  });

  it("fills the first ring first", () => {
    const radii = (n: number) => ringPositions(n).map((p) => Math.round(Math.hypot(p.x, p.y)));
    expect(radii(9).every((r) => r === 130)).toBe(true);
    const r10 = radii(10);
    expect(r10.filter((r) => r === 240)).toHaveLength(1);
    expect(r10.filter((r) => r === 130)).toHaveLength(9);
  });
});
