import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { alert } from "../test/fixtures";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { RecentAlerts } from "./RecentAlerts";

describe("RecentAlerts", () => {
  it("shows an empty state", async () => {
    mockFetch({ "GET /api/alerts": [200, []] });
    renderWithProviders(<RecentAlerts />);
    expect(await screen.findByText("No active alerts.")).toBeInTheDocument();
  });

  it("shows severity as text, the title, and a link to all alerts", async () => {
    mockFetch({
      "GET /api/alerts": [
        200,
        [
          alert({
            id: 1,
            severity: "warning",
            title: "Latency is high",
            opened_at: Date.now() - 120_000,
          }),
        ],
      ],
    });
    renderWithProviders(<RecentAlerts />);
    expect(await screen.findByText("Latency is high")).toBeInTheDocument();
    expect(screen.getByText("Warning")).toBeInTheDocument();
    expect(screen.getByText("2 min ago")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "See all alerts" })).toHaveAttribute("href", "/alerts");
  });

  it("shows only the 5 newest", async () => {
    const list = Array.from({ length: 7 }, (_, i) =>
      alert({ id: i + 1, title: `A${i + 1}`, opened_at: 1000 + i }),
    );
    mockFetch({ "GET /api/alerts": [200, list] });
    renderWithProviders(<RecentAlerts />);
    expect(await screen.findByText("A7")).toBeInTheDocument();
    expect(screen.queryByText("A2")).not.toBeInTheDocument();
    expect(screen.getByText("A3")).toBeInTheDocument();
  });
});
