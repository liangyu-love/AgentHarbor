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
// A state is a fixture mode plus optional interactions, so refactors are also checked
// on views that only appear after user input.
const PAGES = [
  { name: "image", mode: "image" },
  { name: "image-jobs", mode: "image", async act(page) {
    await page.locator('textarea[placeholder^="描述画面主体"]').fill("Snapshot canvas");
    await page.locator("[data-image-generate]").click();
    await page.locator("[data-image-generate]").click();
    await page.locator("[data-image-generate]").click();
    await page.waitForFunction(() => window.review.imageRequests.length === 2);
    await page.evaluate(() => window.review.imageResolvers[0]());
    await page.locator('[data-generation-job-status="completed"] img').first().waitFor();
    await page.locator("[data-page-scroll-container]").evaluate(element => { element.scrollTop = 0; });
  } },
  { name: "image-gallery", mode: "image", async act(page) {
    await page.getByRole("button", { name: "历史画册", exact: true }).first().click();
    await page.locator("[data-gallery-section]").waitFor();
    await page.waitForFunction(() => window.review.thumbnailCalls.length > 0);
    await page.evaluate(() => { for (const resolve of window.review.thumbnailResolvers) resolve(); });
  } },
  { name: "image-preview", mode: "image", async act(page) {
    await page.locator('textarea[placeholder^="描述画面主体"]').fill("Snapshot preview");
    await page.locator("[data-image-generate]").click();
    await page.waitForFunction(() => window.review.imageRequests.length === 1);
    await page.evaluate(() => window.review.imageResolvers[0]());
    await page.locator('button[title="点击放大预览"]').first().click();
    await page.getByRole("dialog", { name: "成图放大预览" }).waitFor();
  } },
  { name: "groups-expanded", mode: "groups", async act(page) {
    await page.getByRole("button", { name: "展开同中转配置" }).first().click();
    await page.getByRole("button", { name: "收起层叠堆叠" }).last().scrollIntoViewIfNeeded();
  } },
  { name: "groups-next-member", mode: "groups", async act(page) {
    await page.getByRole("button", { name: "下一个站点配置" }).first().click();
    await page.locator("[data-relay-card-face]").first().scrollIntoViewIfNeeded();
  } },
  { name: "groups-filter", mode: "groups", async act(page) {
    await page.getByRole("button", { name: /^solo\.invalid/ }).first().click();
  } },
  { name: "groups-alias", mode: "groups", async act(page) {
    await page.locator("[data-relay-card-face]").first().getByRole("button", { name: "设置别名" }).click();
    await page.getByRole("dialog").waitFor();
  } },
  { name: "groups-network", mode: "groups", async act(page) {
    await page.locator("[data-relay-card-face]").first().getByRole("button", { name: "设置网络模式" }).click();
    await page.getByRole("menu").waitFor();
  } },
  { name: "groups-remove", mode: "groups", async act(page) {
    await page.locator("[data-relay-card-face]").first().getByRole("button", { name: "删除", exact: true }).click();
    await page.getByRole("alertdialog").waitFor();
  } },
  { name: "access-restart", mode: "access", async act(page) {
    await page.getByRole("button", { name: "重启 Codex Desktop" }).click();
    await page.getByRole("alertdialog").waitFor();
  } },
  { name: "threads-grid", mode: "thread-actions", async act(page) {
    await page.getByTitle("网格视图").click();
  } },
  { name: "threads-selected", mode: "thread-actions", async act(page) {
    await page.getByRole("button", { name: "全选当前" }).click();
  } },
  { name: "threads-group-selected", mode: "thread-actions", async act(page) {
    await page.getByRole("button", { name: "全选本工程" }).click();
  } },
  { name: "threads-collapsed", mode: "thread-actions", async act(page) {
    await page.getByRole("button", { name: "收起全部" }).click();
  } },
  { name: "threads-search-empty", mode: "thread-actions", async act(page) {
    await page.getByPlaceholder("搜索标题、工程、会话 ID、提供商或模型…").fill("no-such-thread");
  } },
  { name: "threads-sort-menu", mode: "thread-actions", async act(page) {
    await page.getByRole("button", { name: "最近活跃" }).click();
    await page.getByRole("menu").waitFor();
  } },
  { name: "thread-queue-recovery", mode: "thread-queue-recovery" },
  { name: "accounts-cards", mode: "accounts", async act(page) {
    await page.locator("article").first().scrollIntoViewIfNeeded();
  } },
  { name: "accounts-search", mode: "accounts", async act(page) {
    await page.getByPlaceholder("搜索邮箱或昵称").fill("beta");
    await page.locator("article").first().scrollIntoViewIfNeeded();
  } },
  { name: "accounts-no-match", mode: "accounts", async act(page) {
    await page.getByPlaceholder("搜索邮箱或昵称").fill("nobody");
    await page.getByPlaceholder("搜索邮箱或昵称").scrollIntoViewIfNeeded();
  } },
  { name: "accounts-plan-menu", mode: "accounts", async act(page) {
    await page.getByRole("combobox").click();
    await page.getByRole("listbox").waitFor();
  } },
  { name: "accounts-remove", mode: "accounts", async act(page) {
    await page.getByTitle("删除账号").first().click();
    await page.getByRole("alertdialog").waitFor();
  } },
  { name: "mcp-add", mode: "mcp", async act(page) {
    await page.getByRole("button", { name: "添加 MCP" }).first().click();
    await page.getByRole("dialog").waitFor();
  } },
  { name: "mcp-edit-http", mode: "mcp", async act(page) {
    await page.getByTitle("编辑").nth(1).click();
    await page.getByRole("dialog").waitFor();
  } },
  { name: "mcp-remove", mode: "mcp", async act(page) {
    await page.getByRole("button", { name: "更多操作" }).nth(1).click();
    await page.getByRole("menuitem", { name: "删除" }).click();
    await page.getByRole("alertdialog").waitFor();
  } },
  { name: "mcp-column-menu", mode: "mcp", async act(page) {
    await page.locator('[data-mcp-column="codex"]').click();
    await page.getByRole("menu").waitFor();
  } },
  { name: "mcp-search-empty", mode: "mcp", async act(page) {
    await page.locator("input").first().fill("no-such-server");
  } },
  ...["groups", "access", "warnings", "thread-actions", "mcp", "accounts", "usage", "overview"]
    .map(mode => ({ name: mode, mode })),
];
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
      // backdrop-filter rasterizes nondeterministically under overlays; it is not what refactors change.
      + '*,*::before,*::after{backdrop-filter:none!important}'
      + '</style></head><body style="margin:0;overflow:hidden"><div id="root"></div><script src="/fixture.js"></script></body></html>');
  }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const address = "http://127.0.0.1:" + server.address().port;
await mkdir(mode === "capture" ? baselineDir : currentDir, { recursive: true });

const browser = await playwright.chromium.launch({ headless: true, channel: "chromium",
  // Tiled GPU raster occasionally shifts single pixels under overlays; keep raster deterministic.
  args: ["--disable-gpu", "--disable-partial-raster", "--force-color-profile=srgb"] });
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
    // SNAPSHOT_ONLY=name1,name2 limits a run to a few states when chasing a flaky one.
    const only = process.env.SNAPSHOT_ONLY?.split(",");
    for (const { name, mode: fixtureMode, act } of PAGES.filter(state => !only || only.includes(state.name))) {
      await page.goto(address);
      await page.waitForFunction(() => Boolean(window.review));
      // States must not leak persisted UI choices (view mode, saved config) into each other.
      await page.evaluate(() => localStorage.clear());
      await page.evaluate(value => {
        document.documentElement.classList.toggle("dark", value === "dark");
      }, theme);
      await page.evaluate(value => window.review.mount(value), fixtureMode);
      await page.waitForLoadState("networkidle");
      await page.waitForTimeout(600);
      if (act) {
        await act(page);
        await page.waitForTimeout(400);
      }
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
          const box = [Infinity, Infinity, -1, -1];
          for (let i = 0; i < x.length; i += 4) {
            if (x[i] !== y[i] || x[i + 1] !== y[i + 1] || x[i + 2] !== y[i + 2] || x[i + 3] !== y[i + 3]) {
              pixels += 1;
              const px = (i / 4) % left.width, py = Math.floor(i / 4 / left.width);
              box[0] = Math.min(box[0], px); box[1] = Math.min(box[1], py); box[2] = Math.max(box[2], px); box[3] = Math.max(box[3], py);
            }
          }
          return { size: false, pixels, box };
        }, ["data:image/png;base64," + baseline.toString("base64"), "data:image/png;base64," + shot.toString("base64")]);
        if (diff.size || diff.pixels > 0) {
          failures.push(`${file}: ${diff.size ? "size differs" : `${diff.pixels} pixels differ in ${diff.box.join(",")}`}`);
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
