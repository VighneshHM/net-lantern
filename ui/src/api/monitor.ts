import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { apiFetch, unwrap } from "./client";

export type Range = "1h" | "24h" | "7d" | "30d";

export type MonitorState = "monitoring" | "waiting_for_scan" | "not_connected";

export type Check =
  | { kind: "tcp"; port: number }
  | { kind: "http"; port: number; path: string; https: boolean };

export interface CheckOutcome {
  check: Check;
  ok: boolean;
  at: number;
  detail: string;
}

export interface DeviceChecks {
  device_id: number;
  outcomes: CheckOutcome[];
}

export interface MonitorTick {
  ts: number;
  network_id: number;
  gateway_rtt_ms: number | null;
  internet_rtt_ms: number | null;
  router_lost: number;
  router_window: number;
  dns_ms: number | null;
  wifi_rssi_dbm: number | null;
}

export interface MonitorStatus {
  state: MonitorState;
  network_id: number | null;
  session_started_at: number | null;
  last: MonitorTick | null;
  checks: DeviceChecks[];
}

export interface MetricPoint {
  ts: number;
  min: number | null;
  avg: number | null;
  max: number | null;
  count: number;
  lost: number;
}

export interface SessionSpan {
  started_at: number;
  ended_at: number | null;
  end_reason: "sleep" | "network_left" | "quit" | "crash" | null;
}

export interface MetricSeries {
  resolution: "raw" | "five_min";
  points: MetricPoint[];
  sessions: SessionSpan[];
}

export function useMonitor() {
  return useQuery({
    queryKey: ["monitor"],
    queryFn: async () => unwrap(await apiFetch<MonitorStatus>("/api/monitor")),
    retry: false,
  });
}

/**
 * One metric of one target over a range; refreshed every minute. A range
 * change keeps showing the previous range's data until the new one arrives.
 */
export function useMetrics(target: string, metric: string, range: Range) {
  return useQuery({
    queryKey: ["metrics", target, metric, range],
    queryFn: async () =>
      unwrap(
        await apiFetch<MetricSeries>(
          `/api/metrics?target=${encodeURIComponent(target)}&metric=${encodeURIComponent(metric)}&range=${range}`,
        ),
      ),
    refetchInterval: 60_000,
    placeholderData: keepPreviousData,
    retry: false,
  });
}
