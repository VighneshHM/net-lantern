import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useAlerts } from "../api/alerts";
import { useMonitor } from "../api/monitor";
import { useDevice, useDevices, useNetwork } from "../api/queries";
import { DeviceDetails } from "../components/DeviceDetails";
import { MapCanvas } from "../components/MapCanvas";
import { OutdatedBanner } from "../components/OutdatedBanner";
import {
  buildMap,
  isProblem,
  type MapFilter,
  type MapNode,
  type NodeStatus,
} from "../lib/networkMap";

const FILTERS: { id: MapFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "online", label: "Online" },
  { id: "problems", label: "Problems" },
];

const DOT: Record<NodeStatus, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  problem: "bg-bad",
  offline: "bg-text-muted",
  unknown: "bg-border-strong",
};

/**
 * Wall clock for the "x ago" texts. It is read when the map is rebuilt, so the texts
 * refresh whenever the map's inputs change (each monitor tick while monitoring, otherwise
 * only on a refetch or filter change), not continuously.
 */
function clock(): number {
  return Date.now();
}

const badge =
  "rounded-full border border-border-strong bg-surface-2 px-2 py-0.5 text-xs font-medium text-text";

const PHONE_LAYOUT = "(max-width: 759px)";

/**
 * The device details panel. On the stacked phone layout it sits above the map and out of
 * view, so it is scrolled to. That waits until the device has loaded: the panel grows when
 * its content arrives, and a scroll made while it is still tiny gets undone by the browser
 * keeping the tapped list row in place. Beside the map (wider screens) the page stays put.
 */
function DetailsPanel({ id, onClose }: { id: number; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const loaded = !useDevice(id).isPending; // same query as DeviceDetails, so one request
  useEffect(() => {
    if (!loaded) return;
    if (!window.matchMedia?.(PHONE_LAYOUT)?.matches) return;
    ref.current?.scrollIntoView?.({ block: "start" });
  }, [id, loaded]);
  return (
    <div ref={ref} className="order-1 min-[760px]:order-2">
      <DeviceDetails id={id} onClose={onClose} />
    </div>
  );
}

function NodeRow({ node, onOpen }: { node: MapNode; onOpen: (n: MapNode) => void }) {
  const label =
    node.deviceId !== null ? (
      <button
        type="button"
        onClick={() => onOpen(node)}
        className="min-h-11 cursor-pointer text-left font-medium"
      >
        {node.label}
      </button>
    ) : (
      <span className="font-medium">{node.label}</span>
    );
  return (
    <li className="flex min-h-11 flex-wrap items-center gap-x-3 gap-y-1 border-t border-border px-3 py-1 first:border-t-0">
      <span aria-hidden="true" className={`size-2.5 shrink-0 rounded-full ${DOT[node.status]}`} />
      {label}
      {node.isNew && <span className={badge}>New</span>}
      {node.isSelf && <span className={badge}>This device</span>}
      {node.kind === "router" && node.deviceId !== null && <span className={badge}>Router</span>}
      {node.ip !== null && !node.label.includes(node.ip) && (
        <span className="font-mono text-xs text-text-muted">{node.ip}</span>
      )}
      <span className="text-sm text-text-muted">{node.statusText}</span>
      {node.kind === "internet" && (
        <Link to="/diagnose" className="ml-auto text-sm font-medium underline">
          Diagnose
        </Link>
      )}
    </li>
  );
}

export function NetworkMap() {
  const devices = useDevices();
  const alerts = useAlerts("active");
  const monitor = useMonitor();
  const network = useNetwork();
  const [filter, setFilter] = useState<MapFilter>("all");
  const [params, setParams] = useSearchParams();
  const rawId = params.get("device");
  const selected = rawId !== null && /^\d+$/.test(rawId) ? Number(rawId) : null;

  function select(id: number | null, replace = false) {
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        if (id === null) next.delete("device");
        else next.set("device", String(id));
        return next;
      },
      { replace },
    );
  }

  const deviceData = devices.data;
  const alertData = alerts.data;
  const monitorData = monitor.data;
  const gatewayIp = network.data?.gateway ?? null;
  const map = useMemo(
    () =>
      deviceData
        ? buildMap({
            devices: deviceData,
            alerts: alertData ?? [],
            monitor: monitorData ?? null,
            gatewayIp,
            filter,
            now: clock(),
          })
        : null,
    [deviceData, alertData, monitorData, gatewayIp, filter],
  );

  // A device the new filter takes off the map must not stay open in the panel. The router
  // and internet are always kept, and the gateway device is the router node.
  function changeFilter(next: MapFilter) {
    setFilter(next);
    if (selected === null || !deviceData) return;
    const kept = buildMap({
      devices: deviceData,
      alerts: alertData ?? [],
      monitor: monitorData ?? null,
      gatewayIp,
      filter: next,
      now: clock(),
    });
    const known = deviceData.some((d) => d.id === selected);
    if (known && !kept.nodes.some((x) => x.deviceId === selected)) select(null, true);
  }

  let body;
  if (devices.isPending) {
    body = <p className="m-0 text-text-muted">Loading devices…</p>;
  } else if (devices.error) {
    body = <p className="m-0 text-text-muted">Couldn't load devices: {devices.error.message}</p>;
  } else if (!map || deviceData!.length === 0) {
    body = (
      <p className="m-0 text-text-muted">No devices yet — run a Quick scan from the Dashboard.</p>
    );
  } else {
    const gatewayId = deviceData!.find((d) => d.is_gateway)?.id ?? null;
    const selectedNodeId =
      selected === null ? null : selected === gatewayId ? "router" : `device:${selected}`;
    const deviceNodes = map.nodes.filter((n) => n.kind === "device");
    const n = deviceNodes.length;
    const internet = map.nodes.find((x) => x.kind === "internet")!;
    const router = map.nodes.find((x) => x.kind === "router")!;
    const hubProblems = (isProblem(router) ? 1 : 0) + (isProblem(internet) ? 1 : 0);
    const problems = map.counts.problems + hubProblems;
    const label = `Network map: the router, the internet and ${n} ${n === 1 ? "device" : "devices"}; ${problems} with a problem`;
    const listed = [
      ...deviceNodes.filter(isProblem),
      ...deviceNodes.filter((x) => !isProblem(x)),
    ];
    const open = (node: MapNode) => {
      if (node.deviceId !== null) select(node.deviceId);
    };
    body = (
      <>
        <div className="flex flex-wrap gap-2">
          {FILTERS.map((f) => (
            <button
              key={f.id}
              type="button"
              aria-pressed={filter === f.id}
              onClick={() => changeFilter(f.id)}
              className={`min-h-11 cursor-pointer rounded-lg border px-4 text-sm font-semibold ${
                filter === f.id
                  ? "border-accent bg-accent text-accent-ink"
                  : "border-border-strong bg-surface-2 text-text"
              }`}
            >
              {f.label} <span>{f.id === "problems" ? problems : map.counts[f.id]}</span>
            </button>
          ))}
        </div>
        <div className="flex flex-col gap-4 min-[760px]:flex-row min-[760px]:items-start">
          <div className="order-2 flex min-w-0 flex-1 flex-col gap-4 min-[760px]:order-1">
            <MapCanvas map={map} selectedId={selectedNodeId} onSelect={open} label={label} />
            <section aria-label="Map devices" className="flex flex-col gap-2">
              <h2 className="m-0 text-xl font-bold">On the map</h2>
              <ul className="m-0 list-none overflow-hidden rounded-xl border border-border bg-surface p-0">
                <NodeRow node={internet} onOpen={open} />
                <NodeRow node={router} onOpen={open} />
                {listed.map((x) => (
                  <NodeRow key={x.id} node={x} onOpen={open} />
                ))}
              </ul>
              {filter === "problems" && problems === 0 && (
                <p className="m-0 text-text-muted">No problems right now.</p>
              )}
            </section>
          </div>
          {selected !== null && (
            <DetailsPanel id={selected} onClose={() => select(null)} />
          )}
        </div>
      </>
    );
  }

  return (
    <>
      <h1 className="m-0 text-3xl font-bold">Network map</h1>
      <OutdatedBanner />
      <p className="m-0 text-text-muted">
        Every device is drawn straight to the router. Switches and mesh points between devices
        need access to the router, which is planned for later.
      </p>
      {body}
    </>
  );
}

export default NetworkMap;
