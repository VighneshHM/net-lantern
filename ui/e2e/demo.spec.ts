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
    await expect(page.getByText("192.168.1.0/24").first()).toBeVisible();
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

  test("the dashboard starts monitoring after a scan", async () => {
    await page.getByRole("link", { name: "Dashboard" }).click();
    const router = page.getByLabel("Key numbers").getByText("Router", { exact: true }).locator("..");
    await expect(router).toContainText(/\d+ ms$/, { timeout: 10_000 });
    await expect(page.getByText(/router average .* ms, internet average .* ms\./)).toBeVisible({
      timeout: 10_000,
    });
  });

  test("the alerts screen shows rules and no active alerts", async () => {
    await page.getByRole("link", { name: "Alerts", exact: true }).click();
    await expect(page.getByRole("checkbox", { name: /Starred device offline/ })).toBeChecked();
    await expect(page.getByText("No alerts here.")).toBeVisible();
  });

  test("starring a device shows it under starred devices", async () => {
    await page.getByRole("link", { name: "Devices" }).click();
    // The NAS is renamed "Backup box" by the rename test; star it if it is not starred yet.
    // Exact: a substring match would also find "Unstar Backup box" and unstar it.
    const star = page.getByRole("button", { name: "Star Backup box", exact: true });
    if ((await star.count()) > 0) await star.first().click();
    await expect(page.getByRole("button", { name: "Unstar Backup box" }).first()).toBeVisible();
    await page.getByRole("link", { name: "Dashboard" }).click();
    const starred = page.getByLabel("Starred devices");
    await expect(starred).toContainText("Backup box");
    await expect(starred).toContainText("Online");
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

  test("the map draws the demo network and opens device details", async () => {
    await page.getByRole("link", { name: "Map" }).click();
    await expect(page.getByRole("heading", { name: "Network map" })).toBeVisible();
    await expect(
      page.getByRole("img", { name: /^Network map: the router, the internet and 12 devices/ }),
    ).toBeVisible();
    await page
      .getByLabel("Map devices")
      .getByRole("button", { name: /Backup box/ })
      .click();
    await expect(page.getByLabel("Name")).toHaveValue("Backup box");
    await page.getByRole("button", { name: /^Problems/ }).click();
    const list = page.getByLabel("Map devices");
    const none = page.getByText("No problems right now.");
    if ((await none.count()) === 0) {
      await expect(list.getByRole("button", { name: /Backup box/ })).toHaveCount(0);
    } else {
      await expect(none).toBeVisible();
    }
  });

  test("the map fits a phone screen", async () => {
    try {
      await page.setViewportSize({ width: 375, height: 812 });
      await page.reload();
      await expect(page.getByRole("img", { name: /^Network map:/ })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375);
      const box = await page.getByRole("button", { name: "Zoom in" }).boundingBox();
      expect(box!.height).toBeGreaterThanOrEqual(44);
    } finally {
      await page.setViewportSize({ width: 1280, height: 800 });
    }
  });

  test("the Devices table hides IP, MAC and Latency below 900px", async () => {
    try {
      await page.setViewportSize({ width: 800, height: 900 });
      await openRoute(page, ROUTES[1]);
      const table = page.getByRole("table");
      for (const h of ["IP", "MAC", "Latency"])
        await expect(table.getByRole("columnheader", { name: h, exact: true })).toBeHidden();
      await expect(table.getByRole("columnheader", { name: "Name" })).toBeVisible();
      const row = table.getByRole("row", { name: /192\.168\.1\.88/ });
      // This device has no name, so its button already reads as the IP: look at the small line under it.
      await expect(row.locator("span", { hasText: /^192\.168\.1\.88$/ })).toBeVisible();
      await expect(row.locator("td", { hasText: /^192\.168\.1\.88$/ })).toBeHidden();
      await page.setViewportSize({ width: 1280, height: 800 });
      for (const h of ["IP", "MAC", "Latency"])
        await expect(table.getByRole("columnheader", { name: h, exact: true })).toBeVisible();
    } finally {
      await page.setViewportSize({ width: 1280, height: 800 });
    }
  });

  async function checkWidth(width: number, height: number) {
    try {
      await page.setViewportSize({ width, height });
      const failures: string[] = [];
      for (const route of ROUTES) {
        await openRoute(page, route);
        const sw = await page.evaluate(() => document.documentElement.scrollWidth);
        if (sw > width) failures.push(`${route.path} scrollWidth ${sw} > ${width}`);
      }
      expect(failures).toEqual([]);
    } finally {
      await page.setViewportSize({ width: 1280, height: 800 });
    }
  }

  test("every screen fits a phone", async () => {
    await checkWidth(375, 812);
  });

  test("every screen fits a small laptop", async () => {
    await checkWidth(800, 900);
  });

  test("touch targets are at least 44px", async () => {
    try {
      const failures: string[] = [];
      for (const [width, height] of [
        [375, 812],
        [800, 900],
      ]) {
        await page.setViewportSize({ width, height });
        for (const route of ROUTES) {
          await openRoute(page, route);
          const { measured, small } = await measureTargets(page);
          // Guard against a selector that matches nothing.
          if (measured.length === 0) failures.push(`${width} ${route.path}: nothing was measured`);
          if (route.path === "/alerts" && !measured.some((m) => m.startsWith("input")))
            failures.push(`${width} /alerts: no checkbox was measured`);
          if (route.path === "/map")
            for (const name of ["Zoom in", "Zoom out", "Fit"])
              if (!measured.some((m) => m.includes(`"${name}`)))
                failures.push(`${width} /map: ${name} was not measured`);
          for (const f of small) failures.push(`${width} ${route.path}: ${f}`);
        }
      }
      expect(failures).toEqual([]);
    } finally {
      await page.setViewportSize({ width: 1280, height: 800 });
    }
  });

  test("phone access works from a second browser and stops when turned off", async ({ browser }) => {
    await page.getByRole("link", { name: "Settings", exact: true }).first().click();
    await expect(page.getByRole("heading", { level: 1, name: "Settings" })).toBeVisible();
    await page.getByRole("button", { name: "Allow phone access" }).click();
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page.getByText("Demo mode: this link works on this laptop only.")).toBeVisible();

    // The link carries the phone token: it is read here and used only to navigate.
    const link = (await page.locator("code").filter({ hasText: "?t=" }).first().textContent()) ?? "";
    expect(link).toMatch(/^http:\/\/[^/]+\/\?t=./);
    const origin = new URL(link).origin;

    const phone = await browser.newContext();
    const stranger = await browser.newContext();
    try {
      // The phone has no cookies; the link signs it in.
      const phonePage = await phone.newPage();
      await phonePage.goto(link);
      await expect(phonePage.getByRole("heading", { level: 1, name: "Dashboard" })).toBeVisible();
      await expect(phonePage).not.toHaveURL(/\?t=/);
      expect((await phone.request.get(`${origin}/api/network`)).status()).toBe(200);

      // A third browser with no session and no token gets nothing.
      expect((await stranger.request.get(`${origin}/api/network`)).status()).toBe(401);

      await page.getByRole("button", { name: "Turn off phone access" }).click();
      await expect(page.getByRole("button", { name: "Allow phone access" })).toBeVisible();

      // The phone's session ended with phone access.
      await expect
        .poll(async () => {
          try {
            return (await phone.request.get(`${origin}/api/network`)).status();
          } catch {
            return "closed"; // the listener is gone, which also locks the phone out
          }
        })
        .not.toBe(200);
      await phonePage.reload().catch(() => undefined);
      await expect(phonePage.getByRole("heading", { level: 1, name: "Dashboard" })).toHaveCount(0);
    } finally {
      await phone.close();
      await stranger.close();
    }
  });
  // The demo /api/update scenario: setting is null (the question card is shown on the
  // Dashboard) and the latest release is the CURRENT version, so there is never a banner.
  // The banner is a manual checkpoint step (scenarios/update-available.json). Answering
  // removes the card from "/", which no earlier test depends on.
  test("update question can be answered and changed in Settings", async () => {
    await page.getByRole("link", { name: "Dashboard" }).click();
    await expect(page.getByRole("heading", { name: "Check for new versions?" })).toBeVisible();
    await page.getByRole("button", { name: "No thanks" }).click();
    await expect(page.getByRole("heading", { name: "Check for new versions?" })).toHaveCount(0);

    await page.getByRole("link", { name: "Settings", exact: true }).first().click();
    const check = page.getByRole("checkbox", { name: "Check GitHub for new versions" });
    await expect(check).not.toBeChecked();
    await check.check();
    await expect(check).toBeChecked();
    await expect(page.getByText("You have version 0.1.0.")).toBeVisible();
    await expect(page.getByText(/is available/)).toHaveCount(0);
  });

  test("copy debug info gives redacted text", async () => {
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    await page.getByRole("button", { name: "Copy debug info" }).click();
    // The text is never logged: only checked.
    const fallback = page.getByLabel("Debug info");
    await expect(page.getByRole("status").filter({ hasText: "Copied" }).or(fallback)).toBeVisible();
    const text =
      (await fallback.count()) > 0
        ? await fallback.inputValue()
        : await page.evaluate(() => navigator.clipboard.readText());
    expect(text.startsWith("Net Lantern")).toBe(true);
    expect(text).not.toMatch(/192\.168\.1\./);
    expect(text).not.toContain("Living Room");
  });

  // Runs LAST: the reset clears everything, including the setting and the demo network.
  test("delete all data clears the demo network", async () => {
    await page.getByRole("button", { name: "Delete all data…" }).click();
    const dialog = page.getByRole("alertdialog");
    await dialog.getByRole("button", { name: "Delete all data" }).click();
    await expect(page.getByText(/^All data deleted\./)).toBeVisible();
    await page.getByRole("link", { name: "Devices" }).click();
    await expect(page.getByText("No devices yet")).toBeVisible();
    await page.getByRole("link", { name: "Dashboard" }).click();
    await expect(page.getByRole("heading", { name: "Network detected" })).toBeVisible();
  });
});

const ROUTES: { path: string; name: string; heading: string }[] = [
  { path: "/", name: "Dashboard", heading: "Dashboard" },
  { path: "/devices", name: "Devices", heading: "Devices" },
  { path: "/map", name: "Map", heading: "Network map" },
  { path: "/diagnose", name: "Diagnose", heading: "Diagnose" },
  { path: "/alerts", name: "Alerts", heading: "Alerts" },
  { path: "/settings", name: "Settings", heading: "Settings" },
];

/** Client-side navigation through the nav link, so the single-use token and session survive. */
async function openRoute(page: Page, route: { path: string; name: string; heading: string }) {
  if (route.path === "/") {
    await page.getByRole("link", { name: "Dashboard" }).click();
  } else {
    await page.getByRole("link", { name: route.name, exact: true }).first().click();
  }
  await expect(page).toHaveURL(new RegExp(`${route.path === "/" ? "/$" : route.path + "$"}`));
  // The route's own heading, so nothing from the previous screen is measured.
  await expect(page.getByRole("heading", { level: 1, name: route.heading })).toBeVisible();
}

/** Every visible interactive control on the page, measured. `small` lists those under 44x44. */
async function measureTargets(page: Page): Promise<{ measured: string[]; small: string[] }> {
  return page.evaluate(() => {
    const measured: string[] = [];
    const small: string[] = [];
    const sel =
      "input:not([type=hidden]), select, textarea, button, a[href], [role=button], summary";
    for (const el of document.querySelectorAll<HTMLElement>(sel)) {
      if (el.hasAttribute("data-inline-link")) continue;
      const r = el.getBoundingClientRect();
      const style = getComputedStyle(el);
      if (r.width === 0 || r.height === 0 || style.visibility === "hidden") continue;
      const label =
        el.getAttribute("aria-label") ??
        el.closest("label")?.textContent ??
        el.textContent ??
        "";
      const text = `${el.tagName.toLowerCase()} "${label.trim().slice(0, 40)}"`;
      measured.push(text);
      if (r.width < 43.99 || r.height < 43.99) {
        small.push(`${text} ${Math.round(r.width)}x${Math.round(r.height)}`);
      }
    }
    return { measured, small };
  });
}
