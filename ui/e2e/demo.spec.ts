import { test, expect, type Page } from "@playwright/test";

// One page for both tests: the launch link's token is single-use, so the second
// test must run in the same browser context (same session cookie) as the first.
test.describe.serial("demo mode", () => {
  let page: Page;

  test.beforeAll(async ({ browser }) => {
    page = await browser.newPage();
    await page.goto(process.env.NETLANTERN_URL!);
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

  test("devices show vendors, names and types", async () => {
    await page.getByRole("link", { name: "Devices" }).click();
    const tv = page.getByRole("row", { name: /\[TV\] Living Room/ });
    await expect(tv).toContainText("Samsung");
    // The Type cell itself, not the "[TV]" in the row's name.
    await expect(tv.getByRole("cell", { name: "TV", exact: true })).toBeVisible();
    await expect(page.getByRole("row", { name: /192\.168\.1\.44/ })).toContainText(
      "Private address",
    );
  });

  test("rename and star persist across a reload", async () => {
    await page.getByRole("button", { name: "DiskStation", exact: true }).click();
    await page.getByLabel("Name").fill("Backup box");
    await page.getByLabel("Name").press("Enter");
    await page.getByRole("button", { name: "Star Backup box" }).first().click();
    await page.reload();
    await page.getByRole("button", { name: /Starred/ }).click();
    await expect(page.getByRole("row", { name: /Backup box/ })).toBeVisible();
  });

  test("diagnose: every layer passes on the demo network", async () => {
    await page.getByRole("link", { name: "Diagnose" }).click();
    await page.getByRole("button", { name: "Run diagnosis" }).click();
    await expect(page.getByRole("heading", { name: "Everything looks fine." })).toBeVisible({
      timeout: 30_000,
    });
    const steps = page.getByRole("list", { name: "Diagnosis steps" }).locator(":scope > li");
    await expect(steps).toHaveCount(5);
    for (let i = 0; i < 5; i++) await expect(steps.nth(i)).toContainText("Passed");
  });

  test("diagnose: Wi-Fi quality and nearby networks", async () => {
    await expect(page.getByLabel("Wi-Fi quality")).toContainText("Lantern Home");
    await expect(page.getByLabel("Nearby networks").getByRole("row").nth(1)).toBeVisible();
    await expect(page.getByRole("img", { name: /2\.4 GHz channels/ })).toBeVisible();
  });

  test("diagnose: the last run is still shown after a reload", async () => {
    await page.reload();
    await expect(page.getByRole("heading", { name: "Everything looks fine." })).toBeVisible();
  });
});
