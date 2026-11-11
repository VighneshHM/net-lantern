import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { RulesCard } from "./RulesCard";

const RULES = {
  starred_offline: true,
  internet_down: true,
  latency_high: true,
  packet_loss: true,
  new_device: true,
  notifications: true,
};

describe("RulesCard", () => {
  it("saves a rule change at once", async () => {
    const calls = mockFetch({
      "GET /api/rules": [200, RULES],
      "PUT /api/rules": [200, { ...RULES, latency_high: false }],
    });
    renderWithProviders(<RulesCard />);
    const box = await screen.findByRole("checkbox", { name: /Internet latency high/ });
    expect(screen.getAllByRole("checkbox")).toHaveLength(6);
    await userEvent.click(box);
    const put = calls.find((c) => c.method === "PUT");
    expect(put?.body).toEqual({ ...RULES, latency_high: false });
  });

  it("shows an error inline when saving fails", async () => {
    mockFetch({
      "GET /api/rules": [200, RULES],
      "PUT /api/rules": [500, { code: "internal", message: "Could not save.", hint: null }],
    });
    renderWithProviders(<RulesCard />);
    await userEvent.click(await screen.findByRole("checkbox", { name: /Packet loss/ }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save.");
  });

  it("shows the scan hint when the network is unknown", async () => {
    mockFetch({
      "GET /api/rules": [
        409,
        { code: "no_network", message: "No network yet.", hint: "Run a quick scan first." },
      ],
    });
    renderWithProviders(<RulesCard />);
    expect(await screen.findByText("Run a quick scan first.")).toBeInTheDocument();
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
  });
});
