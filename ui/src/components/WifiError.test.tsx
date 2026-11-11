import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ApiRequestError } from "../api/client";
import { WifiError } from "./WifiError";

describe("WifiError", () => {
  it("shows the message and, when present, the hint", () => {
    render(<WifiError error={new ApiRequestError(503, { code: "wifi_unavailable", message: "Off.", hint: "Turn it on." })} />);
    expect(screen.getByText("Off.")).toBeInTheDocument();
    expect(screen.getByText("Turn it on.")).toBeInTheDocument();
  });

  it("leaves out a missing hint", () => {
    const { container } = render(
      <WifiError error={new ApiRequestError(503, { code: "wifi_unavailable", message: "Off.", hint: null })} />,
    );
    expect(container.querySelectorAll("p")).toHaveLength(1);
  });

  it("shows a plain sentence for a failure that is not an API error", () => {
    render(<WifiError error={new TypeError("Cannot read properties of undefined (reading 'x')")} />);
    expect(screen.getByText("Couldn't read Wi-Fi details.")).toBeInTheDocument();
    expect(screen.queryByText(/Cannot read/)).toBeNull();
  });
});
