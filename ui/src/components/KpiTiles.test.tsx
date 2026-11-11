import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { alert, device, monitorStatus, tick } from "../test/fixtures";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { KpiTiles } from "./KpiTiles";

describe("KpiTiles", () => {
  it("shows text values and dashes when empty", async () => {
    const devices = Array.from({ length: 13 }, (_, i) => device({ id: i + 1, online: i < 9 }));
    mockFetch({
      "GET /api/monitor": [
        200,
        monitorStatus({ last: tick({ gateway_rtt_ms: 3, internet_rtt_ms: null }) }),
      ],
      "GET /api/devices": [200, devices],
      "GET /api/alerts": [200, [alert({ id: 1 }), alert({ id: 2 })]],
    });
    const { unmount } = renderWithProviders(<KpiTiles />);
    expect(await screen.findByText("9 of 13")).toBeInTheDocument();
    expect(await screen.findByText("3 ms")).toBeInTheDocument();
    expect(screen.getByText("No reply")).toBeInTheDocument();
    expect(screen.getByText("0 of 30")).toBeInTheDocument();
    expect(await screen.findByText("2")).toBeInTheDocument();
    expect(screen.getByText("Devices online")).toBeInTheDocument();
    expect(screen.getByText("Router loss (15 min)")).toBeInTheDocument();
    unmount();

    mockFetch({
      "GET /api/monitor": [200, monitorStatus({ state: "not_connected", last: null })],
    });
    renderWithProviders(<KpiTiles />);
    expect((await screen.findAllByText("—")).length).toBeGreaterThanOrEqual(3);
  });
});
