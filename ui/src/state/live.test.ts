import { beforeEach, describe, expect, it, vi } from "vitest";
import { mockFetch } from "../test/mockFetch";
import { resetLive } from "../test/render";
import type { DiagnosisRun, LayerName, LayerResult, LayerStatus, Verdict } from "../api/diagnose";
import { useLive, type LiveScan } from "./live";

function snapshot(id: number, state: string): LiveScan {
  return { id, state, stage: "merge", done: 254, total: 254, devicesFound: 13, error: null, isolated: false };
}

beforeEach(() => {
  resetLive();
});

describe("setScan", () => {
  it("loads a snapshot when no scan is followed yet", () => {
    useLive.getState().setScan(snapshot(4, "done"));
    expect(useLive.getState().scan).toEqual(snapshot(4, "done"));
  });

  it("updates the followed scan from its own snapshot", () => {
    useLive.getState().attach(2);
    useLive.getState().setScan({ ...snapshot(2, "running"), stage: "arp", devicesFound: 0 });
    expect(useLive.getState().scan).toMatchObject({ id: 2, state: "running", stage: "arp" });
  });

  it("ignores a late snapshot of another scan while a scan is followed", () => {
    useLive.getState().attach(2);
    // A resync reload of the previous scan lands after scan 2 started.
    useLive.getState().setScan(snapshot(1, "done"));
    expect(useLive.getState().scan).toMatchObject({ id: 2, state: "running" });
  });

  it("does not undo a finish with an older running snapshot of the same scan", () => {
    useLive.getState().attach(3);
    useLive.getState().applyEvent({
      seq: 9,
      api_version: 1,
      type: "scan.done",
      data: { scan_id: 3, state: "done", devices_found: 13 },
    });
    useLive.getState().setScan(snapshot(3, "running"));
    expect(useLive.getState().scan).toMatchObject({ id: 3, state: "done" });
  });
});

describe("scan.done", () => {
  it("records the error and isolation flag", () => {
    useLive.getState().attach(5);
    useLive.getState().applyEvent({
      seq: 3,
      api_version: 1,
      type: "scan.done",
      data: { scan_id: 5, state: "failed", devices_found: 0, error: "boom", isolated: true },
    });
    expect(useLive.getState().scan).toMatchObject({ state: "failed", error: "boom", isolated: true });
  });

  it("setScan carries error and isolated", () => {
    useLive.getState().setScan({ ...snapshot(6, "done"), isolated: true, error: null });
    expect(useLive.getState().scan).toMatchObject({ isolated: true, error: null });
  });
});

function step(layer: LayerName, status: LayerStatus = "pass"): LayerResult {
  return { layer, title: layer, status, finding: "f", evidence: [], explanation: "e", fix: null };
}
function run(over: Partial<DiagnosisRun> = {}): DiagnosisRun {
  return { id: 3, state: "running", started_at: 1, finished_at: null, steps: [], verdict: null, error: null, ...over };
}
const verdict = (): Verdict => ({ finding: "ok", status: "pass", layer: null, headline: "All good", fix: null });
const stepEv = (seq: number, run_id: number, s: LayerResult) => ({
  seq,
  api_version: 1,
  type: "diagnose.step",
  data: { run_id, step: s },
});

describe("diagnosis events", () => {
  it("applies steps and done for the followed run only", () => {
    useLive.getState().followDiagnosis(run({ id: 3 }));
    useLive.getState().applyEvent(stepEv(1, 3, step("link")));
    useLive.getState().applyEvent(stepEv(2, 2, step("router", "fail")));
    expect(useLive.getState().diagnosis!.steps.map((s) => s.layer)).toEqual(["link"]);
    useLive.getState().applyEvent({
      seq: 3,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 3, state: "done", verdict: verdict(), error: null },
    });
    expect(useLive.getState().diagnosis).toMatchObject({ state: "done", verdict: verdict(), error: null });
    useLive.getState().applyEvent({
      seq: 4,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 2, state: "failed", verdict: null, error: "x" },
    });
    expect(useLive.getState().diagnosis!.state).toBe("done");
  });

  it("keeps steps in layer order and replaces a step by layer", () => {
    useLive.getState().followDiagnosis(run());
    useLive.getState().applyEvent(stepEv(1, 3, step("dns")));
    useLive.getState().applyEvent(stepEv(2, 3, step("link")));
    useLive.getState().applyEvent(stepEv(3, 3, step("internet")));
    useLive.getState().applyEvent(stepEv(4, 3, step("link", "warn")));
    expect(useLive.getState().diagnosis!.steps.map((s) => [s.layer, s.status])).toEqual([
      ["link", "warn"],
      ["internet", "pass"],
      ["dns", "pass"],
    ]);
  });

  it("records a failed run's error", () => {
    useLive.getState().followDiagnosis(run());
    useLive.getState().applyEvent({
      seq: 1,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 3, state: "failed", verdict: null, error: "boom" },
    });
    expect(useLive.getState().diagnosis).toMatchObject({ state: "failed", verdict: null, error: "boom" });
  });

  it("stamps finished_at on done when the run has none, and keeps an existing one", () => {
    useLive.getState().followDiagnosis(run());
    const before = Date.now();
    useLive.getState().applyEvent({
      seq: 1,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 3, state: "done", verdict: verdict(), error: null },
    });
    expect(useLive.getState().diagnosis!.finished_at).toBeGreaterThanOrEqual(before);

    useLive.getState().followDiagnosis(run({ finished_at: 42 }));
    useLive.getState().applyEvent({
      seq: 2,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 3, state: "done", verdict: verdict(), error: null },
    });
    expect(useLive.getState().diagnosis!.finished_at).toBe(42);
  });
});

describe("a run started elsewhere", () => {
  it("follows a step for a run when none is followed, and loads its earlier steps", async () => {
    mockFetch({ "GET /api/diagnose/7": [200, run({ id: 7, steps: [step("link"), step("router")] })] });
    useLive.getState().applyEvent(stepEv(1, 7, step("internet")));
    expect(useLive.getState().diagnosis).toMatchObject({ id: 7, state: "running" });
    expect(useLive.getState().diagnosis!.steps.map((s) => s.layer)).toEqual(["internet"]);
    await vi.waitFor(() =>
      expect(useLive.getState().diagnosis!.steps.map((s) => s.layer)).toEqual(["link", "router", "internet"]),
    );
  });

  it("switches to a newer run's step and then applies its done", () => {
    mockFetch({});
    useLive.getState().followDiagnosis(run({ id: 3, state: "done", verdict: verdict() }));
    useLive.getState().applyEvent(stepEv(1, 9, step("link")));
    expect(useLive.getState().diagnosis).toMatchObject({ id: 9, state: "running" });
    useLive.getState().applyEvent({
      seq: 2,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 9, state: "done", verdict: verdict(), error: null },
    });
    expect(useLive.getState().diagnosis).toMatchObject({ id: 9, state: "done", verdict: verdict() });
  });

  it("follows a done event for a newer run with nothing followed", () => {
    mockFetch({});
    useLive.getState().applyEvent({
      seq: 1,
      api_version: 1,
      type: "diagnose.done",
      data: { run_id: 4, state: "done", verdict: verdict(), error: null },
    });
    expect(useLive.getState().diagnosis).toMatchObject({ id: 4, state: "done", verdict: verdict() });
  });
});
