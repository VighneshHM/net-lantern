import { describe, expect, it, vi } from "vitest";
import { ensureSession } from "./session";
import { apiFetch } from "./client";
import { fakeHistory, loc } from "../test/helpers";
import { mockFetch } from "../test/mockFetch";

describe("ensureSession", () => {
  it("redeems the token and cleans the URL", async () => {
    const calls = mockFetch({ "POST /api/session": 204, "GET /api/network": 200 });
    const hist = fakeHistory();
    expect(await ensureSession(loc("/?t=abc"), hist)).toBe("ok");
    expect(hist.replaced).toBe("/");
    expect(calls[0]).toEqual({ method: "POST", path: "/api/session", body: { token: "abc" } });
    expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual([
      "POST /api/session",
      "GET /api/network",
    ]);
  });

  it("keeps other query parameters and the hash when removing the token", async () => {
    mockFetch({ "POST /api/session": 204, "GET /api/network": 200 });
    const hist = fakeHistory();
    await ensureSession(loc("/devices?a=1&t=abc&b=2#row"), hist);
    expect(hist.replaced).toBe("/devices?a=1&b=2#row");
  });

  it("still works when the token was already used but the cookie is valid", async () => {
    mockFetch({ "POST /api/session": 401, "GET /api/network": 200 });
    const hist = fakeHistory();
    expect(await ensureSession(loc("/devices?t=old"), hist)).toBe("ok");
    expect(hist.replaced).toBe("/devices");
  });

  it("asks for a fresh launch when there is no session", async () => {
    const hist = fakeHistory();
    mockFetch({ "GET /api/network": 401 });
    expect(await ensureSession(loc("/"), hist)).toBe("needs-launch");
    expect(hist.replaced).toBeUndefined();
  });

  it("asks for a fresh launch when the token is bad and there is no cookie", async () => {
    mockFetch({ "POST /api/session": 401, "GET /api/network": 401 });
    expect(await ensureSession(loc("/?t=bad"), fakeHistory())).toBe("needs-launch");
  });

  it("treats not_connected as a valid session", async () => {
    mockFetch({
      "GET /api/network": [503, { code: "not_connected", message: "Not connected.", hint: null }],
    });
    expect(await ensureSession(loc("/"), fakeHistory())).toBe("ok");
  });

  it("lets the dashboard show its own error on server errors", async () => {
    mockFetch({ "GET /api/network": 500 });
    expect(await ensureSession(loc("/"), fakeHistory())).toBe("ok");
  });

  it("lets the dashboard show its own error when the server is unreachable", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));
    expect(await ensureSession(loc("/"), fakeHistory())).toBe("ok");
  });
});

describe("apiFetch", () => {
  it("returns parsed data on success and sends same-origin credentials", async () => {
    mockFetch({ "GET /api/network": [200, { ip: "192.168.1.23" }] });
    const result = await apiFetch<{ ip: string }>("/api/network");
    expect(result).toEqual({ ok: true, status: 200, data: { ip: "192.168.1.23" } });
    const init = vi.mocked(fetch).mock.calls[0][1];
    expect(init?.credentials).toBe("same-origin");
  });

  it("returns the error body without throwing on HTTP errors", async () => {
    const error = { code: "scan_in_progress", message: "Busy.", hint: null, scan_id: 7 };
    mockFetch({ "POST /api/scans": [409, error] });
    const result = await apiFetch("/api/scans", { method: "POST", body: JSON.stringify({}) });
    expect(result).toEqual({ ok: false, status: 409, error });
  });

  it("sends a JSON content-type when a body is given", async () => {
    mockFetch({ "POST /api/session": 204 });
    const result = await apiFetch("/api/session", {
      method: "POST",
      body: JSON.stringify({ token: "x" }),
    });
    expect(result.ok).toBe(true);
    const init = vi.mocked(fetch).mock.calls[0][1];
    expect(new Headers(init?.headers).get("content-type")).toBe("application/json");
  });

  it("reports a network failure as status 0", async () => {
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("Failed to fetch")));
    const result = await apiFetch("/api/network");
    expect(result).toMatchObject({
      ok: false,
      status: 0,
      error: { code: "network_error", hint: null },
    });
  });

  it("reports a body that fails to arrive as a network error with the response status", async () => {
    // The connection drops after the headers: `res.text()` rejects.
    const res = {
      ok: true,
      status: 200,
      text: () => Promise.reject(new TypeError("network error while reading body")),
    } as unknown as Response;
    vi.stubGlobal("fetch", () => Promise.resolve(res));
    const result = await apiFetch("/api/network");
    expect(result).toEqual({
      ok: false,
      status: 200,
      error: {
        code: "network_error",
        message: "network error while reading body",
        hint: null,
      },
    });
  });
});
