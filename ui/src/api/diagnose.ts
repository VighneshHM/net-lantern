import { apiFetch, ApiRequestError } from "./client";

export type LayerName = "link" | "router" | "internet" | "dns" | "website";
export type LayerStatus = "pass" | "warn" | "fail" | "skipped";

/** The layers in the order a diagnosis runs them. */
export const LAYERS: LayerName[] = ["link", "router", "internet", "dns", "website"];

export interface LayerResult {
  layer: LayerName;
  title: string;
  status: LayerStatus;
  finding: string;
  evidence: string[];
  explanation: string;
  fix: string | null;
}

export interface Verdict {
  finding: string;
  status: LayerStatus;
  layer: LayerName | null;
  headline: string;
  fix: string | null;
}

export interface DiagnosisRun {
  id: number;
  state: "running" | "done" | "failed";
  started_at: number;
  finished_at: number | null;
  steps: LayerResult[];
  verdict: Verdict | null;
  error: string | null;
}

/**
 * Start a diagnosis and return its run id. If one is already running, that
 * run's id is returned, so the caller just follows it. Any other failure
 * throws an `ApiRequestError`.
 */
export async function startDiagnosis(): Promise<number> {
  const res = await apiFetch<{ run_id: number; already_running: boolean }>("/api/diagnose", {
    method: "POST",
  });
  if (res.ok) return res.data.run_id;
  throw new ApiRequestError(res.status, res.error);
}

/** A run by id, or null when it is unknown (or the call failed; events catch up). */
export async function fetchDiagnosis(id: number): Promise<DiagnosisRun | null> {
  const res = await apiFetch<DiagnosisRun>(`/api/diagnose/${id}`);
  return res.ok ? res.data : null;
}

/** The most recent run, or null when there has never been one (or the call failed). */
export async function fetchLatestDiagnosis(): Promise<DiagnosisRun | null> {
  const res = await apiFetch<DiagnosisRun>("/api/diagnose/latest");
  return res.ok ? res.data : null;
}
