function toNumber(ip: unknown): number | null {
  if (typeof ip !== "string") return null;
  const parts = ip.split(".");
  if (parts.length !== 4) return null;
  let n = 0;
  for (const p of parts) {
    if (!/^\d{1,3}$/.test(p) || Number(p) > 255) return null;
    n = n * 256 + Number(p);
  }
  return n;
}

function toDotted(n: number): string {
  return [n >>> 24, (n >>> 16) & 255, (n >>> 8) & 255, n & 255].join(".");
}

/** The network address of `ip`/`prefix` (e.g. 192.168.1.23/24 → 192.168.1.0), or null if either is invalid. */
export function networkAddress(ip: unknown, prefix: unknown): string | null {
  const n = toNumber(ip);
  if (n === null || typeof prefix !== "number" || !Number.isInteger(prefix)) return null;
  if (prefix < 0 || prefix > 32) return null;
  const mask = prefix === 0 ? 0 : (0xffffffff << (32 - prefix)) >>> 0;
  return toDotted((n & mask) >>> 0);
}

/** Numeric IPv4 order, for `Array.sort`. Anything unparseable sorts last. */
export function compareIp(a: string, b: string): number {
  const x = toNumber(a);
  const y = toNumber(b);
  if (x === null || y === null) return x === null ? (y === null ? a.localeCompare(b) : 1) : -1;
  return x - y;
}
