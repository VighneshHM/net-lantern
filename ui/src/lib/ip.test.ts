import { describe, expect, it } from "vitest";
import { compareIp, networkAddress } from "./ip";

describe("networkAddress", () => {
  it("masks the host bits", () => {
    expect(networkAddress("192.168.1.23", 24)).toBe("192.168.1.0");
    expect(networkAddress("10.20.30.40", 20)).toBe("10.20.16.0");
    expect(networkAddress("172.16.5.9", 12)).toBe("172.16.0.0");
  });

  it("handles the extremes", () => {
    expect(networkAddress("192.168.1.23", 32)).toBe("192.168.1.23");
    expect(networkAddress("192.168.1.23", 0)).toBe("0.0.0.0");
  });

  it("returns null for anything that is not an IPv4 address and prefix", () => {
    expect(networkAddress("not-an-ip", 24)).toBeNull();
    expect(networkAddress("192.168.1.256", 24)).toBeNull();
    expect(networkAddress("192.168.1.1", 33)).toBeNull();
    expect(networkAddress(undefined, 24)).toBeNull();
  });
});

describe("compareIp", () => {
  it("orders numerically, not as strings", () => {
    const ips = ["192.168.1.100", "192.168.1.9", "192.168.1.20", "10.0.0.200"];
    expect([...ips].sort(compareIp)).toEqual([
      "10.0.0.200",
      "192.168.1.9",
      "192.168.1.20",
      "192.168.1.100",
    ]);
  });
});
