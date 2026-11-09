import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useLive } from "../state/live";
import { API_VERSION } from "./client";
import { fetchDiagnosis, fetchLatestDiagnosis, type DiagnosisRun } from "./diagnose";
import { fetchLatestScan, followIfRunning, refreshScan } from "./queries";

export type ServerEvent = { seq: number; api_version: number; type: string; data?: unknown };

/**
 * Sequence tracking. `hello` sets the baseline; after that every event must
 * be exactly `lastSeq + 1`, and anything else (or an event before any hello)
 * means events were missed.
 */
export function track(
  lastSeq: number | null,
  ev: ServerEvent,
): { lastSeq: number; gap: boolean } {
  if (ev.type === "hello") return { lastSeq: ev.seq, gap: false };
  return { lastSeq: ev.seq, gap: lastSeq === null || ev.seq !== lastSeq + 1 };
}

export interface EventHandlers {
  onEvent(ev: ServerEvent): void;
  /** Reload state over REST: after every (re)connect and whenever events were missed. */
  onResync(): void;
  /** `true` on each hello; `false` at start (before the first hello) and on every close. */
  onStatus(open: boolean): void;
  onVersionMismatch(): void;
}

const MIN_DELAY_MS = 1000;
const MAX_DELAY_MS = 10_000;

function parse(data: unknown): ServerEvent | null {
  if (typeof data !== "string") return null;
  try {
    const v = JSON.parse(data) as Partial<ServerEvent> | null;
    if (v && typeof v.type === "string" && typeof v.seq === "number") return v as ServerEvent;
  } catch {
    // Not JSON: ignore. Never log the message itself.
  }
  return null;
}

/**
 * Open the live-event WebSocket and keep it open: on close, reconnect after
 * 1 s, doubling to 10 s, with the delay reset by each successful hello.
 * Returns a cleanup function that closes the socket and stops reconnecting.
 * `WS` is injectable for tests.
 */
export function connectEvents(
  handlers: EventHandlers,
  WS: typeof WebSocket = globalThis.WebSocket,
): () => void {
  const url = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/events`;
  let stopped = false;
  let delay = MIN_DELAY_MS;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let socket: WebSocket | undefined;

  function scheduleReconnect() {
    timer = setTimeout(open, delay);
    delay = Math.min(delay * 2, MAX_DELAY_MS);
  }

  function open() {
    let ws: WebSocket;
    try {
      ws = new WS(url);
    } catch {
      // A throwing constructor must not end reconnection.
      scheduleReconnect();
      return;
    }
    socket = ws;
    let lastSeq: number | null = null;

    ws.onmessage = (e) => {
      const ev = parse(e.data);
      if (!ev) return;
      if (ev.api_version !== API_VERSION) {
        handlers.onVersionMismatch();
        return;
      }
      const t = track(lastSeq, ev);
      lastSeq = t.lastSeq;
      if (ev.type === "hello") {
        delay = MIN_DELAY_MS;
        handlers.onStatus(true);
        handlers.onResync();
        return;
      }
      handlers.onEvent(ev);
      if (t.gap) handlers.onResync();
    };

    ws.onclose = () => {
      if (stopped) return;
      handlers.onStatus(false);
      scheduleReconnect();
    };
  }

  handlers.onStatus(false);
  open();

  return () => {
    stopped = true;
    clearTimeout(timer);
    socket?.close();
  };
}

const POLL_MS = 2000;

/** Load a snapshot of the followed run, unless a finish event already ended it. */
function followSnapshot(run: DiagnosisRun | null): void {
  if (!run) return;
  const live = useLive.getState();
  const cur = live.diagnosis;
  if (cur?.id === run.id && cur.state !== "running" && run.state === "running") return;
  live.followDiagnosis(run);
}

/**
 * Keep the live store and the query cache current from the WebSocket. While
 * the socket is down it falls back to polling over REST. Call it once, at App
 * level, inside the authenticated part of the app. `WS` is injectable for tests.
 */
export function useLiveEvents(WS: typeof WebSocket = globalThis.WebSocket): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    let alive = true;
    let poll: ReturnType<typeof setInterval> | undefined;
    let busy = false;

    const refreshLists = () => {
      void queryClient.invalidateQueries({ queryKey: ["network"] });
      void queryClient.invalidateQueries({ queryKey: ["devices"] });
      void queryClient.invalidateQueries({ queryKey: ["device"] });
    };

    const tick = async () => {
      if (busy) return;
      busy = true;
      try {
        const scan = useLive.getState().scan;
        if (scan?.state === "running") {
          await refreshScan(scan.id);
          if (alive) void queryClient.invalidateQueries({ queryKey: ["devices"] });
        } else if (!scan) {
          const latest = await fetchLatestScan();
          if (alive) followIfRunning(latest);
        }
        const diagnosis = useLive.getState().diagnosis;
        if (diagnosis?.state === "running") {
          const run = await fetchDiagnosis(diagnosis.id);
          if (alive) followSnapshot(run);
        }
      } finally {
        busy = false;
      }
    };

    const stop = connectEvents(
      {
        onEvent(ev) {
          useLive.getState().applyEvent(ev);
          if (ev.type === "scan.device" || ev.type === "scan.done") {
            void queryClient.invalidateQueries({ queryKey: ["devices"] });
            // An open details panel reads ["device", id]; refresh it too.
            void queryClient.invalidateQueries({ queryKey: ["device"] });
          }
        },
        onResync() {
          refreshLists();
          const scan = useLive.getState().scan;
          if (scan) void refreshScan(scan.id);
          // Catch a scan started elsewhere before this tab connected.
          void fetchLatestScan().then((latest) => {
            if (alive) followIfRunning(latest);
          });
          const diagnosis = useLive.getState().diagnosis;
          if (diagnosis?.state === "running") {
            void fetchDiagnosis(diagnosis.id).then((run) => {
              if (alive) followSnapshot(run);
            });
          }
          // Catch a diagnosis started elsewhere before this tab connected.
          void fetchLatestDiagnosis().then((latest) => {
            if (!alive || !latest || latest.state !== "running") return;
            if (useLive.getState().diagnosis?.id !== latest.id) {
              useLive.getState().followDiagnosis(latest);
            }
          });
        },
        onStatus(open) {
          if (open) {
            clearInterval(poll);
            poll = undefined;
          } else if (poll === undefined) {
            poll = setInterval(() => void tick(), POLL_MS);
          }
        },
        onVersionMismatch() {
          useLive.getState().markOutdated();
        },
      },
      WS,
    );
    return () => {
      alive = false;
      clearInterval(poll);
      stop();
    };
  }, [queryClient, WS]);
}
