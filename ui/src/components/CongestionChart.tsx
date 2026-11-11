import type { Congestion } from "../api/wifi";
import { BAND_LABEL } from "../lib/band";

type ChartBand = "2.4ghz" | "5ghz";
interface Bar {
  channel: number;
  networks: number;
}

const SLOT = 30;
const BAR_AREA = 90;
const TOP = 16;
const HEIGHT = TOP + BAR_AREA + 20;
/** Every chart is at least this many slots wide, so its height stays bounded. */
const MIN_SLOTS = 13;

/** The channels a band's chart shows: 1-13 always for 2.4 GHz, else those with networks, plus yours. */
function chartBars(congestion: Congestion, band: ChartBand): Bar[] {
  const counts = new Map<number, number>();
  if (band === "2.4ghz") for (let c = 1; c <= 13; c++) counts.set(c, 0);
  for (const l of congestion.channels) {
    if (l.band === band && (l.networks > 0 || band === "2.4ghz")) counts.set(l.channel, l.networks);
  }
  const cur = congestion.current;
  if (cur && cur.band === band && !counts.has(cur.channel)) counts.set(cur.channel, 0);
  return [...counts.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([channel, networks]) => ({ channel, networks }));
}

function describeBars(band: ChartBand, bars: Bar[]): string {
  const busy = bars
    .filter((b) => b.networks > 0)
    .map((b) => `channel ${b.channel} has ${b.networks} ${b.networks === 1 ? "network" : "networks"}`);
  return `${BAND_LABEL[band]} channels: ${busy.length > 0 ? busy.join(", ") : "no networks"}`;
}

/** Whether a band's chart is worth showing (2.4 GHz always is). */
export function showChart(congestion: Congestion, band: ChartBand): boolean {
  if (band === "2.4ghz") return true;
  return (
    congestion.current?.band === band ||
    congestion.channels.some((l) => l.band === band && l.networks > 0)
  );
}

export function CongestionChart({ band, congestion }: { band: ChartBand; congestion: Congestion }) {
  const bars = chartBars(congestion, band);
  const max = Math.max(1, ...bars.map((b) => b.networks));
  const current = congestion.current?.band === band ? congestion.current.channel : null;
  const width = Math.max(bars.length, MIN_SLOTS) * SLOT;
  return (
    <figure className="m-0 flex flex-col gap-1">
      <figcaption className="text-sm font-medium text-text-muted">
        {BAND_LABEL[band]} channels
      </figcaption>
      <svg
        role="img"
        aria-label={describeBars(band, bars)}
        viewBox={`0 0 ${width} ${HEIGHT}`}
        width="100%"
        className="max-w-xl"
      >
        {bars.map((b, i) => {
          const h = (b.networks / max) * BAR_AREA;
          const x = i * SLOT;
          const isCurrent = b.channel === current;
          return (
            <g key={b.channel}>
              <rect
                x={x + 5}
                y={TOP + BAR_AREA - h}
                width={SLOT - 10}
                height={h}
                rx={2}
                className={isCurrent ? "fill-accent" : "fill-text-muted"}
              />
              <text
                x={x + SLOT / 2}
                y={TOP + BAR_AREA - h - 3}
                textAnchor="middle"
                fontSize={12}
                className="fill-text"
              >
                {b.networks}
              </text>
              <text
                x={x + SLOT / 2}
                y={TOP + BAR_AREA + 15}
                textAnchor="middle"
                fontSize={12}
                className="fill-text-muted"
              >
                {b.channel}
              </text>
            </g>
          );
        })}
        <line
          x1={0}
          x2={width}
          y1={TOP + BAR_AREA}
          y2={TOP + BAR_AREA}
          strokeWidth={1}
          className="stroke-border-strong"
        />
      </svg>
      {current !== null && (
        <p className="m-0 text-sm font-medium text-accent">Your channel: {current} (highlighted bar)</p>
      )}
    </figure>
  );
}
