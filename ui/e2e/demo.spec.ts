import { test, expect, type Page } from "@playwright/test";

// One page for both tests: the launch link's token is single-use, so the second
// test must run in the same browser context (same session cookie) as the first.
test.describe.serial("demo mode", () => {
  let page: Page;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage();
    await page.goto(process.env.NETBOT_URL!);
  });

  test.afterAll(async () => {
    await page.close();
  });

  test("demo: scan finds the simulated network", async () => {
    await expect(page.getByText("192.168.1.0/24")).toBeVisible();
    await page.getByRole("button", { name: "Quick scan" }).click();
    await expect(page.getByText("13 devices found")).toBeVisible({ timeout: 30_000 });
    await page.getByRole("link", { name: "Devices" }).click();
    await expect(page.getByRole("row", { name: /192\.168\.1\.88/ })).toContainText(
      "doesn't answer ping",
    );
  });

  test("reload after the one-time token is used keeps working", async () => {
    await expect(page).not.toHaveURL(/\?t=/);
    // The previous test left the page on /devices: reloading a deep link must
    // keep working on the session cookie too.
    await page.reload();
    await expect(page.getByRole("heading", { name: "Devices" })).toBeVisible();
    await page.getByRole("link", { name: "Dashboard" }).click();
    await page.reload();
    await expect(page.getByRole("button", { name: "Quick scan" })).toBeVisible();
  });
});
