import { describe, expect, it } from "vitest";
import { mockFetch } from "../test/mockFetch";
import { fetchDiagnosis, fetchLatestDiagnosis, startDiagnosis } from "./diagnose";
import { ApiRequestError } from "./queries";

const RUN = { id: 2, state: "done", started_at: 1, finished_at: 2, steps: [], verdict: null, error: null };

describe("startDiagnosis", () => {
  it("returns the run id even when already running", async () => {
    mockFetch({ "POST /api/diagnose": [202, { run_id: 4, already_running: true }] });
    expect(await startDiagnosis()).toBe(4);
  });

  it("throws an ApiRequestError on any other failure", async () => {
    mockFetch({ "POST /api/diagnose": [500, { code: "boom", message: "Broke.", hint: null }] });
    await expect(startDiagnosis()).rejects.toBeInstanceOf(ApiRequestError);
  });
});

describe("fetchDiagnosis and fetchLatestDiagnosis", () => {
  it("return the run, or null on 404", async () => {
    mockFetch({ "GET /api/diagnose/2": [200, RUN] });
    expect(await fetchDiagnosis(2)).toEqual(RUN);
    expect(await fetchDiagnosis(9)).toBeNull();
    expect(await fetchLatestDiagnosis()).toBeNull();
  });

  it("return null on any other failure", async () => {
    mockFetch({ "GET /api/diagnose/latest": [500, { code: "boom", message: "Broke.", hint: null }] });
    expect(await fetchLatestDiagnosis()).toBeNull();
  });
});
