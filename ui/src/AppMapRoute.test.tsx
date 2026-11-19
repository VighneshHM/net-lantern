import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppRoutes } from "./App";
import { mockFetch } from "./test/mockFetch";
import { renderWithProviders } from "./test/render";

// Kept apart from NetworkMap.test.tsx, which imports the page statically and so
// would have it cached before the lazy import runs.
describe("map route", () => {
  it("shows the loading text, then the lazily loaded page", async () => {
    mockFetch({ "GET /api/devices": [200, []] });
    renderWithProviders(<AppRoutes />, "/map");
    expect(screen.getByText("Loading the map…")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "Network map" })).toBeInTheDocument();
    expect(screen.queryByText("Loading the map…")).toBeNull();
  });
});
