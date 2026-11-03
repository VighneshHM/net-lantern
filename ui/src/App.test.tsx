import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";
import { mockFetch } from "./test/mockFetch";

describe("App", () => {
  it("shows Connecting… and then the shell when the session is valid", async () => {
    mockFetch({ "GET /api/network": 200 });
    render(<App />);
    expect(screen.getByText("Connecting…")).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "Dashboard" })).toBeInTheDocument();
  });

  it("shows the expired-link page when there is no session", async () => {
    mockFetch({ "GET /api/network": 401 });
    render(<App />);
    expect(
      await screen.findByText(
        "This dashboard link has expired. Start NetBot again and it will open a fresh one.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
  });
});
