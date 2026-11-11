import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { mockFetch } from "./test/mockFetch";
import { newClient, resetLive } from "./test/render";

/** When set, `ensureSession` rejects instead of running for real. */
const session = vi.hoisted(() => ({ fails: false }));
vi.mock("./api/session", async (importOriginal) => {
  const real = await importOriginal<typeof import("./api/session")>();
  return {
    ...real,
    ensureSession: (...args: Parameters<typeof real.ensureSession>) =>
      session.fails ? Promise.reject(new Error("history unavailable")) : real.ensureSession(...args),
  };
});

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
  session.fails = false;
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
    await waitFor(() => expect(FakeSocket.instances).toHaveLength(1));
  });

  it("falls through to the dashboard when the session check itself fails", async () => {
    mockFetch({ "GET /api/network": 200 });
    session.fails = true;
    renderApp();
    expect(await screen.findByRole("link", { name: "Dashboard" })).toBeInTheDocument();
    expect(screen.queryByText("Connecting…")).toBeNull();
  });

  it("shows the expired-link page when there is no session", async () => {
    mockFetch({ "GET /api/network": 401 });
    renderApp();
    expect(
      await screen.findByText(
        "This dashboard link has expired. Start Net Lantern again and it will open a fresh one.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
    expect(FakeSocket.instances).toHaveLength(0);
  });
});
