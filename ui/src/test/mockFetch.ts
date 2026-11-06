import { vi } from "vitest";

/** One recorded `fetch` call. `path` has no query string. */
export interface FetchCall {
  method: string;
  path: string;
  body: unknown;
}

/**
 * A route is either a status code or `[status, jsonBody]`.
 *
 * A bare status code returns that status with no body, except 200, which
 * returns the minimal JSON body `{}` (so `apiFetch` sees a successful JSON
 * response). 204 and every other bare code carry no body at all.
 */
export type MockRoutes = Record<string, number | [number, unknown]>;

const NOT_FOUND = { code: "not_found", message: "Not found.", hint: null };

/**
 * Stub `globalThis.fetch` (via `vi.stubGlobal`; undone after each test by
 * `src/test/setup.ts`). Routes are keyed `"METHOD /path"`; the path excludes
 * the query string. Unmatched requests get a 404 with a `not_found` body.
 * Returns the list of calls made, in order.
 */
export function mockFetch(routes: MockRoutes): FetchCall[] {
  const calls: FetchCall[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), "http://localhost");
      const method = (init?.method ?? "GET").toUpperCase();
      const raw = init?.body;
      calls.push({
        method,
        path: url.pathname,
        body: typeof raw === "string" ? JSON.parse(raw) : undefined,
      });
      const route = routes[`${method} ${url.pathname}`];
      if (route === undefined) return Promise.resolve(json(404, NOT_FOUND));
      const [status, body] = typeof route === "number" ? [route, undefined] : route;
      if (body !== undefined) return Promise.resolve(json(status, body));
      if (status === 200) return Promise.resolve(json(200, {}));
      return Promise.resolve(new Response(null, { status }));
    }),
  );
  return calls;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}
