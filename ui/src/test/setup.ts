import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach, vi } from "vitest";

// findBy and waitFor wait up to 3 s, so a busy machine running many test files
// at once does not time them out.
configure({ asyncUtilTimeout: 3000 });

if (typeof window !== "undefined") {
  if (!window.matchMedia) {
    window.matchMedia = ((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      dispatchEvent: () => false,
    })) as typeof window.matchMedia;
  }
  if (!("ResizeObserver" in globalThis)) {
    globalThis.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  }
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  // A test that switched to fake timers must not leave them to the next one.
  vi.useRealTimers();
});
