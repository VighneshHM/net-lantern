import { describe, expect, it } from "vitest";
import { signalLabel } from "./signal";

describe("signalLabel", () => {
  it("labels signal strength", () => {
    expect(signalLabel(-50)).toBe("Excellent");
    expect(signalLabel(-51)).toBe("Good");
    expect(signalLabel(-60)).toBe("Good");
    expect(signalLabel(-70)).toBe("Fair");
    expect(signalLabel(-71)).toBe("Weak");
  });
});
