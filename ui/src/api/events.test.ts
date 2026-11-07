import { createElement, type ReactNode } from "react";
import { QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useLive } from "../state/live";
import { mockFetch } from "../test/mockFetch";
import { newClient, resetLive } from "../test/render";
import { connectEvents, track, useLiveEvents, type ServerEvent } from "./events";

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
  return { onEvent: vi.fn(), onResync: vi.fn(), onVersionMismatch: vi.fn() };
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
    return { invalidate, hook, ws: FakeSocket.instances[0] };
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

    invalidate.mockClear();
    ws.receive(ev("scan.done", 4, { scan_id: 3, state: "done", devices_found: 13 }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["devices"] });
    expect(useLive.getState().scan).toMatchObject({ state: "done", devicesFound: 13 });
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
