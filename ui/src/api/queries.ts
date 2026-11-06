import { useQuery } from "@tanstack/react-query";
import { useLive } from "../state/live";
import { apiFetch, type ApiError, type ApiResult } from "./client";

export interface Network {
  ip: string;
  prefix: number;
  gateway: string | null;
  dns_servers: string[];
  kind: string;
  mac: string | null;
}

export interface Device {
  ip: string;
  mac: string | null;
  private_mac: boolean;
  answers_ping: boolean;
  rtt_ms: number | null;
  is_self: boolean;
  is_gateway: boolean;
}

export interface ScanStatus {
  id: number;
  mode: string;
  state: string;
  stage: string;
  done: number;
  total: number;
  devices_found: number;
}

/** A failed API call, as thrown into TanStack Query (which wants errors thrown). */
export class ApiRequestError extends Error {
  readonly status: number;
  readonly error: ApiError;
  constructor(status: number, error: ApiError) {
    super(error.message);
    this.name = "ApiRequestError";
    this.status = status;
    this.error = error;
  }
}

function unwrap<T>(res: ApiResult<T>): T {
  if (res.ok) return res.data;
  throw new ApiRequestError(res.status, res.error);
}

export function useNetwork() {
  return useQuery({
    queryKey: ["network"],
    queryFn: async () => unwrap(await apiFetch<Network>("/api/network")),
    retry: false,
  });
}

export function useDevices() {
  return useQuery({
    queryKey: ["devices"],
    queryFn: async () => unwrap(await apiFetch<Device[]>("/api/devices")),
    retry: false,
  });
}

/**
 * Start a scan and return its id. If one is already running (409 — double
 * click, second tab) that scan's id is returned, so the caller just follows it.
 * Any other failure throws an `ApiRequestError`.
 */
export async function startScan(mode: "quick" | "deep"): Promise<number> {
  const res = await apiFetch<{ scan_id: number }>("/api/scans", {
    method: "POST",
    body: JSON.stringify({ mode }),
  });
  if (res.ok) return res.data.scan_id;
  if (res.status === 409 && typeof res.error.scan_id === "number") return res.error.scan_id;
  throw new ApiRequestError(res.status, res.error);
}

/** Reload a scan's state over REST into the live store. Failures are ignored; events will catch up. */
export async function refreshScan(id: number): Promise<void> {
  const res = await apiFetch<ScanStatus>(`/api/scans/${id}`);
  if (!res.ok) return;
  const s = res.data;
  useLive.getState().setScan({
    id: s.id,
    state: s.state,
    stage: s.stage,
    done: s.done,
    total: s.total,
    devicesFound: s.devices_found,
  });
}
