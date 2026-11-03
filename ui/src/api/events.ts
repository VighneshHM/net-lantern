import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useLive } from "../state/live";
import { API_VERSION } from "./client";
import { refreshScan } from "./queries";

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

  function open() {
    const ws = new WS(url);
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
        handlers.onResync();
        return;
      }
      handlers.onEvent(ev);
      if (t.gap) handlers.onResync();
    };

    ws.onclose = () => {
      if (stopped) return;
      timer = setTimeout(open, delay);
      delay = Math.min(delay * 2, MAX_DELAY_MS);
    };
  }

  open();

  return () => {
    stopped = true;
    clearTimeout(timer);
    socket?.close();
  };
}

/**
 * Keep the live store and the query cache current from the WebSocket. Call it
 * once, at App level, inside the authenticated part of the app.
 */
export function useLiveEvents(): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    const refreshLists = () => {
      void queryClient.invalidateQueries({ queryKey: ["network"] });
      void queryClient.invalidateQueries({ queryKey: ["devices"] });
    };
    return connectEvents({
      onEvent(ev) {
        useLive.getState().applyEvent(ev);
        if (ev.type === "scan.device" || ev.type === "scan.done") {
          void queryClient.invalidateQueries({ queryKey: ["devices"] });
        }
      },
      onResync() {
        refreshLists();
        const scan = useLive.getState().scan;
        if (scan) void refreshScan(scan.id);
      },
      onVersionMismatch() {
        useLive.getState().markOutdated();
      },
    });
  }, [queryClient]);
}
