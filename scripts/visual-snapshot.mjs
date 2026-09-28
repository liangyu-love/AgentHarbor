// Pixel snapshots of the large pages, used to prove refactors change no UI.
// Run after `pnpm build`:
//   node scripts/visual-snapshot.mjs capture   -> writes .visual-baseline/*.png
//   node scripts/visual-snapshot.mjs compare   -> diffs against the baseline, exits 1 on change
import { createRequire } from "node:module";
import { mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const mode = process.argv[2];
if (mode !== "capture" && mode !== "compare") {
  console.error("usage: node scripts/visual-snapshot.mjs capture|compare");
  process.exit(2);
}
const playwright = require("playwright");
const esbuild = require(require.resolve("esbuild", { paths: [require.resolve("vite")] }));
const baselineDir = path.join(root, ".visual-baseline");
const currentDir = path.join(root, ".visual-current");

// Pages under refactor; each renders from the regression fixture with synthetic data.
const PAGES = ["image", "groups", "access", "warnings", "thread-actions", "mcp", "accounts", "usage", "overview"];
const THEMES = ["light", "dark"];

const fixture = await readFile(path.join(root, "scripts/fixtures/frontend-regressions.tsx"), "utf8");
const bundle = await esbuild.build({ stdin: { contents: fixture, loader: "tsx", resolveDir: root },
  bundle: true, write: false, format: "iife", platform: "browser", alias: { "@": path.join(root, "src") },
  define: { "process.env.NODE_ENV": '"production"' } });
const assets = path.join(root, "dist/assets");
const cssEntries = await Promise.all((await readdir(assets)).filter(name => name.endsWith(".css"))
  .map(async name => ({ name, modified: (await stat(path.join(assets, name))).mtimeMs })));
if (!cssEntries.length) throw new Error("Build CSS is missing. Run pnpm build first.");
cssEntries.sort((a, b) => b.modified - a.modified);
const css = await readFile(path.join(assets, cssEntries[0].name), "utf8");

const server = http.createServer((request, response) => {
  if (request.url === "/fixture.js") {
    response.setHeader("Content-Type", "text/javascript");
    response.end(bundle.outputFiles[0].text);
  } else {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><html><head><style>' + css
      + '*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}'
      + '</style></head><body style="margin:0;overflow:hidden"><div id="root"></div><script src="/fixture.js"></script></body></html>');
  }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const address = "http://127.0.0.1:" + server.address().port;
await mkdir(mode === "capture" ? baselineDir : currentDir, { recursive: true });

const browser = await playwright.chromium.launch({ headless: true, channel: "chromium" });
const failures = [];
try {
  for (const theme of THEMES) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, deviceScaleFactor: 1,
      colorScheme: theme, reducedMotion: "reduce", timezoneId: "UTC", locale: "zh-CN" });
    // Freeze time so relative dates and quota reset labels are stable.
    await context.addInitScript(() => {
      const fixed = 1_800_000_000_000;
      const RealDate = Date;
      // eslint-disable-next-line no-global-assign
      Date = class extends RealDate {
        constructor(...args) { super(...(args.length ? args : [fixed])); }
        static now() { return fixed; }
      };
      Math.random = () => 0.5;
    });
    const page = await context.newPage();
    await page.route("**/*", route => route.request().url().startsWith(address + "/") ? route.continue() : route.abort());
    for (const name of PAGES) {
      await page.goto(address);
      await page.waitForFunction(() => Boolean(window.review));
      await page.evaluate(value => {
        document.documentElement.classList.toggle("dark", value === "dark");
      }, theme);
      await page.evaluate(value => window.review.mount(value), name);
      await page.waitForLoadState("networkidle");
      await page.waitForTimeout(600);
      const file = `${name}-${theme}.png`;
      const shot = await page.screenshot({ fullPage: false });
      if (mode === "capture") {
        await writeFile(path.join(baselineDir, file), shot);
        console.log("captured", file);
        continue;
      }
      await writeFile(path.join(currentDir, file), shot);
      let baseline;
      try { baseline = await readFile(path.join(baselineDir, file)); } catch { failures.push(`${file}: no baseline`); continue; }
      if (!baseline.equals(shot)) {
        // Byte differences can come from PNG encoding alone; compare decoded pixels.
        const diff = await page.evaluate(async ([a, b]) => {
          const load = src => new Promise((resolve, reject) => {
            const image = new Image(); image.onload = () => resolve(image); image.onerror = reject; image.src = src;
          });
          const [left, right] = await Promise.all([load(a), load(b)]);
          if (left.width !== right.width || left.height !== right.height) return { size: true, pixels: -1 };
          const canvas = document.createElement("canvas"); canvas.width = left.width; canvas.height = left.height;
          const context = canvas.getContext("2d");
          context.drawImage(left, 0, 0); const x = context.getImageData(0, 0, left.width, left.height).data;
          context.clearRect(0, 0, left.width, left.height);
          context.drawImage(right, 0, 0); const y = context.getImageData(0, 0, left.width, left.height).data;
          let pixels = 0;
          for (let i = 0; i < x.length; i += 4) {
            if (x[i] !== y[i] || x[i + 1] !== y[i + 1] || x[i + 2] !== y[i + 2] || x[i + 3] !== y[i + 3]) pixels += 1;
          }
          return { size: false, pixels };
        }, ["data:image/png;base64," + baseline.toString("base64"), "data:image/png;base64," + shot.toString("base64")]);
        if (diff.size || diff.pixels > 0) {
          failures.push(`${file}: ${diff.size ? "size differs" : diff.pixels + " pixels differ"}`);
          continue;
        }
      }
      console.log("same", file);
    }
    await context.close();
  }
} finally {
  await browser.close();
  server.close();
}
if (failures.length) {
  console.error("VISUAL DIFF\n" + failures.join("\n"));
  process.exit(1);
}
if (mode === "compare") console.log("PASS all snapshots match");
