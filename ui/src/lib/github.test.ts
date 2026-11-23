import { describe, expect, it } from "vitest";
import { isGithubUrl } from "./github";

describe("isGithubUrl", () => {
  it("accepts a github.com https URL", () => {
    expect(isGithubUrl("https://github.com/VighneshHM/net-lantern/releases/tag/v0.2.0")).toBe(true);
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
  ])("rejects %s", (url) => {
    expect(isGithubUrl(url)).toBe(false);
  });
});
