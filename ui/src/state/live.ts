import { create } from "zustand";
import { LAYERS, type DiagnosisRun, type LayerResult, type Verdict } from "../api/diagnose";
import type { ServerEvent } from "../api/events";

export interface LiveScan {
  id: number;
  state: string;
  stage: string;
  done: number;
  total: number;
  devicesFound: number;
  error: string | null;
  isolated: boolean;
}

interface LiveState {
  /** The scan being followed (or the last one that finished); null before any scan. */
  scan: LiveScan | null;
  /** The diagnosis run being followed (or the last one that finished); null before any. */
  diagnosis: DiagnosisRun | null;
  /** True when the server speaks a different `api_version` than this build. */
  outdated: boolean;
  applyEvent(ev: ServerEvent): void;
  /** Start following `scanId` (a no-op if it is already the followed scan). */
  attach(scanId: number): void;
  /**
   * Replace the followed scan with a snapshot loaded over REST. Ignored if it
   * is of a different scan than the one followed (unless none is followed).
   */
  setScan(scan: LiveScan): void;
  /** Replace the followed diagnosis run. */
  followDiagnosis(run: DiagnosisRun): void;
  /**
   * Fold a REST snapshot into the followed run. A snapshot of another run
   * replaces it; of the same run it never undoes a finish or drops steps.
   */
  mergeDiagnosis(snapshot: DiagnosisRun): void;
  markOutdated(): void;
}

/** Put `step` into `steps` by layer, keeping layer order. */
function withStep(steps: LayerResult[], step: LayerResult): LayerResult[] {
  const i = steps.findIndex((s) => s.layer === step.layer);
  if (i >= 0) return steps.map((s, j) => (j === i ? step : s));
  const at = LAYERS.indexOf(step.layer);
  const next = steps.findIndex((s) => LAYERS.indexOf(s.layer) > at);
  if (next < 0) return [...steps, step];
  return [...steps.slice(0, next), step, ...steps.slice(next)];
}

function running(id: number): LiveScan {
  return { id, state: "running", stage: "sweep", done: 0, total: 0, devicesFound: 0, error: null, isolated: false };
}

const num = (v: unknown, fallback: number) => (typeof v === "number" ? v : fallback);

/**
 * Live scan state, fed by WebSocket events and REST reloads. A scan whose
 * events arrive is followed even if this tab did not start it.
 */
export const useLive = create<LiveState>((set, get) => ({
  scan: null,
  diagnosis: null,
  outdated: false,

  applyEvent(ev) {
    const data = (ev.data ?? {}) as Record<string, unknown>;
    if (ev.type === "diagnose.step" || ev.type === "diagnose.done") {
      const run = get().diagnosis;
      if (!run || data.run_id !== run.id) return;
      if (ev.type === "diagnose.step") {
        const step = data.step as LayerResult | undefined;
        if (!step || typeof step.layer !== "string") return;
        set({ diagnosis: { ...run, steps: withStep(run.steps, step) } });
      } else {
        set({
          diagnosis: {
            ...run,
            finished_at: run.finished_at ?? Date.now(),
            state: data.state === "failed" ? "failed" : "done",
            verdict: (data.verdict as Verdict | null | undefined) ?? null,
            error: typeof data.error === "string" ? data.error : null,
          },
        });
      }
      return;
    }
    const id = data.scan_id;
    if (typeof id !== "number") return;
    const current = get().scan;
    const base = current?.id === id ? current : running(id);

    switch (ev.type) {
      case "scan.progress":
        // A late progress event must not bring a finished scan back to life.
        if (base.state !== "running") return;
        set({
          scan: {
            ...base,
            stage: typeof data.stage === "string" ? data.stage : base.stage,
            done: num(data.done, base.done),
            total: num(data.total, base.total),
          },
        });
        return;
      case "scan.device":
        if (base.state !== "running") return;
        set({ scan: { ...base, devicesFound: base.devicesFound + 1 } });
        return;
      case "scan.done":
        set({
          scan: {
            ...base,
            state: typeof data.state === "string" ? data.state : "done",
            devicesFound: num(data.devices_found, base.devicesFound),
            error: typeof data.error === "string" ? data.error : null,
            isolated: data.isolated === true,
          },
        });
        return;
    }
  },

  attach(scanId) {
    if (get().scan?.id === scanId) return;
    set({ scan: running(scanId) });
  },

  setScan(scan) {
    const current = get().scan;
    // A late reload of another scan (say, the previous one, after a resync)
    // must not replace the scan being followed.
    if (current !== null && current.id !== scan.id) return;
    // A snapshot taken before a finish event arrived must not undo the finish.
    if (current?.id === scan.id && current.state !== "running" && scan.state === "running") return;
    set({ scan });
  },

  followDiagnosis(run) {
    set({ diagnosis: run });
  },

  mergeDiagnosis(snapshot) {
    const cur = get().diagnosis;
    if (!cur || cur.id !== snapshot.id) {
      set({ diagnosis: snapshot });
      return;
    }
    if (cur.state !== "running") {
      if (snapshot.state === "running") return;
      return set({ diagnosis: snapshot });
    }
    if (snapshot.state === "running") {
      let steps = snapshot.steps;
      for (const s of cur.steps) steps = withStep(steps, s);
      set({ diagnosis: { ...snapshot, steps } });
      return;
    }
    set({ diagnosis: snapshot });
  },

  markOutdated() {
    set({ outdated: true });
  },
}));
