import type { Alert } from "../api/alerts";
import type { MonitorStatus } from "../api/monitor";
import type { Device } from "../api/queries";
import { statusText } from "./deviceStatus";
import { compareIp } from "./ip";

export type NodeStatus = "ok" | "warn" | "problem" | "offline" | "unknown";
export type MapFilter = "all" | "online" | "problems";

export interface MapNode {
  id: string;
  kind: "internet" | "router" | "device";
  deviceId: number | null;
  label: string;
  status: NodeStatus;
  statusText: string;
  isNew: boolean;
  starred: boolean;
  ip: string | null;
  x: number;
  y: number;
}

export interface MapEdge {
  id: string;
  source: string;
  target: string;
  status: NodeStatus;
}

export interface NetworkMap {
  nodes: MapNode[];
  edges: MapEdge[];
  counts: Record<MapFilter, number>;
}

export interface MapInput {
  devices: Device[];
  alerts: Alert[];
  monitor: MonitorStatus | null;
  gatewayIp: string | null;
  filter: MapFilter;
  now: number;
}

export const LAYOUT = {
  FIRST_RING: 130,
  RING_STEP: 110,
  SPACING: 72,
  GAP_DEG: 24,
  INTERNET_CLEARANCE: 120,
} as const;

/** Points on concentric rings around (0,0), leaving a clear lane straight up. */
export function ringPositions(count: number): { x: number; y: number }[] {
  const out: { x: number; y: number }[] = [];
  const start = ((-90 + LAYOUT.GAP_DEG) * Math.PI) / 180;
  const span = ((360 - 2 * LAYOUT.GAP_DEG) * Math.PI) / 180;
  let left = count;
  for (let k = 0; left > 0; k++) {
    const r = LAYOUT.FIRST_RING + LAYOUT.RING_STEP * k;
    const capacity = Math.max(1, Math.floor((r * span) / LAYOUT.SPACING));
    const n = Math.min(left, capacity);
    for (let i = 0; i < n; i++) {
      const a = start + ((i + 0.5) * span) / n;
      out.push({
        x: Math.round(r * Math.cos(a)),
        y: Math.round(r * Math.sin(a)),
      });
    }
    left -= n;
  }
  return out;
}

export function isProblem(n: MapNode): boolean {
  return n.status === "problem" || n.status === "warn";
}

interface Mark {
  status: "problem" | "warn";
  text: string;
}

/** Keeps the worse mark; on a tie the earlier one stays. */
function mark(marks: Map<string, Mark>, who: string, status: "problem" | "warn", text: string) {
  const prev = marks.get(who);
  if (!prev || (prev.status === "warn" && status === "problem")) marks.set(who, { status, text });
}

function readAlerts(
  alerts: Alert[],
  known: Set<number>,
  gatewayId: number | null,
): Map<string, Mark> {
  const marks = new Map<string, Mark>();
  // The gateway device is drawn as the router node, so its alerts belong to "router".
  const who = (id: string) => (Number(id) === gatewayId ? "router" : `device:${id}`);
  for (const a of alerts) {
    const key = a.key;
    let m: RegExpMatchArray | null;
    if (
      (m = key.match(/^starred_offline:(\d+)$/)) ||
      (m = key.match(/^packet_loss:device:(\d+)$/))
    ) {
      if (known.has(Number(m[1]))) mark(marks, who(m[1]), "problem", a.title);
    } else if ((m = key.match(/^new_device:(\d+)$/))) {
      if (known.has(Number(m[1]))) mark(marks, who(m[1]), "warn", a.title);
    } else if (key === "router_down" || key === "packet_loss:gateway") {
      mark(marks, "router", "problem", a.title);
    } else if (key === "internet_down") {
      mark(marks, "internet", "problem", a.title);
    } else if (key === "latency_high" || key === "packet_loss:internet") {
      mark(marks, "internet", "warn", a.title);
    }
  }
  return marks;
}

function deviceStatus(d: Device, now: number): { status: NodeStatus; text: string } {
  const text = statusText(d, now);
  if (!d.online) return { status: d.starred ? "problem" : "offline", text };
  return { status: "ok", text };
}

function deviceNode(
  d: Device,
  id: string,
  kind: "router" | "device",
  now: number,
  marks: Map<string, Mark>,
): MapNode {
  const base = deviceStatus(d, now);
  const m = marks.get(id);
  return {
    id,
    kind,
    deviceId: d.id,
    label: d.is_self ? "This laptop" : d.display_name,
    status: m ? m.status : base.status,
    statusText: m ? m.text : base.text,
    isNew: d.is_new,
    starred: d.starred,
    ip: d.ip,
    x: 0,
    y: 0,
  };
}

function compareDevices(a: Device, b: Device): number {
  if (a.ip && b.ip) return compareIp(a.ip, b.ip) || a.id - b.id;
  if (a.ip) return -1;
  if (b.ip) return 1;
  return a.id - b.id;
}

export function buildMap(input: MapInput): NetworkMap {
  const { devices, alerts, monitor, gatewayIp, filter, now } = input;
  const gateway = devices.find((d) => d.is_gateway) ?? null;
  const marks = readAlerts(alerts, new Set(devices.map((d) => d.id)), gateway?.id ?? null);
  const router: MapNode = gateway
    ? deviceNode(gateway, "router", "router", now, marks)
    : {
        id: "router",
        kind: "router",
        deviceId: null,
        label: gatewayIp ? `Router · ${gatewayIp}` : "Router",
        status: marks.get("router")?.status ?? "unknown",
        statusText: marks.get("router")?.text ?? "Not found by the last scan",
        isNew: false,
        starred: false,
        ip: gatewayIp,
        x: 0,
        y: 0,
      };

  const all = devices
    .filter((d) => d !== gateway)
    .sort(compareDevices)
    .map((d) => deviceNode(d, `device:${d.id}`, "device", now, marks));

  const counts: Record<MapFilter, number> = {
    all: all.length,
    online: 0,
    problems: all.filter(isProblem).length,
  };
  const onlineIds = new Set(devices.filter((d) => d.online).map((d) => `device:${d.id}`));
  counts.online = all.filter((n) => onlineIds.has(n.id)).length;

  const kept =
    filter === "online"
      ? all.filter((n) => onlineIds.has(n.id))
      : filter === "problems"
        ? all.filter(isProblem)
        : all;
  const spots = ringPositions(kept.length);
  kept.forEach((n, i) => {
    n.x = spots[i].x;
    n.y = spots[i].y;
  });

  const rings = spots.length
    ? Math.max(...spots.map((p) => Math.round(Math.hypot(p.x, p.y))))
    : LAYOUT.FIRST_RING;
  const internet = internetNode(
    monitor,
    marks.get("internet"),
    -(rings + LAYOUT.INTERNET_CLEARANCE),
  );

  const edges: MapEdge[] = [
    {
      id: "e:internet",
      source: "internet",
      target: "router",
      status: router.status,
    },
    ...kept.map((n) => ({
      id: `e:${n.id}`,
      source: "router",
      target: n.id,
      status: n.status,
    })),
  ];
  return { nodes: [internet, router, ...kept], edges, counts };
}

function internetNode(monitor: MonitorStatus | null, m: Mark | undefined, y: number): MapNode {
  let status: NodeStatus;
  let text: string;
  if (!monitor || monitor.state !== "monitoring") {
    status = "unknown";
    text = "Not monitoring";
  } else if (m) {
    status = m.status;
    text = m.text;
  } else if (monitor.last?.internet_rtt_ms == null) {
    status = "warn";
    text = "No reply";
  } else {
    status = "ok";
    text = `Online · ${monitor.last.internet_rtt_ms} ms`;
  }
  return {
    id: "internet",
    kind: "internet",
    deviceId: null,
    label: "Internet",
    status,
    statusText: text,
    isNew: false,
    starred: false,
    ip: null,
    x: 0,
    y,
  };
}
