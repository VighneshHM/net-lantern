export const API_VERSION = 1;

export interface ApiError {
  code: string;
  message: string;
  hint: string | null;
  /** Present on 409 `scan_in_progress`. */
  scan_id?: number;
}

export type ApiResult<T> =
  | { ok: true; status: number; data: T }
  | { ok: false; status: number; error: ApiError };

function isApiError(v: unknown): v is ApiError {
  return (
    typeof v === "object" &&
    v !== null &&
    typeof (v as ApiError).code === "string" &&
    typeof (v as ApiError).message === "string"
  );
}

function networkError(e: unknown): ApiError {
  return {
    code: "network_error",
    message: e instanceof Error ? e.message : "Could not reach NetBot.",
    hint: null,
  };
}

/**
 * Call the NetBot API. Never throws: HTTP errors come back as `{ok:false}` with
 * the server's `{code, message, hint}` body, and a network failure comes back
 * as status 0 with code `network_error`. A body that fails to arrive after the
 * headers did also comes back as `network_error`, with the response's status.
 */
export async function apiFetch<T>(path: string, init: RequestInit = {}): Promise<ApiResult<T>> {
  const headers = new Headers(init.headers);
  if (init.body != null && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }

  let res: Response;
  try {
    res = await fetch(path, { ...init, headers, credentials: "same-origin" });
  } catch (e) {
    return { ok: false, status: 0, error: networkError(e) };
  }

  // Parse JSON only when there is a body (204 and some errors have none).
  let text: string;
  try {
    text = await res.text();
  } catch (e) {
    return { ok: false, status: res.status, error: networkError(e) };
  }
  let body: unknown;
  if (text !== "") {
    try {
      body = JSON.parse(text);
    } catch {
      body = undefined;
    }
  }

  if (res.ok) {
    return { ok: true, status: res.status, data: body as T };
  }
  const error: ApiError = isApiError(body)
    ? body
    : { code: `http_${res.status}`, message: `Request failed (${res.status}).`, hint: null };
  return { ok: false, status: res.status, error };
}
