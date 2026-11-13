import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { alert } from "../test/fixtures";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { NotificationPreview } from "./NotificationPreview";

describe("NotificationPreview", () => {
  it("previews the newest active alert, else a sample", async () => {
    mockFetch({
      "GET /api/alerts": [
        200,
        [
          alert({ id: 1, title: "Old one", detail: "old detail", opened_at: 1_000 }),
          alert({ id: 2, title: "New one", detail: "new detail", opened_at: 2_000 }),
        ],
      ],
    });
    renderWithProviders(<NotificationPreview />);
    expect(await screen.findByText("New one")).toBeInTheDocument();
    expect(screen.getByText("new detail")).toBeInTheDocument();
    expect(screen.queryByText("Old one")).toBeNull();
    expect(screen.getByText("Net Lantern")).toBeInTheDocument();
    expect(screen.getByText("What a desktop notification looks like")).toBeInTheDocument();
    expect(
      screen.getByText(
        "If notifications don't appear, check Windows Settings → System → Notifications.",
      ),
    ).toBeInTheDocument();
  });

  it("shows the sample when there are none", async () => {
    mockFetch({ "GET /api/alerts": [200, []] });
    renderWithProviders(<NotificationPreview />);
    expect(await screen.findByText("Office printer is offline")).toBeInTheDocument();
    expect(screen.getByText("Not seen for 2 min.")).toBeInTheDocument();
  });
});
