import type { MetricPoint, SessionSpan } from "../api/monitor";

const NUDGE_S = 0.001;

function sessionIndex(ts: number, sessions: SessionSpan[]): number {
  return sessions.findIndex((s) => ts >= s.started_at && (s.ended_at === null || ts <= s.ended_at));
}

/**
 * Chart data for uPlot: times in seconds, with a `null` value inserted where
 * Net Lantern stopped monitoring (the end of a session, or the start of a gap
 * between points of different sessions) so the line breaks there.
 */
export function withGaps(
  points: MetricPoint[],
  sessions: SessionSpan[],
): { ts: number[]; avg: (number | null)[] } {
  const ts: number[] = [];
  const avg: (number | null)[] = [];
  let prev: { ts: number; session: number } | null = null;
  for (const p of points) {
    const session = sessionIndex(p.ts, sessions);
    if (prev !== null && sessions.length > 0 && (session !== prev.session || session < 0)) {
      const ended = prev.session >= 0 ? sessions[prev.session].ended_at : null;
      let at = Math.max(ended ?? prev.ts, prev.ts) / 1000;
      if (at <= ts[ts.length - 1]) at = ts[ts.length - 1] + NUDGE_S;
      if (at < p.ts / 1000) {
        ts.push(at);
        avg.push(null);
      }
    }
    ts.push(p.ts / 1000);
    avg.push(p.avg);
    prev = { ts: p.ts, session };
  }
  return { ts, avg };
}
