import { useQuery } from "@tanstack/react-query";
import type { DeviceType } from "../lib/deviceType";
import { useLive, type LiveScan } from "../state/live";
import { apiFetch, ApiRequestError, unwrap } from "./client";
import type { Check } from "./monitor";

export { ApiRequestError, unwrap };

export interface Network {
  ip: string;
  prefix: number;
  gateway: string | null;
  dns_servers: string[];
  kind: string;
  mac: string | null;
}

export interface Device {
  id: number;
  ip: string | null;
  mac: string | null;
  private_mac: boolean;
  vendor: string | null;
  hostname: string | null;
  name: string | null;
  display_name: string;
  type: DeviceType;
  type_confidence: number;
  starred: boolean;
  is_new: boolean;
  online: boolean;
  answers_ping: boolean;
  rtt_ms: number | null;
  is_self: boolean;
  is_gateway: boolean;
  first_seen: number;
  last_seen: number;
  services: { source: string; value: string }[];
  open_ports: number[];
  /** Service checks run on a starred device (Phase 3). */
  checks?: Check[];
  /** When presence last flipped between online and offline; null if never. */
  presence_changed_at?: number | null;
}

export interface ScanStatus {
  id: number;
  mode: string;
  state: string;
  stage: string;
  done: number;
  total: number;
  devices_found: number;
  error: string | null;
  isolated: boolean;
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

export function useDevice(id: number) {
  return useQuery({
    queryKey: ["device", id],
    queryFn: async () => unwrap(await apiFetch<Device>(`/api/devices/${id}`)),
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

function toLive(s: ScanStatus): LiveScan {
  return {
    id: s.id,
    state: s.state,
    stage: s.stage,
    done: s.done,
    total: s.total,
    devicesFound: s.devices_found,
    error: s.error,
    isolated: s.isolated,
  };
}

/** Reload a scan's state over REST into the live store. Failures are ignored; events will catch up. */
export async function refreshScan(id: number): Promise<void> {
  const res = await apiFetch<ScanStatus>(`/api/scans/${id}`);
  if (!res.ok) return;
  useLive.getState().setScan(toLive(res.data));
}

/** The most recent scan, or null when there has never been one (or the call failed). */
export async function fetchLatestScan(): Promise<ScanStatus | null> {
  const res = await apiFetch<ScanStatus>("/api/scans/latest");
  return res.ok ? res.data : null;
}

/** Follow `s` if it is running and is not the scan already followed. */
export function followIfRunning(s: ScanStatus | null): void {
  if (!s || s.state !== "running") return;
  const live = useLive.getState();
  if (live.scan?.id === s.id) return;
  live.attach(s.id);
  live.setScan(toLive(s));
}
