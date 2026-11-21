import { afterEach, describe, expect, it, vi } from "vitest";
import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PhoneStatus } from "../api/settings";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { PhoneAccess } from "./PhoneAccess";

const URL_ON = "http://192.168.1.20:47800/?t=abc";

function status(over: Partial<PhoneStatus> = {}): PhoneStatus {
  return {
    state: "off",
    reason: null,
    url: null,
    firewall: null,
    category: "private",
    demo: false,
    ...over,
  };
}

function on(over: Partial<PhoneStatus> = {}): PhoneStatus {
  return status({ state: "on", url: URL_ON, firewall: "allowed", ...over });
}

const PUBLIC_TEXT =
  "Phone access is paused because Windows marks this network as Public. On a network you trust, open Settings → Network & internet → Wi-Fi → your network and set it to Private.";

afterEach(() => {
  vi.useRealTimers();
});

describe("PhoneAccess", () => {
  it("off_shows_the_explanation_before_enabling", async () => {
    const calls = mockFetch({
      "GET /api/phone-access": [200, status()],
      "POST /api/phone-access": [200, on()],
    });
    renderWithProviders(<PhoneAccess />);
    expect(
      await screen.findByText(
        "Open this dashboard on your phone over Wi-Fi. Your phone and this laptop must be on the same network.",
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Allow phone access" }));
    expect(
      screen.getByText(/Windows may ask whether Net Lantern can use the network\./),
    ).toHaveTextContent(
      "Windows may ask whether Net Lantern can use the network. Choose Allow for Private networks. This needs an administrator's approval once. Anyone with the QR code can open the dashboard until you turn phone access off.",
    );
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(0);
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() =>
      expect(calls.filter((c) => c.method === "POST")).toEqual([
        { method: "POST", path: "/api/phone-access", body: { enabled: true } },
      ]),
    );
    expect(await screen.findByText(URL_ON)).toBeInTheDocument();
  });

  it("cancel closes the explanation without sending anything", async () => {
    const calls = mockFetch({ "GET /api/phone-access": [200, status()] });
    renderWithProviders(<PhoneAccess />);
    await userEvent.click(await screen.findByRole("button", { name: "Allow phone access" }));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByRole("button", { name: "Allow phone access" })).toBeInTheDocument();
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(0);
  });

  it.each([
    ["allowed", "Windows Firewall allows phones to connect."],
    ["off", "Windows Firewall allows phones to connect."],
    ["not_decided", "Waiting for your answer to the Windows Firewall prompt."],
    [
      "blocked",
      "Windows Firewall is blocking phones. Open Windows Security → Firewall & network protection → Allow an app through firewall, find netlantern.exe, and tick Private. Then turn phone access off and on. If it is still blocked, delete the Block rules for netlantern.exe in Windows Defender Firewall → Advanced settings → Inbound Rules.",
    ],
  ] as const)("on_shows_qr_url_and_firewall_line_for_each_verdict (%s)", async (verdict, line) => {
    mockFetch({ "GET /api/phone-access": [200, on({ firewall: verdict })] });
    renderWithProviders(<PhoneAccess />);
    expect(await screen.findByText(URL_ON)).toBeInTheDocument();
    expect(await screen.findByRole("img", { name: "QR code for the phone link" })).toHaveAttribute(
      "src",
      expect.stringMatching(/^data:image\/svg\+xml/),
    );
    expect(screen.getByText("Scan this with your phone's camera.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy link" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Turn off phone access" })).toBeInTheDocument();
    const el = screen.getByTestId("firewall-line");
    expect(el).toHaveTextContent(line);
    if (verdict === "blocked") expect(el).toHaveAttribute("role", "alert");
    else expect(el).not.toHaveAttribute("role");
  });

  it("not_decided_polls_until_a_verdict", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const calls = mockFetch({ "GET /api/phone-access": [200, on({ firewall: "not_decided" })] });
    renderWithProviders(<PhoneAccess />);
    expect(
      await screen.findByText("Waiting for your answer to the Windows Firewall prompt."),
    ).toBeInTheDocument();
    const gets = () => calls.filter((c) => c.method === "GET").length;
    const before = gets();
    mockFetch({ "GET /api/phone-access": [200, on({ firewall: "allowed" })] });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2100);
    });
    expect(
      await screen.findByText("Windows Firewall allows phones to connect."),
    ).toBeInTheDocument();
    expect(gets()).toBe(before);
  });

  it("not_decided stops polling after 60 seconds", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const calls = mockFetch({ "GET /api/phone-access": [200, on({ firewall: "not_decided" })] });
    renderWithProviders(<PhoneAccess />);
    await screen.findByText("Waiting for your answer to the Windows Firewall prompt.");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(61_000);
    });
    const after = calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(20_000);
    });
    expect(calls.length).toBe(after);
    expect(after).toBeLessThanOrEqual(1 + 31);
    expect(after).toBeGreaterThan(20);
  });

  it("not_decided shows the timed-out explanation after 60 seconds", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockFetch({ "GET /api/phone-access": [200, on({ firewall: "not_decided" })] });
    renderWithProviders(<PhoneAccess />);
    await screen.findByText("Waiting for your answer to the Windows Firewall prompt.");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(61_000);
    });
    const line = await screen.findByTestId("firewall-line");
    expect(line).toHaveTextContent(
      "Windows Firewall isn't letting phones in yet. If you didn't see a prompt, open Windows Security → Firewall & network protection → Allow an app through firewall, find netlantern.exe and tick Private. Then turn phone access off and on.",
    );
    expect(line).toHaveAttribute("role", "alert");
    expect(
      screen.queryByText("Waiting for your answer to the Windows Firewall prompt."),
    ).not.toBeInTheDocument();
  });

  it("a failed refetch keeps the card and shows a small alert", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    mockFetch({ "GET /api/phone-access": [200, on({ firewall: "not_decided" })] });
    renderWithProviders(<PhoneAccess />);
    await screen.findByText(URL_ON);
    mockFetch({ "GET /api/phone-access": [500, { code: "boom", message: "Boom.", hint: null }] });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2100);
    });
    expect(await screen.findByText("Couldn't refresh phone access status.")).toHaveAttribute(
      "role",
      "alert",
    );
    expect(screen.getByText(URL_ON)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Turn off phone access" })).toBeInTheDocument();
  });

  it("paused for bind_failed says why", async () => {
    mockFetch({
      "GET /api/phone-access": [200, status({ state: "paused", reason: "bind_failed" })],
    });
    renderWithProviders(<PhoneAccess />);
    expect(await screen.findByText(/Phone access is paused\./)).toHaveTextContent(
      "Phone access is paused. Net Lantern couldn't listen on this computer's network address; it will keep trying.",
    );
  });

  it("a QR code that fails to draw says so and keeps the link", async () => {
    vi.resetModules();
    vi.doMock("qrcode", () => ({
      toString: () => Promise.reject(new Error("no")),
    }));
    try {
      const { PhoneAccess: Fresh } = await import("./PhoneAccess");
      mockFetch({ "GET /api/phone-access": [200, on()] });
      renderWithProviders(<Fresh />);
      expect(
        await screen.findByText("The QR code couldn't be drawn — use the link below."),
      ).toBeInTheDocument();
      expect(screen.getByText(URL_ON)).toBeInTheDocument();
    } finally {
      vi.doUnmock("qrcode");
    }
  });

  it("demo_note_shown_only_in_demo", async () => {
    mockFetch({ "GET /api/phone-access": [200, on({ demo: true })] });
    const first = renderWithProviders(<PhoneAccess />);
    expect(
      await screen.findByText("Demo mode: this link works on this laptop only."),
    ).toBeInTheDocument();
    first.unmount();
    mockFetch({ "GET /api/phone-access": [200, on({ demo: false })] });
    renderWithProviders(<PhoneAccess />);
    await screen.findByText(URL_ON);
    expect(screen.queryByText(/Demo mode/)).not.toBeInTheDocument();
  });

  it("paused_public_and_not_connected_texts", async () => {
    mockFetch({
      "GET /api/phone-access": [200, status({ state: "paused", reason: "public_network" })],
    });
    const first = renderWithProviders(<PhoneAccess />);
    expect(await screen.findByText(/Phone access is paused because/)).toHaveTextContent(
      PUBLIC_TEXT,
    );
    expect(screen.getByRole("button", { name: "Turn off phone access" })).toBeInTheDocument();
    first.unmount();
    mockFetch({
      "GET /api/phone-access": [200, status({ state: "paused", reason: "not_connected" })],
    });
    renderWithProviders(<PhoneAccess />);
    expect(
      await screen.findByText("Phone access is paused until this laptop is connected to a network."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Turn off phone access" })).toBeInTheDocument();
  });

  it("enable_409_public_network_shows_the_public_text", async () => {
    mockFetch({
      "GET /api/phone-access": [200, status()],
      "POST /api/phone-access": [
        409,
        {
          code: "public_network",
          message: "Phone access stays off on Public networks.",
          hint: "Open Settings",
        },
      ],
    });
    renderWithProviders(<PhoneAccess />);
    await userEvent.click(await screen.findByRole("button", { name: "Allow phone access" }));
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText(/Phone access is paused because/)).toHaveTextContent(
      PUBLIC_TEXT,
    );
    expect(screen.getByRole("button", { name: "Turn off phone access" })).toBeInTheDocument();
  });

  it("enable failure other than public shows the server message", async () => {
    mockFetch({
      "GET /api/phone-access": [200, status()],
      "POST /api/phone-access": [
        503,
        { code: "phone_access_unavailable", message: "Connect to a network first.", hint: null },
      ],
    });
    renderWithProviders(<PhoneAccess />);
    await userEvent.click(await screen.findByRole("button", { name: "Allow phone access" }));
    await userEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Connect to a network first.");
  });

  it("turn_off_posts_false", async () => {
    const calls = mockFetch({
      "GET /api/phone-access": [200, on()],
      "POST /api/phone-access": [200, status()],
    });
    renderWithProviders(<PhoneAccess />);
    await userEvent.click(await screen.findByRole("button", { name: "Turn off phone access" }));
    await waitFor(() =>
      expect(calls.filter((c) => c.method === "POST")).toEqual([
        { method: "POST", path: "/api/phone-access", body: { enabled: false } },
      ]),
    );
    expect(await screen.findByRole("button", { name: "Allow phone access" })).toBeInTheDocument();
  });

  it("copy link writes the URL to the clipboard", async () => {
    mockFetch({ "GET /api/phone-access": [200, on()] });
    const user = userEvent.setup();
    const spy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
    renderWithProviders(<PhoneAccess />);
    await user.click(await screen.findByRole("button", { name: "Copy link" }));
    expect(spy).toHaveBeenCalledWith(URL_ON);
    expect(await screen.findByText("Copied")).toBeInTheDocument();
  });
});
