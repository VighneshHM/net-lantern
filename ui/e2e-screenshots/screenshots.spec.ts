import { test, expect, type Page } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const outDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "docs", "screenshots");

// One page for every shot: the launch link's token is single-use, so later shots
// must reuse the session cookie of the first.
test.describe.serial("README screenshots (demo network)", () => {
  let page: Page;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage({ viewport: { width: 1280, height: 800 } });
    await page.goto(process.env.NETLANTERN_URL!);
  });

  test.afterAll(async () => {
    await page.close();
  });

  async function shot(name: string) {
    await page.screenshot({ path: path.join(outDir, `${name}.png`) });
  }

  test("dashboard", async () => {
    await page.getByRole("button", { name: "Quick scan" }).click();
    await expect(page.getByText("13 devices found")).toBeVisible({ timeout: 30_000 });
    // Answer the one-time question so its card is not in the picture.
    await page.getByRole("button", { name: "No thanks" }).click();
    await expect(page.getByRole("heading", { name: "Check for new versions?" })).toHaveCount(0);
    // Monitoring has numbers once the router row shows a latency.
    const router = page.getByLabel("Key numbers").getByText("Router", { exact: true }).locator("..");
    await expect(router).toContainText(/\d+ ms$/, { timeout: 15_000 });
    await expect(page.getByText(/router average .* ms, internet average .* ms\./)).toBeVisible({
      timeout: 15_000,
    });
    // Let the demo network (running 60x faster) fill the latency graph.
    await page.waitForTimeout(45_000);
    await shot("dashboard");
  });

  test("devices with details open", async () => {
    await page.getByRole("link", { name: "Devices" }).click();
    await expect(page.getByRole("heading", { name: "Devices" })).toBeVisible();
    await page.getByRole("button", { name: "DiskStation", exact: true }).click();
    await expect(page.getByLabel("Name")).toBeVisible();
    await shot("devices");
  });

  test("diagnose", async () => {
    await page.getByRole("link", { name: "Diagnose" }).click();
    await page.getByRole("button", { name: "Run diagnosis" }).click();
    await expect(page.getByRole("heading", { name: "Everything looks fine." })).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByLabel("Wi-Fi quality")).toContainText("Lantern Home");
    await expect(page.getByLabel("Nearby networks").getByRole("row").nth(1)).toBeVisible();
    await shot("diagnose");
  });

  test("map", async () => {
    await page.getByRole("link", { name: "Map", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Network map" })).toBeVisible();
    await expect(page.getByRole("img", { name: /^Network map:/ })).toBeVisible();
    // Let the layout settle.
    await page.waitForTimeout(1500);
    await shot("map");
  });

  test("alerts", async () => {
    await page.getByRole("link", { name: "Alerts", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: /Starred device offline/ })).toBeChecked();
    await shot("alerts");
  });

  test("settings", async () => {
    await page.getByRole("link", { name: "Settings", exact: true }).first().click();
    await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy debug info" })).toBeVisible();
    await shot("settings");
  });

  test("phone", async () => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.getByRole("link", { name: "Dashboard" }).first().click();
    await expect(page.getByRole("heading", { level: 1, name: "Dashboard" })).toBeVisible();
    await expect(page.getByText("13 devices found")).toBeVisible();
    await shot("phone");
    await page.setViewportSize({ width: 1280, height: 800 });
  });
});
