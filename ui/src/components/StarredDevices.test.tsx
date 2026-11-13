import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { device, monitorStatus } from "../test/fixtures";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { StarredDevices } from "./StarredDevices";

describe("StarredDevices", () => {
  it("shows an empty state", async () => {
    mockFetch({
      "GET /api/devices": [200, [device()]],
      "GET /api/monitor": [200, monitorStatus()],
    });
    renderWithProviders(<StarredDevices />);
    expect(
      await screen.findByText("Star a device on the Devices screen to watch it closely."),
    ).toBeInTheDocument();
  });

  it("shows presence and check outcomes", async () => {
    const now = Date.now();
    mockFetch({
      "GET /api/devices": [
        200,
        [
          device({
            id: 7,
            display_name: "NAS",
            starred: true,
            online: true,
            presence_changed_at: now - 5 * 60_000,
          }),
          device({ id: 8, display_name: "Plain" }),
        ],
      ],
      "GET /api/monitor": [
        200,
        monitorStatus({
          checks: [
            {
              device_id: 7,
              outcomes: [
                { check: { kind: "tcp", port: 80 }, ok: true, at: now, detail: "open" },
                {
                  check: { kind: "http", port: 80, path: "/status", https: false },
                  ok: false,
                  at: now,
                  detail: "no answer",
                },
              ],
            },
          ],
        }),
      ],
    });
    renderWithProviders(<StarredDevices />);
    expect(await screen.findByText("NAS")).toBeInTheDocument();
    expect(screen.queryByText("Plain")).not.toBeInTheDocument();
    expect(screen.getByText("Online")).toBeInTheDocument();
    expect(screen.getByText(/online for 5 min/)).toBeInTheDocument();
    expect(await screen.findByText("Port 80: open")).toBeInTheDocument();
    expect(screen.getByText("HTTP /status: no answer")).toBeInTheDocument();
  });
});
