import { useEffect, useRef, useState } from "react";
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import { useMetrics, type MetricSeries, type Range } from "../api/monitor";
import { withGaps } from "../lib/series";

const RANGES: { value: Range; phrase: string }[] = [
  { value: "1h", phrase: "Last hour" },
  { value: "24h", phrase: "Last 24 hours" },
  { value: "7d", phrase: "Last 7 days" },
];

const HEIGHT = 220;

type Gapped = ReturnType<typeof withGaps>;

/**
 * Join two gap-aware series on their timestamps. A series with no sample at a
 * shared time gets `undefined` there, which uPlot connects over; only an
 * explicit `null` breaks a line.
 */
function join(a: Gapped, b: Gapped): uPlot.AlignedData {
  const ts = [...new Set([...a.ts, ...b.ts])].sort((x, y) => x - y);
  const column = (s: Gapped) => {
    const m = new Map<number, number | null>();
    s.ts.forEach((t, i) => m.set(t, s.avg[i]));
    return ts.map((t) => (m.has(t) ? (m.get(t) as number | null) : undefined));
  };
  return [ts, column(a), column(b)] as unknown as uPlot.AlignedData;
}

function average(s: MetricSeries | undefined): number | null {
  const v = (s?.points ?? []).map((p) => p.avg).filter((x): x is number => x !== null);
  return v.length === 0 ? null : v.reduce((x, y) => x + y, 0) / v.length;
}

function token(name: string, fallback: string): string {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v === "" ? fallback : v;
}

/** Router and internet round-trip times as one chart, with a text summary. */
export function LatencyChart() {
  const [range, setRange] = useState<Range>("1h");
  const router = useMetrics("gateway", "rtt_ms", range);
  const internet = useMetrics("internet", "rtt_ms", range);
  const host = useRef<HTMLDivElement>(null);
  const routerData = router.data;
  const internetData = internet.data;

  useEffect(() => {
    const el = host.current;
    if (!el || !routerData || !internetData) return;
    const sessions = routerData.sessions.length > 0 ? routerData.sessions : internetData.sessions;
    const data = join(withGaps(routerData.points, sessions), withGaps(internetData.points, sessions));
    const muted = token("--text-muted", "#93A4B3");
    const grid = token("--border", "#223040");
    const axis = { stroke: muted, grid: { stroke: grid }, ticks: { stroke: grid } };
    const width = () => Math.max(el.clientWidth, 200);
    const chart = new uPlot(
      {
        width: width(),
        height: HEIGHT,
        cursor: { drag: { x: false, y: false } },
        scales: { x: { time: true } },
        axes: [axis, { ...axis, size: 60, values: (_u, v) => v.map((n) => `${n} ms`) }],
        series: [
          {},
          { label: "Router", stroke: token("--accent", "#46C2E8"), width: 2 },
          { label: "Internet", stroke: token("--warn", "#E3B341"), width: 2 },
        ],
      },
      data,
      el,
    );
    const observer = new ResizeObserver(() => chart.setSize({ width: width(), height: HEIGHT }));
    observer.observe(el);
    return () => {
      observer.disconnect();
      chart.destroy();
    };
  }, [routerData, internetData]);

  const phrase = RANGES.find((r) => r.value === range)?.phrase ?? "";
  const r = average(routerData);
  const i = average(internetData);
  let summary = "";
  if (routerData && internetData) {
    summary =
      r === null && i === null
        ? "No samples yet."
        : `${phrase}: router average ${r === null ? "—" : Math.round(r)} ms, internet average ${
            i === null ? "—" : Math.round(i)
          } ms.`;
  }

  return (
    <section
      aria-label="Latency"
      className="flex flex-col gap-3 rounded-xl border border-border bg-surface p-5"
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h2 className="m-0 text-sm font-medium text-text-muted">Router and internet latency</h2>
        <div role="group" aria-label="Time range" className="flex gap-1">
          {RANGES.map((o) => (
            <button
              key={o.value}
              type="button"
              aria-pressed={range === o.value}
              onClick={() => setRange(o.value)}
              className={`min-h-11 min-w-11 rounded-lg border px-3 text-sm font-semibold ${
                range === o.value
                  ? "border-accent bg-surface-2 text-text"
                  : "border-border bg-surface text-text-muted"
              }`}
            >
              {o.value}
            </button>
          ))}
        </div>
      </div>
      <div ref={host} className="w-full min-w-0 overflow-hidden" />
      {summary !== "" && <p className="m-0 text-sm">{summary}</p>}
      <p className="m-0 text-xs text-text-muted">
        Breaks in the lines are times Net Lantern wasn&apos;t monitoring.
      </p>
    </section>
  );
}
