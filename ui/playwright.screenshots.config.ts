import { defineConfig, devices } from "@playwright/test";

// Separate from playwright.config.ts so `npm run e2e` never regenerates the README images.
// It reuses the e2e global setup: the demo exe on its own pipe, with the URL in NETLANTERN_URL.
export default defineConfig({
  testDir: "./e2e-screenshots",
  globalSetup: "./e2e/global-setup.ts",
  workers: 1,
  fullyParallel: false,
  // No retries: the launch token is single-use.
  retries: 0,
  timeout: 90_000,
  reporter: "list",
  use: { viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } } }],
});
