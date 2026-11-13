import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
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

  it("explains that the internet switch also covers the router and its suppression", async () => {
    mockFetch({ "GET /api/rules": [200, RULES] });
    renderWithProviders(<RulesCard />);
    expect(await screen.findByRole("checkbox", { name: /Internet down/ })).toBeInTheDocument();
    expect(
      screen.getByText(
        "router or internet unreachable for 2 checks; with it off, a router outage no longer holds back the alerts of starred devices",
      ),
    ).toBeInTheDocument();
  });

  it("applies both of two quick toggles", async () => {
    const puts: unknown[] = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
        const reply = (body: unknown) =>
          new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } });
        if ((init?.method ?? "GET") === "GET") return Promise.resolve(reply(RULES));
        const body: unknown = JSON.parse(String(init?.body));
        puts.push(body);
        // The server is slow: the second click lands while this is in flight.
        return new Promise<Response>((r) => setTimeout(() => r(reply(body)), 100));
      }),
    );
    renderWithProviders(<RulesCard />);
    const latency = await screen.findByRole("checkbox", { name: /Internet latency high/ });
    const loss = screen.getByRole("checkbox", { name: /Packet loss/ });
    await userEvent.click(latency);
    await userEvent.click(loss);
    await waitFor(() =>
      expect(puts.at(-1)).toEqual({ ...RULES, latency_high: false, packet_loss: false }),
    );
    await new Promise((r) => setTimeout(r, 300));
    expect(puts.at(-1)).toEqual({ ...RULES, latency_high: false, packet_loss: false });
    expect(screen.getByRole("checkbox", { name: /Internet latency high/ })).not.toBeChecked();
    expect(screen.getByRole("checkbox", { name: /Packet loss/ })).not.toBeChecked();
  });
});
