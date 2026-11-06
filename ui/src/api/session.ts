import { apiFetch } from "./client";

/**
 * Turn the one-time `?t=<token>` launch link into a session cookie, then check
 * that the cookie works.
 *
 * - If `?t=` is present, POST it to `/api/session` (its status is ignored: a
 *   used-up token is fine when the cookie from an earlier launch is still
 *   valid), then ALWAYS remove `t` from the address bar so the token never
 *   lingers in history or gets bookmarked.
 * - Then `GET /api/network`: 200 or 503 (`not_connected`) mean we have a session
 *   -> "ok"; 401 -> "needs-launch".
 * - Any other outcome (network error, 5xx other than 503) also returns "ok":
 *   that is not a session problem, so the dashboard shows its own error state
 *   rather than telling the user their link expired.
 *
 * Never log the token or the URL it came in.
 */
export async function ensureSession(
  loc: Location = window.location,
  hist: History = window.history,
): Promise<"ok" | "needs-launch"> {
  const params = new URLSearchParams(loc.search);
  const token = params.get("t");

  if (token !== null) {
    await apiFetch("/api/session", { method: "POST", body: JSON.stringify({ token }) });
    params.delete("t");
    const rest = params.toString();
    hist.replaceState(hist.state, "", loc.pathname + (rest ? `?${rest}` : "") + loc.hash);
  }

  const res = await apiFetch("/api/network");
  if (!res.ok && res.status === 401) return "needs-launch";
  return "ok";
}
