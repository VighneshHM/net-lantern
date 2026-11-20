import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiFetch, ApiRequestError, unwrap } from "./client";

export type PhoneState = "off" | "on" | "paused";
export type FirewallVerdict = "allowed" | "blocked" | "not_decided" | "off";

export interface PhoneStatus {
  state: PhoneState;
  /** Why it is paused or could not start: "public_network", "not_connected" or "bind_failed". */
  reason: string | null;
  /** The phone link, token included. Only present while On. */
  url: string | null;
  firewall: FirewallVerdict | null;
  category: "public" | "private" | "domain" | null;
  /** The link works on this laptop only. */
  demo: boolean;
}

export function usePhoneAccess() {
  return useQuery({
    queryKey: ["phone"],
    queryFn: async () => unwrap(await apiFetch<PhoneStatus>("/api/phone-access")),
    retry: false,
  });
}

/**
 * Returns a function that turns phone access on or off and returns the new
 * status. Throws an `ApiRequestError` on failure (409 `public_network` when
 * turning on while Windows marks the network Public; the setting is still saved).
 */
export function useSetPhoneAccess(): (enabled: boolean) => Promise<PhoneStatus> {
  const queryClient = useQueryClient();
  return async (enabled) => {
    const res = await apiFetch<PhoneStatus>("/api/phone-access", {
      method: "POST",
      body: JSON.stringify({ enabled }),
    });
    if (!res.ok) throw new ApiRequestError(res.status, res.error);
    queryClient.setQueryData(["phone"], res.data);
    return res.data;
  };
}

/** Answers the new-router question: keep the known network (true) or start a new one. */
export function useAnswerReplacedRouter(): (keep: boolean) => Promise<void> {
  const queryClient = useQueryClient();
  return async (keep) => {
    const res = await apiFetch<unknown>("/api/network/replaced-router", {
      method: "POST",
      body: JSON.stringify({ keep }),
    });
    if (!res.ok) throw new ApiRequestError(res.status, res.error);
    await queryClient.invalidateQueries({ queryKey: ["network"] });
  };
}
