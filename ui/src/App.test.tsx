import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { mockFetch } from "./test/mockFetch";
import { newClient, resetLive } from "./test/render";

class FakeSocket {
  static instances: FakeSocket[] = [];
  onmessage: unknown = null;
  onclose: unknown = null;
  constructor() {
    FakeSocket.instances.push(this);
  }
  close() {}
}

function renderApp() {
  return render(
    <QueryClientProvider client={newClient()}>
      <App />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  FakeSocket.instances = [];
  resetLive();
  vi.stubGlobal("WebSocket", FakeSocket);
});

describe("App", () => {
  it("shows Connecting… and then the shell when the session is valid", async () => {
    mockFetch({ "GET /api/network": 200 });
    renderApp();
    expect(screen.getByText("Connecting…")).toBeInTheDocument();
    expect(await screen.findByRole("link", { name: "Dashboard" })).toBeInTheDocument();
  });

  it("opens the live event socket once, only after the session is valid", async () => {
    mockFetch({ "GET /api/network": 200 });
    renderApp();
    expect(FakeSocket.instances).toHaveLength(0);
    await screen.findByRole("link", { name: "Dashboard" });
    expect(FakeSocket.instances).toHaveLength(1);
  });

  it("shows the expired-link page when there is no session", async () => {
    mockFetch({ "GET /api/network": 401 });
    renderApp();
    expect(
      await screen.findByText(
        "This dashboard link has expired. Start NetBot again and it will open a fresh one.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
    expect(FakeSocket.instances).toHaveLength(0);
  });
});
