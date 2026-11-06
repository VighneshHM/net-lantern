import type { ReactElement } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { useLive } from "../state/live";

/** A fresh `QueryClient` per test, with retries off so failures show up at once. */
export function newClient(): QueryClient {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}

/** Render under a fresh QueryClient and a MemoryRouter. */
export function renderWithProviders(ui: ReactElement, path = "/") {
  const client = newClient();
  return {
    client,
    ...render(
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={[path]}>{ui}</MemoryRouter>
      </QueryClientProvider>,
    ),
  };
}

/** Put the live store back to its initial state (call in `beforeEach`). */
export function resetLive() {
  useLive.setState({ scan: null, outdated: false });
}
