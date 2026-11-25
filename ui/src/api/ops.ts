import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiFetch, ApiRequestError, unwrap } from "./client";

export interface HostInfo {
  kind: "exe" | "desktop";
  version: string;
  autostart: { available: boolean; enabled: boolean | null };
}

export interface UpdateStatus {
  setting: boolean | null;
  current: string;
  latest: string | null;
  newer: boolean;
  url: string | null;
  checked_at: number | null;
}

/** The text the server's 500 `reset_failed` carries when the data was cleared but an old backup file stayed. */
export const BACKUP_LEFT_MESSAGE = "Your data was cleared, but an old backup file could not be removed.";

export function useHost() {
  return useQuery({
    queryKey: ["host"],
    queryFn: async () => unwrap(await apiFetch<HostInfo>("/api/host")),
    retry: false,
  });
}

export function useUpdate() {
  return useQuery({
    queryKey: ["update"],
    queryFn: async () => unwrap(await apiFetch<UpdateStatus>("/api/update")),
    retry: false,
  });
}

/** Turns the daily update check on or off. Throws an `ApiRequestError` on failure. */
export function useSetUpdateCheck(): (on: boolean) => Promise<UpdateStatus> {
  const queryClient = useQueryClient();
  return async (on) => {
    const res = await apiFetch<UpdateStatus>("/api/update", {
      method: "PUT",
      body: JSON.stringify({ enabled: on }),
    });
    if (!res.ok) throw new ApiRequestError(res.status, res.error);
    queryClient.setQueryData(["update"], res.data);
    return res.data;
  };
}

/** Turns start at login on or off. Throws an `ApiRequestError` on failure (403 `phone_not_allowed` from a phone). */
export function useSetAutostart(): (on: boolean) => Promise<HostInfo> {
  const queryClient = useQueryClient();
  return async (on) => {
    const res = await apiFetch<HostInfo>("/api/host/autostart", {
      method: "PUT",
      body: JSON.stringify({ enabled: on }),
    });
    if (!res.ok) throw new ApiRequestError(res.status, res.error);
    queryClient.setQueryData(["host"], res.data);
    return res.data;
  };
}

/** The redacted debug text. Plain text, so it does not go through `apiFetch`. */
export async function fetchDebugInfo(): Promise<string> {
  let res: Response;
  try {
    res = await fetch("/api/debug-info", { credentials: "same-origin" });
  } catch {
    throw new Error("Could not reach Net Lantern.");
  }
  if (!res.ok) throw new Error("Net Lantern couldn't gather the debug info.");
  return await res.text();
}

/**
 * Deletes every stored record. Throws an `ApiRequestError`; status 0 means the
 * connection dropped (a phone is cut off when the reset turns phone access off).
 */
export async function resetAllData(): Promise<void> {
  const res = await apiFetch<unknown>("/api/data/reset", {
    method: "POST",
    body: JSON.stringify({ confirm: "delete all data" }),
  });
  if (!res.ok) throw new ApiRequestError(res.status, res.error);
}
