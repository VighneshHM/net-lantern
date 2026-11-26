// Renders the Net Lantern icon set from icons/netlantern.svg (full detail) and
// icons/netlantern-small.svg (fewer, bolder shapes for 16-32 px).
//
// Needs the UI's dev dependencies and Chromium: run `npm ci` in ../../ui and
// `npx playwright install chromium` once. Then, from apps/desktop:
//   node scripts/make-icons.mjs
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..");
const uiModules = path.resolve(root, "..", "..", "ui", "node_modules", "@playwright", "test", "index.mjs");
const { chromium } = await import(pathToFileURL(uiModules).href);

const full = readFileSync(path.join(root, "icons", "netlantern.svg"));
const small = readFileSync(path.join(root, "icons", "netlantern-small.svg"));
const out = path.join(root, "src-tauri", "icons");

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: 1 });

/** PNG bytes of the icon at `size` px, transparent outside the rounded tile. */
async function render(size) {
  const svg = size <= 32 ? small : full;
  const src = `data:image/svg+xml;base64,${svg.toString("base64")}`;
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(
    `<style>html,body{margin:0;background:transparent}</style><img src="${src}" width="${size}" height="${size}" style="display:block">`,
  );
  await page.evaluate(() => Promise.all([...document.images].map((i) => i.decode())));
  return page.screenshot({ omitBackground: true, clip: { x: 0, y: 0, width: size, height: size } });
}

/** An ICO file holding PNG images (valid from Windows Vista on). */
function ico(images) {
  const head = Buffer.alloc(6 + 16 * images.length);
  head.writeUInt16LE(0, 0);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(images.length, 4);
  let offset = head.length;
  images.forEach(({ size, png }, i) => {
    const e = 6 + 16 * i;
    head.writeUInt8(size >= 256 ? 0 : size, e);
    head.writeUInt8(size >= 256 ? 0 : size, e + 1);
    head.writeUInt16LE(1, e + 4);
    head.writeUInt16LE(32, e + 6);
    head.writeUInt32LE(png.length, e + 8);
    head.writeUInt32LE(offset, e + 12);
    offset += png.length;
  });
  return Buffer.concat([head, ...images.map((i) => i.png)]);
}

const sizes = [16, 24, 32, 48, 64, 128, 256, 512];
const png = {};
for (const s of sizes) png[s] = await render(s);
await browser.close();

writeFileSync(path.join(out, "32x32.png"), png[32]);
writeFileSync(path.join(out, "128x128.png"), png[128]);
writeFileSync(path.join(out, "128x128@2x.png"), png[256]);
writeFileSync(path.join(out, "icon.png"), png[512]);
writeFileSync(
  path.join(out, "icon.ico"),
  ico([16, 24, 32, 48, 64, 256].map((size) => ({ size, png: png[size] }))),
);
console.log("icons written to", out);
