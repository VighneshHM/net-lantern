import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiagnosisRun, LayerName, LayerResult, LayerStatus } from "../api/diagnose";
import { useLive } from "../state/live";
import { mockFetch, type MockRoutes } from "../test/mockFetch";
import { renderWithProviders, resetLive } from "../test/render";
import { Diagnose } from "./Diagnose";

const notFound = { code: "not_found", message: "Not found.", hint: null };

function step(layer: LayerName, status: LayerStatus = "pass", over: Partial<LayerResult> = {}): LayerResult {
  return { layer, title: `T ${layer}`, status, finding: "f", evidence: [], explanation: "", fix: null, ...over };
}
function run(over: Partial<DiagnosisRun> = {}): DiagnosisRun {
  return { id: 1, state: "running", started_at: 1, finished_at: null, steps: [], verdict: null, error: null, ...over };
}
const stepEvent = (run_id: number, s: LayerResult) => ({
  seq: 1,
  api_version: 1,
  type: "diagnose.step",
  data: { run_id, step: s },
});
const items = () => {
  const ol = screen.getByRole("list", { name: "Diagnosis steps" });
  return within(ol)
    .getAllByRole("listitem")
    .filter((li) => li.parentElement === ol);
};

const cafe = (): DiagnosisRun =>
  run({
    state: "done",
    finished_at: Date.now(),
    steps: [step("link"), step("router"), step("internet", "fail")],
    verdict: {
      finding: "captive_portal",
      status: "fail",
      layer: "internet",
      headline: "Sign in to this network.",
      fix: "Open any website to see the sign-in page.",
    },
  });

function renderDiagnose(routes: MockRoutes = {}) {
  const calls = mockFetch({ "GET /api/diagnose/latest": [404, notFound], ...routes });
  renderWithProviders(<Diagnose />);
  return calls;
}

beforeEach(() => {
  resetLive();
});

describe("Diagnose", () => {
  it("runs a diagnosis and streams steps into the trace", async () => {
    const user = userEvent.setup();
    renderDiagnose({
      "POST /api/diagnose": [202, { run_id: 1, already_running: false }],
      "GET /api/diagnose/1": [200, run({ id: 1, state: "running", steps: [] })],
    });
    await user.click(await screen.findByRole("button", { name: "Run diagnosis" }));
    expect(await screen.findByRole("button", { name: "Diagnosing…" })).toBeDisabled();
    act(() =>
      useLive.getState().applyEvent(
        stepEvent(1, step("link", "pass", { explanation: "Your laptop is connected and the signal is fine." })),
      ),
    );
    expect(items()[0]).toHaveTextContent("Passed");
    expect(items()[0]).toHaveTextContent("Your laptop is connected and the signal is fine.");
    expect(items()[1]).toHaveTextContent("Checking…");
    expect(items()[2]).toHaveTextContent("Waiting");
  });

  it("shows the verdict with its fix and skipped layers as Not checked", async () => {
    renderDiagnose();
    await screen.findByRole("button", { name: "Run diagnosis" });
    act(() => useLive.getState().followDiagnosis(cafe()));
    const verdict = screen.getByRole("region", { name: "Verdict" });
    expect(within(verdict).getByRole("heading", { name: "Sign in to this network." })).toBeInTheDocument();
    expect(verdict).toHaveTextContent("Problem found");
    expect(verdict).toHaveTextContent("Try this: Open any website to see the sign-in page.");
    expect(verdict).toHaveTextContent("Checked just now");
    expect(items()[3]).toHaveTextContent("Not checked");
    expect(items()[4]).toHaveTextContent("Not checked");
  });

  it("says what to do before any run", async () => {
    renderDiagnose();
    expect(
      await screen.findByText("Run a diagnosis to check each step from your Wi-Fi to a website."),
    ).toBeInTheDocument();
  });

  it("loads the latest run on open", async () => {
    const done = cafe();
    done.finished_at = Date.now() - 5 * 60_000;
    renderDiagnose({ "GET /api/diagnose/latest": [200, done] });
    expect(await screen.findByRole("heading", { name: "Sign in to this network." })).toBeInTheDocument();
    expect(screen.getByText("Checked 5 min ago")).toBeInTheDocument();
  });

  it("shows the server's error when starting fails", async () => {
    const user = userEvent.setup();
    renderDiagnose({
      "POST /api/diagnose": [500, { code: "internal", message: "Could not start the diagnosis.", hint: null }],
    });
    await user.click(await screen.findByRole("button", { name: "Run diagnosis" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not start the diagnosis.");
  });

  it("a failed run says why it stopped", async () => {
    renderDiagnose();
    await screen.findByRole("button", { name: "Run diagnosis" });
    act(() =>
      useLive.getState().followDiagnosis(run({ state: "failed", error: "Internal error during the diagnosis." })),
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "The diagnosis stopped: Internal error during the diagnosis.",
    );
  });

  it("ends done when the finish event arrives while the snapshot is loading", async () => {
    const user = userEvent.setup();
    let release: (r: Response) => void = () => {};
    const jsonRes = (status: number, body: unknown) =>
      new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    vi.stubGlobal(
      "fetch",
      vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
        const path = new URL(String(input), "http://localhost").pathname;
        const method = (init?.method ?? "GET").toUpperCase();
        if (method === "POST") return Promise.resolve(jsonRes(202, { run_id: 1, already_running: false }));
        if (path === "/api/diagnose/1") return new Promise<Response>((r) => (release = r));
        return Promise.resolve(jsonRes(404, notFound));
      }),
    );
    renderWithProviders(<Diagnose />);
    await user.click(await screen.findByRole("button", { name: "Run diagnosis" }));
    await vi.waitFor(() => expect(useLive.getState().diagnosis?.id).toBe(1));
    act(() => {
      useLive.getState().applyEvent({
        seq: 1,
        api_version: 1,
        type: "diagnose.done",
        data: { run_id: 1, state: "done", verdict: cafe().verdict, error: null },
      });
    });
    await act(async () => {
      release(jsonRes(200, run({ id: 1, state: "running", steps: [] })));
    });
    expect(useLive.getState().diagnosis!.state).toBe("done");
    expect(await screen.findByRole("button", { name: "Run diagnosis" })).toBeEnabled();
    expect(screen.getByRole("heading", { name: "Sign in to this network." })).toBeInTheDocument();
  });

  it("keeps following live steps when the snapshot call fails", async () => {
    const user = userEvent.setup();
    renderDiagnose({
      "POST /api/diagnose": [202, { run_id: 1, already_running: false }],
      "GET /api/diagnose/1": [500, { code: "internal", message: "x", hint: null }],
    });
    await user.click(await screen.findByRole("button", { name: "Run diagnosis" }));
    expect(await screen.findByText("Diagnosing…")).toBeDisabled();
    act(() => useLive.getState().applyEvent(stepEvent(1, step("link"))));
    expect(items()[0]).toHaveTextContent("Passed");
    expect(items()[1]).toHaveTextContent("Checking…");
  });

  it("does not let the latest run on open replace a newer live run", async () => {
    useLive.getState().followDiagnosis(run({ id: 5, steps: [step("link")] }));
    renderDiagnose({ "GET /api/diagnose/latest": [200, cafe()] });
    expect(await screen.findByText("Diagnosing…")).toBeDisabled();
    await vi.waitFor(() => expect(useLive.getState().diagnosis?.id).toBe(5));
    expect(useLive.getState().diagnosis!.steps).toHaveLength(1);
  });

  it("does not let the latest run on open undo a finish already seen live", async () => {
    const live = cafe();
    useLive.getState().followDiagnosis(live);
    renderDiagnose({ "GET /api/diagnose/latest": [200, run({ id: 1, state: "running" })] });
    await screen.findByRole("button", { name: "Run diagnosis" });
    expect(useLive.getState().diagnosis!.state).toBe("done");
  });
});
