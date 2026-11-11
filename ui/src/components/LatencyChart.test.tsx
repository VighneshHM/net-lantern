import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders } from "../test/render";
import { LatencyChart } from "./LatencyChart";

const created = vi.hoisted(
  () => [] as { opts: { series: { label?: string }[] }; data: unknown[][] }[],
);
vi.mock("uplot", () => ({
  default: class {
    constructor(opts: { series: { label?: string }[] }, data: unknown[][]) {
      created.push({ opts, data });
    }
    setData() {}
    setSize() {}
    destroy() {}
  },
}));
vi.mock("uplot/dist/uPlot.min.css", () => ({}));

const point = (s: number, avg: number) => ({
  ts: s * 1000,
  min: avg,
  avg,
  max: avg,
  count: 1,
  lost: 0,
});

function stubMetrics() {
  const urls: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = new URL(String(input), "http://localhost");
      urls.push(url.pathname + url.search);
      const gateway = url.searchParams.get("target") === "gateway";
      const body = {
        resolution: "raw",
        points: [point(0, gateway ? 2 : 20), point(30, gateway ? 4 : 28)],
        sessions: [{ started_at: 0, ended_at: null, end_reason: null }],
      };
      return Promise.resolve(
        new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } }),
      );
    }),
  );
  return urls;
}

beforeEach(() => {
  created.length = 0;
});

describe("LatencyChart", () => {
  it("draws router and internet series and a text summary", async () => {
    stubMetrics();
    renderWithProviders(<LatencyChart />);
    expect(
      await screen.findByText("Last hour: router average 3 ms, internet average 24 ms."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Breaks in the lines are times Net Lantern wasn't monitoring."),
    ).toBeInTheDocument();
    await waitFor(() => expect(created.length).toBeGreaterThan(0));
    const labels = created[created.length - 1].opts.series.map((s) => s.label);
    expect(labels).toEqual(expect.arrayContaining(["Router", "Internet"]));
  });

  it("switches range", async () => {
    const urls = stubMetrics();
    renderWithProviders(<LatencyChart />);
    const h1 = await screen.findByRole("button", { name: "1h" });
    expect(h1).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(screen.getByRole("button", { name: "24h" }));
    expect(screen.getByRole("button", { name: "24h" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "1h" })).toHaveAttribute("aria-pressed", "false");
    await waitFor(() => expect(urls.some((u) => u.includes("range=24h"))).toBe(true));
    expect(await screen.findByText(/^Last 24 hours:/)).toBeInTheDocument();
  });

  it("says when there are no samples", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(
          new Response(JSON.stringify({ resolution: "raw", points: [], sessions: [] }), {
            headers: { "content-type": "application/json" },
          }),
        ),
      ),
    );
    renderWithProviders(<LatencyChart />);
    expect(await screen.findByText("No samples yet.")).toBeInTheDocument();
  });
});
