import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { UpdateBanner, UpdateQuestion } from "./UpdateCard";

function update(over: Record<string, unknown> = {}) {
  return {
    setting: true,
    current: "0.7.0",
    latest: null,
    newer: false,
    url: null,
    checked_at: null,
    ...over,
  };
}

const NEWER = {
  setting: true,
  latest: "0.8.0",
  newer: true,
  url: "https://github.com/VighneshHM/net-lantern/releases/tag/v0.8.0",
};

function renderBoth() {
  renderWithProviders(
    <>
      <UpdateBanner />
      <UpdateQuestion />
    </>,
  );
}

beforeEach(() => {
  try {
    localStorage.clear();
  } catch {
    // no storage here
  }
});

describe("UpdateQuestion", () => {
  it("question_shows_when_unanswered_and_sets_yes_or_no", async () => {
    const user = userEvent.setup();
    const calls = mockFetch({
      "GET /api/update": [200, update({ setting: null })],
      "PUT /api/update": [200, update({ setting: true })],
    });
    renderBoth();
    expect(await screen.findByRole("heading", { name: "Check for new versions?" })).toBeInTheDocument();
    expect(
      screen.getByText(
        "Net Lantern can ask GitHub once a day whether a newer version is out. It sends nothing about your network.",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Yes, check for updates" }));
    expect(calls.find((c) => c.method === "PUT")?.body).toEqual({ enabled: true });
    await waitFor(() => expect(screen.queryByText("Check for new versions?")).toBeNull());
  });

  it("answering no sends enabled false and hides the card", async () => {
    const user = userEvent.setup();
    const calls = mockFetch({
      "GET /api/update": [200, update({ setting: null })],
      "PUT /api/update": [200, update({ setting: false })],
    });
    renderBoth();
    await user.click(await screen.findByRole("button", { name: "No thanks" }));
    expect(calls.find((c) => c.method === "PUT")?.body).toEqual({ enabled: false });
    await waitFor(() => expect(screen.queryByText("Check for new versions?")).toBeNull());
  });

  it("neither_answer_is_styled_as_the_default", async () => {
    mockFetch({ "GET /api/update": [200, update({ setting: null })] });
    renderBoth();
    const yes = await screen.findByRole("button", { name: "Yes, check for updates" });
    const no = screen.getByRole("button", { name: "No thanks" });
    expect(yes.className).toBe(no.className);
    expect(yes.className).not.toContain("bg-accent");
  });

  it("a failed answer shows an alert and keeps the card", async () => {
    const user = userEvent.setup();
    mockFetch({
      "GET /api/update": [200, update({ setting: null })],
      "PUT /api/update": [500, { code: "internal", message: "Couldn't save that.", hint: null }],
    });
    renderBoth();
    await user.click(await screen.findByRole("button", { name: "No thanks" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't save that.");
    expect(screen.getByRole("heading", { name: "Check for new versions?" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "No thanks" })).toBeEnabled();
  });

  it("is not shown once answered", async () => {
    mockFetch({ "GET /api/update": [200, update({ setting: false })] });
    renderBoth();
    await waitFor(() => expect(screen.queryByText("Check for new versions?")).toBeNull());
  });
});

describe("UpdateBanner", () => {
  it("banner_shows_for_a_newer_version_and_links_to_github_only", async () => {
    mockFetch({ "GET /api/update": [200, update(NEWER)] });
    renderBoth();
    expect(await screen.findByText("Version 0.8.0 is available.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Download from GitHub" })).toHaveAttribute("href", NEWER.url);
  });

  it.each([
    null,
    "https://evil.example/x",
    "https://github.com.evil.com/x",
    "http://github.com/x",
    "https://github.com@evil.com/x",
    "https://github.com",
    "https://github.com/other/x",
    "https://github.com/VighneshHM/net-lantern/issues",
    "https://github.com/VighneshHM/net-lantern-evil/releases/x",
  ])("banner_shows_no_link_for_%s", async (url) => {
    mockFetch({ "GET /api/update": [200, update({ ...NEWER, url })] });
    renderBoth();
    await screen.findByText("Version 0.8.0 is available.");
    expect(screen.queryByRole("link")).toBeNull();
  });

  it("the github link opens in a new tab without a referrer", async () => {
    mockFetch({
      "GET /api/update": [
        200,
        update({ ...NEWER, url: "https://github.com/VighneshHM/net-lantern/releases/tag/v0.2.0" }),
      ],
    });
    renderBoth();
    const link = await screen.findByRole("link", { name: "Download from GitHub" });
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noreferrer");
  });

  it("dismissing moves focus to the page heading", async () => {
    const user = userEvent.setup();
    mockFetch({ "GET /api/update": [200, update(NEWER)] });
    renderWithProviders(
      <>
        <h1 tabIndex={-1}>Dashboard</h1>
        <UpdateBanner />
      </>,
    );
    await user.click(await screen.findByRole("button", { name: "Dismiss" }));
    expect(screen.getByRole("heading", { name: "Dashboard" })).toHaveFocus();
  });

  it("banner_dismissal_is_remembered_for_that_version_only", async () => {
    const user = userEvent.setup();
    mockFetch({ "GET /api/update": [200, update(NEWER)] });
    const first = renderWithProviders(<UpdateBanner />);
    await user.click(await screen.findByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Version 0.8.0 is available.")).toBeNull();
    expect(localStorage.getItem("nl_dismissed_update")).toBe("0.8.0");
    first.unmount();

    renderWithProviders(<UpdateBanner />);
    await waitFor(() => expect(globalThis.fetch).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText("Version 0.8.0 is available.")).toBeNull();
  });

  it("a newer version shows again after an older one was dismissed", async () => {
    localStorage.setItem("nl_dismissed_update", "0.8.0");
    mockFetch({ "GET /api/update": [200, update({ ...NEWER, latest: "0.9.0" })] });
    renderWithProviders(<UpdateBanner />);
    expect(await screen.findByText("Version 0.9.0 is available.")).toBeInTheDocument();
  });

  it("works when storage throws", async () => {
    const user = userEvent.setup();
    const orig = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error("blocked");
    };
    const origSet = Storage.prototype.setItem;
    Storage.prototype.setItem = () => {
      throw new Error("blocked");
    };
    try {
      mockFetch({ "GET /api/update": [200, update(NEWER)] });
      renderWithProviders(<UpdateBanner />);
      await user.click(await screen.findByRole("button", { name: "Dismiss" }));
      expect(screen.queryByText("Version 0.8.0 is available.")).toBeNull();
    } finally {
      Storage.prototype.getItem = orig;
      Storage.prototype.setItem = origSet;
    }
  });

  it("nothing_shows_while_loading_or_on_error", async () => {
    mockFetch({ "GET /api/update": [500, { code: "internal", message: "Boom.", hint: null }] });
    renderBoth();
    expect(screen.queryByText("Check for new versions?")).toBeNull();
    expect(screen.queryByText(/is available/)).toBeNull();
    await waitFor(() => expect(globalThis.fetch).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText("Check for new versions?")).toBeNull();
    expect(screen.queryByText(/is available/)).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });
});
