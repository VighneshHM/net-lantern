import { describe, expect, it } from "vitest";
import { formatAgo } from "./time";

describe("formatAgo", () => {
  it("handles the boundaries", () => {
    expect(formatAgo(0, 59_000)).toBe("just now");
    expect(formatAgo(0, 60_000)).toBe("1 min ago");
    expect(formatAgo(0, 59 * 60_000)).toBe("59 min ago");
    expect(formatAgo(0, 3_600_000)).toBe("1 h ago");
    expect(formatAgo(0, 86_400_000 - 1)).toBe("23 h ago");
    expect(formatAgo(0, 86_400_000)).toBe("1 day ago");
    expect(formatAgo(0, 2 * 86_400_000)).toBe("2 days ago");
  });
  it("treats a future time as just now", () => {
    expect(formatAgo(10_000, 0)).toBe("just now");
  });
});
