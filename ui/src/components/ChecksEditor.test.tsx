import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { device } from "../test/fixtures";
import { mockFetch } from "../test/mockFetch";
import { renderWithProviders } from "../test/render";
import { ChecksEditor } from "./ChecksEditor";

describe("ChecksEditor", () => {
  it("adds an HTTP check and shows a server validation message", async () => {
    const d = device({ id: 7, starred: true, checks: [{ kind: "tcp", port: 22 }] });
    const calls = mockFetch({
      "PATCH /api/devices/7": [
        400,
        { code: "bad_request", message: "Port 99999 is not valid.", hint: null },
      ],
    });
    renderWithProviders(<ChecksEditor device={d} />);
    expect(screen.getByText(/TCP port 22/)).toBeInTheDocument();
    expect(
      screen.getByText(
        "Checks run every 30 s against this device's own address. Ping is always on for starred devices.",
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Add check" }));
    await userEvent.selectOptions(screen.getByLabelText("Kind"), "http");
    await userEvent.clear(screen.getByLabelText("Port"));
    await userEvent.type(screen.getByLabelText("Port"), "8080");
    expect(screen.getByLabelText("Path")).toHaveValue("/");
    await userEvent.click(screen.getByLabelText("HTTPS"));
    await userEvent.click(screen.getByRole("button", { name: "Save check" }));
    const patch = calls.find((c) => c.method === "PATCH");
    expect(patch?.body).toEqual({
      checks: [
        { kind: "tcp", port: 22 },
        { kind: "http", port: 8080, path: "/", https: true },
      ],
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Port 99999 is not valid.");
  });

  it("removes a check", async () => {
    const d = device({ id: 7, starred: true, checks: [{ kind: "tcp", port: 22 }] });
    const calls = mockFetch({ "PATCH /api/devices/7": [200, d] });
    renderWithProviders(<ChecksEditor device={d} />);
    await userEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({ checks: [] });
  });

  it("is not shown for unstarred devices", () => {
    const { container } = renderWithProviders(<ChecksEditor device={device({ starred: false })} />);
    expect(container).toBeEmptyDOMElement();
  });
});
