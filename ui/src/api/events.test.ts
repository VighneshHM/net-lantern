import { createElement, type ReactNode } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useLive } from "../state/live";
import { mockFetch } from "../test/mockFetch";
import { newClient, resetLive } from "../test/render";
import { connectEvents, track, useLiveEvents, type ServerEvent } from "./events";
import * as queries from "./queries";
import type { MonitorStatus } from "./monitor";
import { monitorStatus, tick } from "../test/fixtures";

function ev(type: string, seq: number, data?: unknown, api_version = 1): ServerEvent {
  return { seq, api_version, type, data };
}

class FakeSocket {
  static instances: FakeSocket[] = [];
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  closed = false;
  constructor(public url: string) {
    FakeSocket.instances.push(this);
  }
  close() {
    this.closed = true;
  }
  receive(event: ServerEvent) {
    this.onmessage?.({ data: JSON.stringify(event) });
  }
  drop() {
    this.onclose?.();
  }
}
const FakeWS = FakeSocket as unknown as typeof WebSocket;

function handlers() {
  return { onEvent: vi.fn(), onResync: vi.fn(), onStatus: vi.fn(), onVersionMismatch: vi.fn() };
}

describe("track", () => {
  it("hello sets the baseline; consecutive is fine; a skip is a gap", () => {
    let s = track(null, ev("hello", 5));
    expect(s).toEqual({ lastSeq: 5, gap: false });
    s = track(s.lastSeq, ev("scan.progress", 6));
    expect(s.gap).toBe(false);
    s = track(s.lastSeq, ev("scan.progress", 8));
    expect(s.gap).toBe(true);
  });

  it("an event before any hello counts as a gap", () => {
    expect(track(null, ev("scan.done", 3)).gap).toBe(true);
  });
});

describe("connectEvents", () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("connects to /api/events on the same host", () => {
    const stop = connectEvents(handlers(), FakeWS);
    expect(FakeSocket.instances).toHaveLength(1);
    expect(FakeSocket.instances[0].url).toBe(`ws://${location.host}/api/events`);
    stop();
  });

  it("resyncs on hello, passes events through, and does not treat hello as an event", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    const ws = FakeSocket.instances[0];
    ws.receive(ev("hello", 10));
    expect(h.onResync).toHaveBeenCalledTimes(1);
    expect(h.onEvent).not.toHaveBeenCalled();
    ws.receive(ev("scan.progress", 11, { scan_id: 1 }));
    expect(h.onEvent).toHaveBeenCalledWith(ev("scan.progress", 11, { scan_id: 1 }));
    expect(h.onResync).toHaveBeenCalledTimes(1);
    stop();
  });

  it("resyncs when a sequence number is skipped", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    const ws = FakeSocket.instances[0];
    ws.receive(ev("hello", 1));
    ws.receive(ev("scan.progress", 2));
    ws.receive(ev("scan.progress", 4));
    expect(h.onResync).toHaveBeenCalledTimes(2);
    stop();
  });

  it("reports an api_version other than 1 and ignores that message", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    FakeSocket.instances[0].receive(ev("hello", 1, undefined, 2));
    expect(h.onVersionMismatch).toHaveBeenCalledTimes(1);
    expect(h.onResync).not.toHaveBeenCalled();
    stop();
  });

  it("ignores messages that are not JSON events", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    const ws = FakeSocket.instances[0];
    ws.onmessage?.({ data: "not json" });
    ws.onmessage?.({ data: JSON.stringify({ nope: true }) });
    expect(h.onEvent).not.toHaveBeenCalled();
    expect(h.onResync).not.toHaveBeenCalled();
    stop();
  });

  it("reconnects after 1 s, then backs off to a 10 s cap, and resets after a hello", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    FakeSocket.instances[0].drop();
    vi.advanceTimersByTime(999);
    expect(FakeSocket.instances).toHaveLength(1);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.instances).toHaveLength(2);

    // Second drop without a hello: 2 s.
    FakeSocket.instances[1].drop();
    vi.advanceTimersByTime(1999);
    expect(FakeSocket.instances).toHaveLength(2);
    vi.advanceTimersByTime(1);
    expect(FakeSocket.instances).toHaveLength(3);

    // Keep failing: 4 s, 8 s, then capped at 10 s.
    for (const [i, wait] of [4000, 8000, 10000, 10000].entries()) {
      FakeSocket.instances[2 + i].drop();
      vi.advanceTimersByTime(wait - 1);
      expect(FakeSocket.instances).toHaveLength(3 + i);
      vi.advanceTimersByTime(1);
      expect(FakeSocket.instances).toHaveLength(4 + i);
    }

    // A successful hello resets the delay to 1 s.
    const last = FakeSocket.instances[FakeSocket.instances.length - 1];
    last.receive(ev("hello", 1));
    last.drop();
    const n = FakeSocket.instances.length;
    vi.advanceTimersByTime(1000);
    expect(FakeSocket.instances).toHaveLength(n + 1);
    stop();
  });

  it("a reconnect hello resyncs again and starts a fresh sequence", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    FakeSocket.instances[0].receive(ev("hello", 1));
    FakeSocket.instances[0].drop();
    vi.advanceTimersByTime(1000);
    const second = FakeSocket.instances[1];
    second.receive(ev("hello", 50));
    second.receive(ev("scan.progress", 51));
    expect(h.onResync).toHaveBeenCalledTimes(2);
    stop();
  });

  it("stops reconnecting once cleaned up", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    const ws = FakeSocket.instances[0];
    ws.drop();
    stop();
    expect(ws.closed).toBe(true);
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.instances).toHaveLength(1);

    // Cleaning up a live socket also stops the reconnect that its close would schedule.
    const stop2 = connectEvents(h, FakeWS);
    const ws2 = FakeSocket.instances[1];
    stop2();
    ws2.drop();
    vi.advanceTimersByTime(60_000);
    expect(FakeSocket.instances).toHaveLength(2);
  });
});

describe("connectEvents status and constructor failures", () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reports closed at start, open on hello, closed on close", () => {
    const h = handlers();
    const stop = connectEvents(h, FakeWS);
    expect(h.onStatus.mock.calls).toEqual([[false]]);
    FakeSocket.instances[0].receive(ev("hello", 1));
    expect(h.onStatus.mock.calls).toEqual([[false], [true]]);
    FakeSocket.instances[0].drop();
    expect(h.onStatus.mock.calls).toEqual([[false], [true], [false]]);
    stop();
  });

  it("a throwing WebSocket constructor still reconnects with backoff", () => {
    let calls = 0;
    class Flaky extends FakeSocket {
      constructor(url: string) {
        super(url);
        calls++;
        if (calls <= 2) throw new Error("boom");
      }
    }
    const stop = connectEvents(handlers(), Flaky as unknown as typeof WebSocket);
    expect(calls).toBe(1);
    vi.advanceTimersByTime(999);
    expect(calls).toBe(1);
    vi.advanceTimersByTime(1);
    expect(calls).toBe(2);
    vi.advanceTimersByTime(1999);
    expect(calls).toBe(2);
    vi.advanceTimersByTime(1);
    expect(calls).toBe(3);
    stop();
  });
});

describe("useLiveEvents", () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    resetLive();
    vi.stubGlobal("WebSocket", FakeWS);
  });

  function setup() {
    const client = newClient();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    const hook = renderHook(() => useLiveEvents(), { wrapper });
    return { invalidate, hook, client, ws: FakeSocket.instances[0] };
  }

  it("opens one socket, applies scan events to the store and refreshes devices", () => {
    const { invalidate, ws } = setup();
    expect(FakeSocket.instances).toHaveLength(1);
    ws.receive(ev("hello", 1));
    invalidate.mockClear();

    ws.receive(ev("scan.progress", 2, { scan_id: 3, done: 10, total: 100, stage: "sweep" }));
    expect(useLive.getState().scan).toMatchObject({ id: 3, done: 10, total: 100 });
    expect(invalidate).not.toHaveBeenCalled();

    ws.receive(ev("scan.device", 3, { scan_id: 3, device: {} }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["devices"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["device"] });

    invalidate.mockClear();
    ws.receive(ev("scan.done", 4, { scan_id: 3, state: "done", devices_found: 13 }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["devices"] });
    expect(useLive.getState().scan).toMatchObject({ state: "done", devicesFound: 13 });
  });

  it("puts a monitor tick into the monitor query without refetching", () => {
    const { invalidate, client, ws } = setup();
    client.setQueryData(["monitor"], monitorStatus({ last: null }));
    ws.receive(ev("hello", 1));
    invalidate.mockClear();
    const t = tick({ ts: 5, gateway_rtt_ms: 9 });
    ws.receive(ev("monitor.tick", 2, { tick: t }));
    expect(client.getQueryData<MonitorStatus>(["monitor"])?.last).toEqual(t);
    expect(client.getQueryData<MonitorStatus>(["monitor"])?.state).toBe("monitoring");
    expect(invalidate).not.toHaveBeenCalledWith({ queryKey: ["monitor"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["metrics"] });
  });

  it("invalidates metrics at most once per 30 s", () => {
    vi.useFakeTimers();
    const { invalidate, ws } = setup();
    ws.receive(ev("hello", 1));
    invalidate.mockClear();
    const t = tick();
    const metricsCalls = () =>
      invalidate.mock.calls.filter((c) => c[0]?.queryKey?.[0] === "metrics").length;
    ws.receive(ev("monitor.tick", 2, { tick: t }));
    vi.advanceTimersByTime(10_000);
    ws.receive(ev("monitor.tick", 3, { tick: t }));
    expect(metricsCalls()).toBe(1);
    vi.advanceTimersByTime(21_000);
    ws.receive(ev("monitor.tick", 4, { tick: t }));
    expect(metricsCalls()).toBe(2);
  });

  it("refreshes alerts when one changes", () => {
    const { invalidate, ws } = setup();
    ws.receive(ev("hello", 1));
    invalidate.mockClear();
    ws.receive(ev("alert.changed", 2, { alert: {} }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["alerts"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["incident"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["monitor"] });
  });

  it("refreshes the monitor on state changes and devices on presence changes", () => {
    const { invalidate, ws } = setup();
    ws.receive(ev("hello", 1));
    invalidate.mockClear();
    ws.receive(ev("monitor.state", 2, { state: "monitoring" }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["monitor"] });
    invalidate.mockClear();
    ws.receive(ev("device.presence", 3, { device_id: 4, online: false, at: 9 }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["devices"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["device"] });
  });

  it("on resync also refreshes the monitor, alerts and metrics", () => {
    const { invalidate, ws } = setup();
    invalidate.mockClear();
    ws.receive(ev("hello", 1));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["monitor"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["alerts"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["metrics"] });
  });

  it("refreshes the Wi-Fi cards when a diagnosis finishes, even one this tab isn't following", () => {
    const { invalidate, ws } = setup();
    ws.receive(ev("hello", 1));
    invalidate.mockClear();

    ws.receive(ev("diagnose.step", 2, { run_id: 4, step: { layer: "link" } }));
    expect(invalidate).not.toHaveBeenCalledWith({ queryKey: ["wifi"] });

    ws.receive(ev("diagnose.done", 3, { run_id: 4, state: "done", verdict: null, error: null }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["wifi"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["nearby"] });
  });

  it("on resync refreshes network and devices and reloads the attached scan over REST", async () => {
    const calls = mockFetch({
      "GET /api/scans/7": [
        200,
        { id: 7, mode: "quick", state: "running", stage: "arp", done: 254, total: 254, devices_found: 5 },
      ],
    });
    const { invalidate, ws } = setup();
    useLive.getState().attach(7);
    calls.length = 0;

    ws.receive(ev("hello", 1));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["network"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["devices"] });
    await vi.waitFor(() =>
      expect(useLive.getState().scan).toMatchObject({ id: 7, stage: "arp", devicesFound: 5 }),
    );
    expect(calls.map((c) => `${c.method} ${c.path}`)).toContain("GET /api/scans/7");
  });

  it("marks the dashboard outdated on an api_version mismatch", () => {
    const { ws } = setup();
    ws.receive(ev("hello", 1, undefined, 2));
    expect(useLive.getState().outdated).toBe(true);
  });

  it("closes the socket on unmount", () => {
    const { hook, ws } = setup();
    hook.unmount();
    expect(ws.closed).toBe(true);
  });
});

function scanStatus(over: Record<string, unknown> = {}) {
  return {
    id: 1,
    mode: "quick",
    state: "running",
    stage: "arp",
    done: 1,
    total: 10,
    devices_found: 0,
    error: null,
    isolated: false,
    ...over,
  };
}

describe("useLiveEvents REST fallback", () => {
  beforeEach(() => {
    FakeSocket.instances = [];
    resetLive();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function mountLiveEvents() {
    const client = newClient();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    const hook = renderHook(() => useLiveEvents(FakeWS), { wrapper });
    const last = () => {
      const s = FakeSocket.instances[FakeSocket.instances.length - 1];
      return {
        open() {},
        message: (e: ServerEvent) => s.receive(e),
        close: () => s.drop(),
      };
    };
    return { last, invalidate, hook };
  }

  it("polls the running scan every 2 s while the socket is down, and stops on hello", async () => {
    const refresh = vi.spyOn(queries, "refreshScan").mockResolvedValue();
    mockFetch({});
    useLive.getState().attach(7);
    const ws = mountLiveEvents();
    ws.last().close();
    await vi.advanceTimersByTimeAsync(4_100);
    expect(refresh).toHaveBeenCalledTimes(2);
    expect(refresh).toHaveBeenCalledWith(7);
    ws.last().open();
    ws.last().message(ev("hello", 3));
    refresh.mockClear();
    await vi.advanceTimersByTimeAsync(4_100);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("invalidates devices on each poll tick of a running scan", async () => {
    vi.spyOn(queries, "refreshScan").mockResolvedValue();
    useLive.getState().attach(7);
    const { invalidate } = mountLiveEvents();
    invalidate.mockClear();
    await vi.advanceTimersByTimeAsync(2_100);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["devices"] });
  });

  it("does not poll a finished scan", async () => {
    const refresh = vi.spyOn(queries, "refreshScan").mockResolvedValue();
    mockFetch({});
    useLive.setState({ scan: { ...scanStatusLive(5), state: "done" } });
    mountLiveEvents();
    await vi.advanceTimersByTimeAsync(6_100);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("with no scan followed, polls the latest scan and attaches if it is running", async () => {
    const calls = mockFetch({ "GET /api/scans/latest": [200, scanStatus({ id: 4 })] });
    mountLiveEvents();
    await vi.advanceTimersByTimeAsync(2_100);
    expect(calls.map((c) => c.path)).toContain("/api/scans/latest");
    expect(useLive.getState().scan).toMatchObject({ id: 4, state: "running", stage: "arp" });
  });

  it("does not stack poll ticks while one is still in flight", async () => {
    let release: () => void = () => {};
    const refresh = vi
      .spyOn(queries, "refreshScan")
      .mockImplementation(() => new Promise<void>((r) => (release = r)));
    useLive.getState().attach(7);
    mountLiveEvents();
    await vi.advanceTimersByTimeAsync(6_100);
    expect(refresh).toHaveBeenCalledTimes(1);
    release();
    await vi.advanceTimersByTimeAsync(2_100);
    expect(refresh).toHaveBeenCalledTimes(2);
  });

  it("stops polling on unmount", async () => {
    const refresh = vi.spyOn(queries, "refreshScan").mockResolvedValue();
    useLive.getState().attach(7);
    const { hook } = mountLiveEvents();
    hook.unmount();
    await vi.advanceTimersByTimeAsync(6_100);
    expect(refresh).not.toHaveBeenCalled();
  });

  it("resync attaches to a scan running elsewhere", async () => {
    mockFetch({ "GET /api/scans/latest": [200, scanStatus({ id: 9 })] });
    const ws = mountLiveEvents();
    ws.last().message(ev("hello", 1));
    await vi.advanceTimersByTimeAsync(0);
    expect(useLive.getState().scan).toMatchObject({ id: 9, state: "running", stage: "arp", done: 1, total: 10 });
  });

  it("resync ignores a latest scan that is finished or absent", async () => {
    mockFetch({ "GET /api/scans/latest": [200, scanStatus({ id: 9, state: "done" })] });
    const ws = mountLiveEvents();
    ws.last().message(ev("hello", 1));
    await vi.advanceTimersByTimeAsync(0);
    expect(useLive.getState().scan).toBeNull();

    mockFetch({});
    ws.last().message(ev("hello", 2));
    await vi.advanceTimersByTimeAsync(0);
    expect(useLive.getState().scan).toBeNull();
  });
});

function scanStatusLive(id: number) {
  return { id, state: "running", stage: "sweep", done: 0, total: 0, devicesFound: 0, error: null, isolated: false };
}

describe("fetchLatestScan", () => {
  it("returns the scan, or null on 404", async () => {
    mockFetch({ "GET /api/scans/latest": [200, scanStatus({ id: 2 })] });
    expect(await queries.fetchLatestScan()).toMatchObject({ id: 2 });
    mockFetch({});
    expect(await queries.fetchLatestScan()).toBeNull();
  });
});

describe("useLiveEvents diagnosis", () => {
  const RUNNING = { id: 5, state: "running", started_at: 1, finished_at: null, steps: [], verdict: null, error: null };

  beforeEach(() => {
    FakeSocket.instances = [];
    resetLive();
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  function mount() {
    const client = newClient();
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    renderHook(() => useLiveEvents(FakeWS), { wrapper });
    return FakeSocket.instances[0];
  }

  it("resync reloads a running diagnosis and attaches to one started elsewhere", async () => {
    mockFetch({ "GET /api/diagnose/latest": [200, RUNNING] });
    const ws = mount();
    ws.receive(ev("hello", 1));
    await vi.waitFor(() => expect(useLive.getState().diagnosis?.id).toBe(5));

    // A followed running run is reloaded by id.
    useLive.getState().followDiagnosis({ ...RUNNING, id: 3 } as never);
    const calls = mockFetch({
      "GET /api/diagnose/3": [200, { ...RUNNING, id: 3, steps: [{ layer: "link" }] }],
      "GET /api/diagnose/latest": [404, { code: "not_found", message: "Not found.", hint: null }],
    });
    ws.receive(ev("hello", 2));
    await vi.waitFor(() => expect(useLive.getState().diagnosis?.steps).toHaveLength(1));
    expect(calls.map((c) => c.path)).toContain("/api/diagnose/3");
    expect(useLive.getState().diagnosis?.id).toBe(3);
  });

  it("polls a running diagnosis while the socket is down", async () => {
    const calls = mockFetch({ "GET /api/diagnose/3": [200, { ...RUNNING, id: 3, steps: [{ layer: "link" }] }] });
    useLive.getState().followDiagnosis({ ...RUNNING, id: 3 } as never);
    const ws = mount();
    ws.drop();
    await vi.advanceTimersByTimeAsync(2_100);
    expect(calls.map((c) => c.path)).toContain("/api/diagnose/3");
    expect(useLive.getState().diagnosis?.steps).toHaveLength(1);
  });

  it("a short snapshot that lands after a later step keeps that step", async () => {
    // Step 3 (internet) arrived over the socket before the 2-step snapshot landed.
    const step = (layer: string) => ({ layer, title: layer, status: "pass", finding: "f", evidence: [], explanation: "e", fix: null });
    mockFetch({ "GET /api/diagnose/3": [200, { ...RUNNING, id: 3, steps: [step("link"), step("router")] }] });
    useLive.getState().followDiagnosis({ ...RUNNING, id: 3, steps: [step("link"), step("router"), step("internet")] } as never);
    const ws = mount();
    ws.receive(ev("hello", 1));
    await vi.advanceTimersByTimeAsync(10);
    expect(useLive.getState().diagnosis?.steps.map((s) => s.layer)).toEqual(["link", "router", "internet"]);
  });

  it("does not let a stale running snapshot undo a finish", async () => {
    mockFetch({ "GET /api/diagnose/3": [200, { ...RUNNING, id: 3 }] });
    useLive.getState().followDiagnosis({ ...RUNNING, id: 3, state: "done" } as never);
    const ws = mount();
    ws.receive(ev("hello", 1));
    await vi.advanceTimersByTimeAsync(10);
    expect(useLive.getState().diagnosis?.state).toBe("done");
  });

  it("ignores a late snapshot of an older run", async () => {
    mockFetch({
      "GET /api/diagnose/4": [200, { ...RUNNING, id: 4, steps: [{ layer: "link" }] }],
      "GET /api/diagnose/latest": [404, { code: "not_found", message: "Not found.", hint: null }],
    });
    useLive.getState().followDiagnosis({ ...RUNNING, id: 4 } as never);
    const ws = mount();
    ws.receive(ev("hello", 1)); // starts the snapshot request for run 4
    useLive.getState().followDiagnosis({ ...RUNNING, id: 5 } as never); // a newer run takes over
    await vi.advanceTimersByTimeAsync(10);
    expect(useLive.getState().diagnosis?.id).toBe(5);
    expect(useLive.getState().diagnosis?.steps).toHaveLength(0);
  });
});
