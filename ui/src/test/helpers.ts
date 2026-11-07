/** A `Location` for a path like `"/devices?t=abc#x"` (only path/search/hash are used). */
export function loc(url: string): Location {
  return new URL(url, "http://127.0.0.1:47800") as unknown as Location;
}

export interface FakeHistory extends History {
  /** The URL passed to the most recent `replaceState`, if any. */
  replaced?: string;
}

/** A `History` that records the URL given to `replaceState`. */
export function fakeHistory(state: unknown = null): FakeHistory {
  const h = {
    state,
    replaced: undefined as string | undefined,
    replaceState(_state: unknown, _unused: string, url?: string | URL | null) {
      h.replaced = url === undefined || url === null ? undefined : String(url);
    },
  };
  return h as unknown as FakeHistory;
}
