import cytoscape from "cytoscape";
import { useEffect, useRef, useState } from "react";
import type { MapNode, NetworkMap, NodeStatus } from "../lib/networkMap";

export interface MapCanvasProps {
  map: NetworkMap;
  selectedId: string | null; // a MapNode id
  onSelect: (node: MapNode) => void; // fired only for nodes whose deviceId is not null
  label: string; // accessible summary, put on the canvas wrapper
}

export const COMPACT_BELOW = 480; // px of container width
const FIT_PADDING = 16; // px kept clear around the drawing when it is fitted
const LABEL_SIZE = 13; // px at zoom 1; the fitted zoom is usually below 1

/** Fallbacks for jsdom, where custom properties resolve to nothing. Mirrors styles/tokens.css. */
const FALLBACKS: Record<string, string> = {
  "--bg": "#0A0F14",
  "--text": "#E6EDF3",
  "--text-muted": "#93A4B3",
  "--border-strong": "#2C3E50",
  "--accent": "#46C2E8",
  "--ok": "#56D364",
  "--warn": "#E3B341",
  "--bad": "#FF7B72",
};

function token(name: string): string {
  const v = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim();
  return v === "" ? FALLBACKS[name] : v;
}

const STATUS_TOKEN: Record<NodeStatus, string> = {
  ok: "--ok",
  warn: "--warn",
  problem: "--bad",
  offline: "--text-muted",
  unknown: "--border-strong",
};

function buildStyle(compact: boolean): cytoscape.StylesheetJson {
  const device = compact ? 16 : 28;
  const hub = compact ? 26 : 44;
  const style: cytoscape.StylesheetJson = [
    {
      selector: "node",
      style: {
        width: device,
        height: device,
        "background-color": token("--border-strong"),
        "border-width": 0,
        label: compact ? "" : "data(label)",
        color: token("--text"),
        "font-size": LABEL_SIZE,
        "text-valign": "bottom",
        "text-margin-y": 4,
        "text-wrap": "ellipsis",
        "text-max-width": "120px",
        "text-background-color": token("--bg"),
        "text-background-opacity": 0.7,
      },
    },
    {
      selector: "node.device",
      style: {
        width: device,
        height: device,
        label: compact ? "" : "data(label)",
      },
    },
    { selector: "node.router", style: { width: hub, height: hub } },
    { selector: "node.internet", style: { width: hub, height: hub } },
  ];
  for (const [status, name] of Object.entries(STATUS_TOKEN)) {
    style.push({
      selector: `node[status = "${status}"]`,
      style: { "background-color": token(name) },
    });
  }
  style.push(
    {
      selector: "node[?isNew]",
      style: { "border-width": 3, "border-color": token("--accent") },
    },
    {
      selector: "node[?selected]",
      style: {
        "border-width": 3,
        "border-color": token("--text"),
        label: "data(label)",
      },
    },
    {
      selector: "edge",
      style: {
        width: 1.5,
        "line-color": token("--border-strong"),
        "curve-style": "straight",
      },
    },
    { selector: 'edge[status = "offline"]', style: { "line-style": "dashed" } },
    {
      selector: 'edge[status = "problem"]',
      style: { "line-color": token("--bad") },
    },
  );
  return style;
}

function toElements(
  map: NetworkMap,
  selectedId: string | null,
): cytoscape.ElementDefinition[] {
  return [
    ...map.nodes.map((n) => ({
      group: "nodes" as const,
      data: {
        id: n.id,
        label: n.label,
        status: n.status,
        isNew: n.isNew,
        selected: n.id === selectedId,
      },
      position: { x: n.x, y: n.y },
      classes: n.kind,
    })),
    ...map.edges.map((e) => ({
      group: "edges" as const,
      data: { id: e.id, source: e.source, target: e.target, status: e.status },
    })),
  ];
}

const ZOOM_BUTTON =
  "min-h-11 min-w-11 rounded-lg border border-border bg-surface-2 text-lg font-semibold text-text";

export function MapCanvas({
  map,
  selectedId,
  onSelect,
  label,
}: MapCanvasProps) {
  const canvas = useRef<HTMLDivElement>(null);
  const cy = useRef<cytoscape.Core | null>(null);
  const [compact, setCompact] = useState(false);

  const latest = useRef({ map, selectedId, compact, onSelect });
  const ids = useRef<string[]>([]);
  const allIds = useRef<string[]>([]);
  const appliedCompact = useRef(false);

  useEffect(() => {
    latest.current = { map, selectedId, compact, onSelect };
  });

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const cur = latest.current;
    const instance = cytoscape({
      container: el,
      elements: toElements(cur.map, cur.selectedId),
      style: buildStyle(cur.compact),
      layout: { name: "preset" },
      minZoom: 0.3,
      maxZoom: 3,
      userZoomingEnabled: false,
      userPanningEnabled: !cur.compact,
      boxSelectionEnabled: false,
      autoungrabify: true,
    });
    cy.current = instance;
    ids.current = cur.map.nodes.map((n) => n.id);
    allIds.current = [...ids.current, ...cur.map.edges.map((e) => e.id)];
    appliedCompact.current = cur.compact;
    instance.on("tap", "node", (e) => {
      const node = latest.current.map.nodes.find((n) => n.id === e.target.id());
      if (node && node.deviceId !== null) latest.current.onSelect(node);
    });
    instance.fit(undefined, FIT_PADDING);

    const observer = new ResizeObserver((entries) => {
      const width = entries[0]?.contentRect.width ?? el.clientWidth;
      setCompact(width < COMPACT_BELOW);
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
      instance.destroy();
      cy.current = null;
    };
  }, []);

  useEffect(() => {
    const instance = cy.current;
    if (!instance) return;
    const next = map.nodes.map((n) => n.id);
    const keep = new Set([...next, ...map.edges.map((e) => e.id)]);
    for (const id of allIds.current)
      if (!keep.has(id)) instance.getElementById(id).remove();
    allIds.current = [...keep];
    instance.json({ elements: toElements(map, selectedId) });
    const changed =
      next.length !== ids.current.length ||
      next.some((id) => !ids.current.includes(id));
    ids.current = next;
    if (changed) instance.fit(undefined, FIT_PADDING);
  }, [map, selectedId]);

  useEffect(() => {
    const instance = cy.current;
    if (!instance || appliedCompact.current === compact) return;
    appliedCompact.current = compact;
    instance.style(buildStyle(compact));
    instance.userPanningEnabled(!compact);
    instance.resize();
    instance.fit(undefined, FIT_PADDING);
  }, [compact]);

  const zoomBy = (factor: number) => {
    const instance = cy.current;
    if (!instance) return;
    instance.zoom({
      level: instance.zoom() * factor,
      renderedPosition: { x: instance.width() / 2, y: instance.height() / 2 },
    });
  };

  return (
    <div className="relative h-[320px] w-full min-w-0 overflow-hidden rounded-xl border border-border bg-surface min-[760px]:h-[520px]">
      <div
        ref={canvas}
        role="img"
        aria-label={label}
        className="h-full w-full"
      />
      <div className="absolute right-2 top-2 flex flex-col gap-1">
        <button
          type="button"
          aria-label="Zoom in"
          className={ZOOM_BUTTON}
          onClick={() => zoomBy(1.25)}
        >
          +
        </button>
        <button
          type="button"
          aria-label="Zoom out"
          className={ZOOM_BUTTON}
          onClick={() => zoomBy(0.8)}
        >
          &minus;
        </button>
        <button
          type="button"
          aria-label="Fit map"
          className={ZOOM_BUTTON}
          onClick={() => cy.current?.fit(undefined, FIT_PADDING)}
        >
          &#x2922;
        </button>
      </div>
    </div>
  );
}
