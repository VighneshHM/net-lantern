import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { AppRoutes } from "../App";

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <AppRoutes />
    </MemoryRouter>,
  );
}

describe("Shell", () => {
  it("shows six nav links and marks the active one", () => {
    renderAt("/devices");
    expect(screen.getAllByRole("link")).toHaveLength(6);
    expect(screen.getByRole("link", { name: "Devices" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Dashboard" })).not.toHaveAttribute("aria-current");
  });

  it("renders the page for the current route", () => {
    renderAt("/devices");
    expect(screen.getByRole("heading", { level: 1, name: "Devices" })).toBeInTheDocument();
  });

  it("shows the phase for placeholder pages", () => {
    renderAt("/map");
    expect(screen.getByRole("heading", { level: 1, name: "Map" })).toBeInTheDocument();
    expect(screen.getByText("Arrives in phase 5")).toBeInTheDocument();
  });

  it("does not make the brand a link", () => {
    renderAt("/");
    expect(screen.getByText("NetBot")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Dashboard" })).toHaveAttribute("aria-current", "page");
  });
});
