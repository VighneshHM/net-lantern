import { describe, expect, it } from "vitest";
import { withGaps } from "./series";

const pt = (s: number, avg: number | null) => ({
  ts: s * 1000,
  min: avg,
  avg,
  max: avg,
  count: 1,
  lost: avg === null ? 1 : 0,
});

describe("withGaps", () => {
  it("breaks the line between sessions", () => {
    const out = withGaps(
      [pt(0, 1), pt(30, 2), pt(60, 3), pt(600, 4)],
      [
        { started_at: 0, ended_at: 60_000, end_reason: "sleep" },
        { started_at: 590_000, ended_at: null, end_reason: null },
      ],
    );
    const i = out.ts.indexOf(600);
    expect(out.ts.slice(0, i)).toEqual(expect.arrayContaining([0, 30, 60]));
    expect(out.avg[i - 1]).toBeNull();
    expect(out.avg[i]).toBe(4);
    expect(out.ts.length).toBe(out.avg.length);
    for (let k = 1; k < out.ts.length; k++) expect(out.ts[k]).toBeGreaterThan(out.ts[k - 1]);
  });

  it("keeps one line inside a session and keeps lost points as nulls", () => {
    const out = withGaps(
      [pt(0, 1), pt(30, null), pt(60, 3)],
      [{ started_at: 0, ended_at: null, end_reason: null }],
    );
    expect(out).toEqual({ ts: [0, 30, 60], avg: [1, null, 3] });
  });

  it("handles no points", () => {
    expect(withGaps([], [])).toEqual({ ts: [], avg: [] });
  });
});
