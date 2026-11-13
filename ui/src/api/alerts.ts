import { useQuery } from "@tanstack/react-query";
import { apiFetch, ApiRequestError, unwrap } from "./client";
import type { Check } from "./monitor";
import type { Device } from "./queries";

export type Severity = "critical" | "warning" | "info";
export type AlertState = "open" | "acked" | "resolved";
export type Rule =
  | "starred_offline"
  | "internet_down"
  | "latency_high"
  | "packet_loss"
  | "new_device";

export interface Alert {
  id: number;
  rule: Rule;
  key: string;
  severity: Severity;
  title: string;
  detail: string;
  suppressed: boolean;
  state: AlertState;
  incident_id: number;
  opened_at: number;
  acked_at: number | null;
  resolved_at: number | null;
}

export type TimelineKind =
  | "last_good"
  | "detected"
  | "alerted"
  | "affected"
  | "diagnosed"
  | "acknowledged"
  | "recovered";

export interface TimelineEvent {
  ts: number;
  kind: TimelineKind;
  text: string;
  alert_id: number | null;
}

export interface Incident {
  id: number;
  opened_at: number;
  resolved_at: number | null;
  root: Alert;
  alerts: Alert[];
  timeline: TimelineEvent[];
}

export interface RuleSettings {
  starred_offline: boolean;
  internet_down: boolean;
  latency_high: boolean;
  packet_loss: boolean;
  new_device: boolean;
  notifications: boolean;
}

export type AlertsFilter = "all" | "active" | "open" | "acked" | "resolved";

export function useAlerts(state: AlertsFilter) {
  return useQuery({
    queryKey: ["alerts", state],
    queryFn: async () => unwrap(await apiFetch<Alert[]>(`/api/alerts?state=${state}`)),
    retry: false,
  });
}

export function useIncident(id: number) {
  return useQuery({
    queryKey: ["incident", id],
    queryFn: async () => unwrap(await apiFetch<Incident>(`/api/incidents/${id}`)),
    retry: false,
  });
}

export function useRules() {
  return useQuery({
    queryKey: ["rules"],
    queryFn: async () => unwrap(await apiFetch<RuleSettings>("/api/rules")),
    retry: false,
  });
}

/** Acknowledge an alert. Throws `ApiRequestError` on failure. */
export async function ackAlert(id: number): Promise<Alert> {
  const res = await apiFetch<Alert>(`/api/alerts/${id}/ack`, { method: "POST" });
  if (res.ok) return res.data;
  throw new ApiRequestError(res.status, res.error);
}

/** Save all six rule switches. Throws `ApiRequestError` on failure. */
export async function saveRules(r: RuleSettings): Promise<RuleSettings> {
  const res = await apiFetch<RuleSettings>("/api/rules", {
    method: "PUT",
    body: JSON.stringify(r),
  });
  if (res.ok) return res.data;
  throw new ApiRequestError(res.status, res.error);
}

/** Replace a starred device's service checks. Throws `ApiRequestError` on failure. */
export async function saveChecks(deviceId: number, checks: Check[]): Promise<Device> {
  const res = await apiFetch<Device>(`/api/devices/${deviceId}`, {
    method: "PATCH",
    body: JSON.stringify({ checks }),
  });
  if (res.ok) return res.data;
  throw new ApiRequestError(res.status, res.error);
}
