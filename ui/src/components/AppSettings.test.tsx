import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { HostInfo, UpdateStatus } from "../api/ops";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { DeleteDataCard, HelpCard, StartWithWindowsCard, UpdatesCard } from "./AppSettings";

function host(over: Partial<HostInfo["autostart"]> = {}, kind: HostInfo["kind"] = "desktop"): HostInfo {
  return { kind, version: "0.7.0", autostart: { available: true, enabled: false, ...over } };
}

function update(over: Partial<UpdateStatus> = {}): UpdateStatus {
  return { setting: false, current: "0.7.0", latest: null, newer: false, url: null, checked_at: null, ...over };
}

function clipboard(writeText: () => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("StartWithWindowsCard", () => {
  it("start_with_windows_card_only_on_desktop_and_toggles", async () => {
    const calls = mockFetch({
      "GET /api/host": [200, host()],
      "PUT /api/host/autostart": [200, host({ enabled: true })],
    });
    renderWithProviders(<StartWithWindowsCard />);
    const box = await screen.findByRole("checkbox", {
      name: "Start Net Lantern when you sign in to Windows",
    });
    expect(box).not.toBeChecked();
    expect(screen.getByText("It starts in the tray, without opening a window.")).toBeInTheDocument();
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeChecked());
    expect(calls.filter((c) => c.method === "PUT")).toEqual([
      { method: "PUT", path: "/api/host/autostart", body: { enabled: true } },
    ]);
  });

  it("is_absent_without_a_switch", async () => {
    const calls = mockFetch({ "GET /api/host": [200, host({ available: false, enabled: null }, "exe")] });
    const { container } = renderWithProviders(<StartWithWindowsCard />);
    await waitFor(() => expect(calls).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 20));
    expect(container).toBeEmptyDOMElement();
  });

  it("says_it_can_only_be_changed_on_the_laptop_from_a_phone", async () => {
    mockFetch({
      "GET /api/host": [200, host()],
      "PUT /api/host/autostart": [
        403,
        { code: "phone_not_allowed", message: "Not from a phone.", hint: null },
      ],
    });
    renderWithProviders(<StartWithWindowsCard />);
    await userEvent.click(await screen.findByRole("checkbox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("This can only be changed on the laptop.");
  });

  it("shows_the_server_message_on_failure", async () => {
    mockFetch({
      "GET /api/host": [200, host()],
      "PUT /api/host/autostart": [
        500,
        { code: "autostart_failed", message: "Could not change it.", hint: null },
      ],
    });
    renderWithProviders(<StartWithWindowsCard />);
    await userEvent.click(await screen.findByRole("checkbox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not change it.");
  });
});

describe("StartWithWindowsCard unavailable", () => {
  it("shows_the_server_message_on_409_autostart_unavailable", async () => {
    mockFetch({
      "GET /api/host": [200, host()],
      "PUT /api/host/autostart": [
        409,
        { code: "autostart_unavailable", message: "Start at login isn't available here.", hint: null },
      ],
    });
    renderWithProviders(<StartWithWindowsCard />);
    await userEvent.click(await screen.findByRole("checkbox"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Start at login isn't available here.");
  });
});

describe("UpdatesCard", () => {
  it("a_failed_update_change_shows_an_alert", async () => {
    mockFetch({
      "GET /api/update": [200, update({ setting: false })],
      "PUT /api/update": [500, { code: "internal", message: "Couldn't save that.", hint: null }],
    });
    renderWithProviders(<UpdatesCard />);
    const box = await screen.findByRole("checkbox");
    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent("Couldn't save that.");
    expect(box).not.toBeChecked();
  });

  it("update_checkbox_reflects_and_sets_the_setting", async () => {
    const calls = mockFetch({
      "GET /api/update": [200, update({ setting: null })],
      "PUT /api/update": [200, update({ setting: true })],
    });
    renderWithProviders(<UpdatesCard />);
    const box = await screen.findByRole("checkbox", { name: "Check GitHub for new versions" });
    expect(box).not.toBeChecked();
    expect(
      screen.getByText(
        "Net Lantern asks GitHub for the latest version number once a day. Nothing about your network is sent.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("You have version 0.7.0.")).toBeInTheDocument();
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeChecked());
    expect(calls.filter((c) => c.method === "PUT")).toEqual([
      { method: "PUT", path: "/api/update", body: { enabled: true } },
    ]);
  });

  it("update_card_shows_newer_version_link_only_for_github", async () => {
    mockFetch({
      "GET /api/update": [
        200,
        update({ setting: true, latest: "0.8.0", newer: true, url: "https://github.com/VighneshHM/net-lantern/releases/tag/v0.8.0" }),
      ],
    });
    const first = renderWithProviders(<UpdatesCard />);
    expect(await screen.findByText(/Version 0\.8\.0 is available\./)).toBeInTheDocument();
    const link = screen.getByRole("link", { name: "See what's new" });
    expect(link).toHaveAttribute("href", "https://github.com/VighneshHM/net-lantern/releases/tag/v0.8.0");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noreferrer");
    first.unmount();

    mockFetch({
      "GET /api/update": [
        200,
        update({ setting: true, latest: "0.8.0", newer: true, url: "https://evil.example/releases" }),
      ],
    });
    renderWithProviders(<UpdatesCard />);
    expect(await screen.findByText(/Version 0\.8\.0 is available\./)).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "See what's new" })).not.toBeInTheDocument();
  });
});

describe("HelpCard", () => {
  it("a_failed_debug_info_fetch_shows_an_alert", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response("{}", { status: 500 }))));
    renderWithProviders(<HelpCard />);
    await userEvent.click(screen.getByRole("button", { name: "Copy debug info" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Net Lantern couldn't gather the debug info.",
    );
  });

  it("copy_debug_info_copies_the_server_text", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    clipboard(writeText);
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(
          new Response("Net Lantern 0.7.0\nlog line", {
            status: 200,
            headers: { "content-type": "text/plain" },
          }),
        ),
      ),
    );
    renderWithProviders(<HelpCard />);
    expect(screen.getByText(/Copies your Net Lantern version, Windows version/)).toHaveTextContent(
      "Copies your Net Lantern version, Windows version, network state and recent log lines, with addresses and names removed. Nothing is sent anywhere.",
    );
    await userEvent.click(screen.getByRole("button", { name: "Copy debug info" }));
    expect(await screen.findByRole("status")).toHaveTextContent("Copied — paste it into your bug report.");
    expect(writeText).toHaveBeenCalledWith("Net Lantern 0.7.0\nlog line");
  });

  it("copy_falls_back_to_a_textarea_when_the_clipboard_fails", async () => {
    clipboard(vi.fn().mockRejectedValue(new Error("denied")));
    vi.stubGlobal("fetch", vi.fn(() => Promise.resolve(new Response("the debug text", { status: 200 }))));
    renderWithProviders(<HelpCard />);
    await userEvent.click(screen.getByRole("button", { name: "Copy debug info" }));
    const area = await screen.findByRole("textbox", { name: "Debug info" });
    expect(area).toHaveValue("the debug text");
    expect(area).toHaveAttribute("readonly");
    expect(screen.getByText("Select the text and copy it.")).toBeInTheDocument();
  });
});

describe("DeleteDataCard", () => {
  const NOTE =
    "Removes every network, device, name, history and setting Net Lantern has stored on this computer. Log files are kept.";

  it("delete_all_data_needs_the_confirm_dialog_and_posts_the_exact_confirm_text", async () => {
    const calls = mockFetch({ "POST /api/data/reset": 204 });
    const { client } = renderWithProviders(<DeleteDataCard />);
    const invalidate = vi.spyOn(client, "invalidateQueries");
    expect(screen.getByText(NOTE)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Delete all data…" }));
    expect(screen.getByRole("alertdialog")).toHaveTextContent(
      "Delete all Net Lantern data? This can't be undone.",
    );
    expect(calls).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Delete all data…" })).toHaveFocus();
    expect(calls).toHaveLength(0);

    await userEvent.click(screen.getByRole("button", { name: "Delete all data…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete all data" }));
    expect(await screen.findByRole("status")).toHaveTextContent("All data deleted.");
    expect(screen.getByRole("button", { name: "Delete all data…" })).toHaveFocus();
    expect(calls).toEqual([
      { method: "POST", path: "/api/data/reset", body: { confirm: "delete all data" } },
    ]);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(invalidate).toHaveBeenCalled();
  });

  it("delete_failure_shows_an_alert_inside_the_dialog", async () => {
    mockFetch({
      "POST /api/data/reset": [
        500,
        { code: "reset_failed", message: "Net Lantern couldn't delete its data.", hint: "Nothing was deleted." },
      ],
    });
    renderWithProviders(<DeleteDataCard />);
    await userEvent.click(screen.getByRole("button", { name: "Delete all data…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete all data" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Net Lantern couldn't delete its data.");
    expect(screen.getByRole("alertdialog")).toContainElement(alert);
    expect(screen.queryByText("All data deleted.")).not.toBeInTheDocument();
  });

  it("a_left_behind_backup_is_a_warning_that_says_the_data_was_cleared", async () => {
    mockFetch({
      "POST /api/data/reset": [
        500,
        {
          code: "reset_failed",
          message: "Your data was cleared, but an old backup file could not be removed.",
          hint: "Try again.",
        },
      ],
    });
    const { client } = renderWithProviders(<DeleteDataCard />);
    const invalidate = vi.spyOn(client, "invalidateQueries");
    await userEvent.click(screen.getByRole("button", { name: "Delete all data…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete all data" }));
    expect(
      await screen.findByText("Your data was cleared, but an old backup file could not be removed."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(invalidate).toHaveBeenCalled();
  });

  it("a_dropped_connection_after_confirming_counts_as_probably_done", async () => {
    vi.stubGlobal("fetch", vi.fn(() => Promise.reject(new TypeError("Failed to fetch"))));
    const { client } = renderWithProviders(<DeleteDataCard />);
    const invalidate = vi.spyOn(client, "invalidateQueries");
    await userEvent.click(screen.getByRole("button", { name: "Delete all data…" }));
    await userEvent.click(screen.getByRole("button", { name: "Delete all data" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Probably deleted. If this page stops responding, reload it on the laptop — and check there that phone access is off.",
    );
    expect(screen.queryByText(/^All data deleted/)).not.toBeInTheDocument();
    expect(invalidate).toHaveBeenCalled();
  });
});
