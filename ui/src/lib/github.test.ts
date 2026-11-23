import { describe, expect, it } from "vitest";
import { isReleaseUrl } from "./github";

describe("isReleaseUrl", () => {
  it("accepts a Net Lantern release page", () => {
    expect(isReleaseUrl("https://github.com/VighneshHM/net-lantern/releases/tag/v0.2.0")).toBe(true);
  });

  it.each([
    null,
    undefined,
    "",
    "https://github.com.evil.com/x",
    "http://github.com/x",
    "https://github.com@evil.com/x",
    "https://github.com",
    "https://evil.example/x",
    "https://github.com/other/x",
    "https://github.com/VighneshHM/net-lantern/issues",
    "https://github.com/VighneshHM/net-lantern/releases",
    "https://github.com/VighneshHM/net-lantern-evil/releases/x",
  ])("rejects %s", (url) => {
    expect(isReleaseUrl(url)).toBe(false);
  });
});
