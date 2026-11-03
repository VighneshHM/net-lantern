import { create } from "zustand";
import type { ServerEvent } from "../api/events";

export interface LiveScan {
  id: number;
  state: string;
  stage: string;
  done: number;
  total: number;
  devicesFound: number;
}

interface LiveState {
  /** The scan being followed (or the last one that finished); null before any scan. */
  scan: LiveScan | null;
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
  markOutdated(): void;
}

function running(id: number): LiveScan {
  return { id, state: "running", stage: "sweep", done: 0, total: 0, devicesFound: 0 };
}

const num = (v: unknown, fallback: number) => (typeof v === "number" ? v : fallback);

/**
 * Live scan state, fed by WebSocket events and REST reloads. A scan whose
 * events arrive is followed even if this tab did not start it.
 */
export const useLive = create<LiveState>((set, get) => ({
  scan: null,
  outdated: false,

  applyEvent(ev) {
    const data = (ev.data ?? {}) as Record<string, unknown>;
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

  markOutdated() {
    set({ outdated: true });
  },
}));
