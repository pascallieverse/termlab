// Takes the README screenshots from the UI running on its mocked daemon (src/devMock.ts),
// so they only ever show sample data. The release workflow runs this for every published
// release (.github/workflows/screenshots.yml); run it by hand with `npm run screenshots`.
//
// Writes docs/screenshots/termlab.png (the framed hero image), focus.png and
// mission-control.png. Uses Playwright's Chromium; set PW_CHANNEL=chrome to use an
// installed Google Chrome instead of downloading one.

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { chromium } from "playwright";

const appDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = resolve(appDir, "../../docs/screenshots");
const WIDTH = 1440;
const HEIGHT = 900;
const SCALE = 2;

// Its own port, so a running `npm run tauri dev` (port 1420) is left alone.
const server = await createServer({ root: appDir, logLevel: "warn", server: { port: 1488, strictPort: false } });
await server.listen();
const url = server.resolvedUrls.local[0];

const browser = await chromium.launch({
  channel: process.env.PW_CHANNEL || undefined,
  // Headless Chrome captures the terminal's WebGL canvas blank at 2x, so the terminal uses
  // xterm's DOM renderer here (the app falls back to it when WebGL is unavailable).
  args: ["--disable-webgl"],
});
try {
  const context = await browser.newContext({
    viewport: { width: WIDTH, height: HEIGHT },
    deviceScaleFactor: SCALE,
    colorScheme: "dark",
    reducedMotion: "reduce",
    // macOS key labels (⌘K) in the UI, as most people will see it.
    userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
  });
  await context.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
    try {
      localStorage.setItem("termlab.inspector", "open");
    } catch {}
  });
  // Nothing outside the dev server: no remote pictures, so every run looks the same.
  await context.route(/^https?:\/\/(?!localhost[:/])/, (route) => route.abort());

  const page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e.message));
  await page.goto(url, { waitUntil: "networkidle" });
  await page.evaluate(() => document.fonts.ready);

  // Focus view on the session that is waiting for an approval.
  await page.getByText("Move invoice retries to a queue worker").first().click();
  await page.locator(".xterm").first().waitFor();
  await page.waitForTimeout(1500);
  await page.mouse.move(0, HEIGHT - 1);
  const focus = await page.screenshot();

  await page.getByRole("button", { name: "Mission control" }).click();
  await page.waitForTimeout(800);
  const mission = await page.screenshot();

  await mkdir(outDir, { recursive: true });
  await writeFile(resolve(outDir, "focus.png"), focus);
  await writeFile(resolve(outDir, "mission-control.png"), mission);

  // The hero image: the focus view in a window frame on a soft background.
  const inter = (await readFile(resolve(appDir, "node_modules/@fontsource/inter/files/inter-latin-500-normal.woff2"))).toString("base64");
  const frame = await context.newPage();
  await frame.setViewportSize({ width: WIDTH + 160, height: HEIGHT + 28 + 160 });
  await frame.setContent(`<!doctype html>
<html><head><style>
  @font-face { font-family: Inter; font-weight: 500; src: url(data:font/woff2;base64,${inter}); }
  html, body { margin: 0; }
  body {
    width: ${WIDTH + 160}px; height: ${HEIGHT + 28 + 160}px; display: grid; place-items: center;
    background: radial-gradient(120% 90% at 20% 0%, #3b2d6b 0%, #191a2e 45%, #0b0c0e 100%);
  }
  .window {
    width: ${WIDTH}px; border-radius: 12px; overflow: hidden; background: #0b0c0e;
    box-shadow: 0 0 0 1px rgba(255,255,255,.08), 0 30px 80px rgba(0,0,0,.55);
  }
  .bar { height: 28px; display: flex; align-items: center; gap: 8px; padding: 0 12px; background: #1b1c20; }
  .bar i { width: 12px; height: 12px; border-radius: 50%; display: block; }
  .bar span { flex: 1; text-align: center; margin-right: 52px; font: 500 13px Inter, sans-serif; color: #9a9ca5; }
  img { display: block; width: ${WIDTH}px; height: ${HEIGHT}px; }
</style></head><body>
  <div class="window">
    <div class="bar"><i style="background:#ff5f57"></i><i style="background:#febc2e"></i><i style="background:#28c840"></i><span>Termlab</span></div>
    <img src="data:image/png;base64,${focus.toString("base64")}" alt="">
  </div>
</body></html>`);
  await frame.evaluate(() => Promise.all([document.querySelector("img").decode(), document.fonts.load("500 13px Inter")]));
  await writeFile(resolve(outDir, "termlab.png"), await frame.screenshot());

  console.log(`Screenshots written to ${outDir}`);
} finally {
  await browser.close();
  await server.close();
}
