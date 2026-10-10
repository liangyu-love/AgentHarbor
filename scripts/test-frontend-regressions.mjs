import { testClaudeSupport } from "./test-claude-support.mjs";
import { testPiIntegration } from "./test-pi-integration.mjs";
import { testImageDownload } from "./test-image-download.mjs";
import { testOAuthLogin } from "./test-oauth-login.mjs";
// Run after npm run build. No real Tauri backend, profiles, or external network.
// Optional: --playwright-module=/path/to/playwright-core or PLAYWRIGHT_MODULE_PATH.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdir, readFile, readdir, stat } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { testAccountThreadNavigation } from "./test-account-thread-navigation.mjs";
import { testThreadDetails } from "./test-thread-details.mjs";
import { testClaudeSessions } from "./test-claude-sessions.mjs";
import { testThreadUiFixes } from "./test-thread-ui-fixes.mjs";
import { testMagpieOptimizations } from "./test-magpie-optimizations.mjs";
import { testPriceCache } from "./test-price-cache.mjs";
import { testProviderFeedback } from "./test-provider-feedback.mjs";
import { testProviderModelResolutions } from "./test-provider-model-resolutions.mjs";
import { testTerminalProxy } from "./test-terminal-proxy.mjs";
import { testCodexRoutingTransitions } from "./test-codex-routing-transitions.mjs";
import { testRelayDeck } from "./test-relay-deck.mjs";
import { testUiRefresh } from "./test-ui-refresh.mjs";
import { testClaudeDesktopCard } from "./test-claude-desktop-card.mjs";
import { testSkillsActivation } from "./test-skills-activation.mjs";
import { testAggregation } from "./test-aggregation.mjs";
import { testCodexSubagents } from "./test-codex-subagents.mjs";
import { testCapabilityClear } from "./test-capability-clear.mjs";
import { testClaudeCliOfficial } from "./test-claude-cli-official.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const argument = name => process.argv.find(value => value.startsWith(name + "="))?.slice(name.length + 1);
const modulePath = argument("--playwright-module") ?? process.env.PLAYWRIGHT_MODULE_PATH;
let playwright;
for (const candidate of modulePath ? [modulePath] : ["playwright", "playwright-core"]) {
  try { playwright = require(candidate); break; } catch { /* Try the next explicitly available module. */ }
}
if (!playwright?.chromium) {
  throw new Error("Playwright is required. Install it or set PLAYWRIGHT_MODULE_PATH / --playwright-module to an installed playwright or playwright-core package.");
}
const esbuild = require(require.resolve("esbuild", { paths: [require.resolve("vite")] }));
const fixture = await readFile(path.join(root, "scripts/fixtures/frontend-regressions.tsx"), "utf8");
const bundle = await esbuild.build({ stdin: { contents: fixture, loader: "tsx", resolveDir: root },
  bundle: true, write: false, format: "iife", platform: "browser", loader: { ".svg": "dataurl" }, alias: { "@": path.join(root, "src") },
  define: { "process.env.NODE_ENV": '"production"' } });
const assets = path.join(root, "dist/assets");
let cssFiles;
try { cssFiles = (await readdir(assets)).filter(name => name.endsWith(".css")); } catch { cssFiles = []; }
if (!cssFiles.length) throw new Error("Build CSS is missing. Run npm run build before the frontend regressions.");
const cssEntries = await Promise.all(cssFiles.map(async name => ({ name, modified: (await stat(path.join(assets, name))).mtimeMs })));
cssEntries.sort((a, b) => b.modified - a.modified);
const css = await readFile(path.join(assets, cssEntries[0].name), "utf8");
const server = http.createServer((request, response) => {
  if (request.url === "/fixture.js") {
    response.setHeader("Content-Type", "text/javascript"); response.end(bundle.outputFiles[0].text);
  } else {
    response.setHeader("Content-Type", "text/html");
    response.end('<!doctype html><html><head><style>' + css + '</style></head><body style="margin:0;overflow:hidden"><div id="root"></div><script src="/fixture.js"></script></body></html>');
  }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const address = "http://127.0.0.1:" + server.address().port;
const screenshotDir = process.env.FRONTEND_QA_CAPTURE_DIR ?? path.join(root, ".playwright-cli");
const captureDir = process.env.FRONTEND_QA_CAPTURE_DIR ? screenshotDir : null;
await mkdir(screenshotDir, { recursive: true });
let browser;
try {
  const executablePath = argument("--browser-executable") ?? process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
  browser = await playwright.chromium.launch({ headless: true, ...(executablePath ? { executablePath } : { channel: "chromium" }) });
  const page = await browser.newPage({ viewport: { width: 1200, height: 700 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  // Native confirm/prompt would block the test silently; the app uses AlertDialog instead.
  page.on("dialog", async dialog => { errors.push(`unexpected native dialog: ${dialog.message()}`); await dialog.dismiss(); });
  const confirmAlert = async name => {
    const alert = page.getByRole("alertdialog");
    await alert.getByRole("button", { name, exact: true }).click();
    await alert.waitFor({ state: "detached" });
  };
  await page.route("**/*", route => route.request().url().startsWith(address + "/") ? route.continue() : route.abort());
  const reset = async mode => {
    await page.goto(address);
    await page.waitForFunction(() => Boolean(window.review));
    await page.evaluate(value => window.review.mount(value), mode);
  };

  if (process.env.FRONTEND_QA_SCOPE === "pi-integration") {
    await testPiIntegration({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "claude-support") {
    await testClaudeSupport({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "image-download") {
    await testImageDownload({ page, reset });
  } else if (process.env.FRONTEND_QA_SCOPE === "claude-cli-official") {
    await testClaudeCliOfficial({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "capability-clear") {
    await testCapabilityClear({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "codex-subagents") {
    await testCodexSubagents({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "aggregation") {
    await testAggregation({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "skills") {
    await testSkillsActivation({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "terminal-proxy") {
    await testTerminalProxy({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "codex-routing-transitions") {
    await testCodexRoutingTransitions({ page, reset });
  } else if (process.env.FRONTEND_QA_SCOPE === "relay-deck") {
    await testRelayDeck({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "ui-refresh") {
    await testUiRefresh({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "oauth-login") {
    await testOAuthLogin({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "provider-model-preview") {
    await testProviderModelResolutions({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "provider-feedback") {
    await testProviderFeedback({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "magpie-optimizations") {
    await testMagpieOptimizations({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "price-cache") {
    await testPriceCache({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "ui-review-fixes") {
    await testThreadUiFixes({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "account-thread-navigation") {
    await testAccountThreadNavigation({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "thread-details") {
    await testThreadDetails({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "claude-sessions") {
    await testClaudeSessions({ page, reset, captureDir });
  } else if (process.env.FRONTEND_QA_SCOPE === "claude-desktop-card") {
    await testClaudeDesktopCard({ page, reset, captureDir });
  } else {
  await reset("compact");
  const errorCases = await page.evaluate(() => {
    const { errorKey, errorMessage, formatInvokeError } = window.review.errors;
    return {
      keyString: formatInvokeError("image.errors.cancelled"),
      keyError: formatInvokeError(new Error("image.errors.cancelled")),
      unknownKey: formatInvokeError("image.errors.doesNotExist", "fallback"),
      raw: formatInvokeError("SQLite error: database is locked"),
      rawSentence: formatInvokeError("Codex config changed while AgentHarbor was updating it."),
      empty: formatInvokeError("", "fallback"),
      message: errorMessage(new Error("  boom  ")),
      keyed: errorKey("newapi.errors.network", ["newapi"], "newapi.errors.unexpected"),
      foreign: errorKey("clients.errors.provider", ["newapi"], "newapi.errors.unexpected"),
      rawKeyed: errorKey("newapi. oops", ["newapi"], "newapi.errors.unexpected"),
    };
  });
  assert.deepEqual(errorCases, {
    keyString: "已取消生成。",
    keyError: "已取消生成。",
    unknownKey: "fallback",
    raw: "SQLite error: database is locked",
    rawSentence: "Codex config changed while AgentHarbor was updating it.",
    empty: "fallback",
    message: "boom",
    keyed: "newapi.errors.network",
    foreign: "newapi.errors.unexpected",
    rawKeyed: "newapi.errors.unexpected",
  });
  console.log("PASS invoke errors translate i18n keys, keep raw text and fall back when empty");
  const retryCases = await page.evaluate(async () => {
    const cases = [];
    for (const failure of ["newapi.errors.network", "newapi.errors.timeout", "newapi.errors.auth", "newapi.errors.response", "newapi.errors.balanceUnsupported"]) {
      let calls = 0;
      window.__TAURI_INTERNALS__.invoke = async () => {
        calls++;
        if (calls === 1) throw failure;
        return { id: "retry-provider" };
      };
      let succeeded = false;
      try { await window.review.realBalanceQuery("retry-provider"); succeeded = true; } catch { /* expected deterministic failure */ }
      cases.push({ calls, succeeded });
    }
    delete window.__TAURI_INTERNALS__.invoke;
    return cases;
  });
  assert.deepEqual(retryCases, [{ calls: 2, succeeded: true }, { calls: 2, succeeded: true }, { calls: 1, succeeded: false }, { calls: 1, succeeded: false }, { calls: 1, succeeded: false }]);
  console.log("PASS balance API retries only transport failures and never retries auth or invalid responses");

  await page.getByText("Review thread 0", { exact: true }).waitFor();
  await page.locator("section").evaluate(element => { element.scrollTop = 2800; });
  await page.waitForFunction(() => [...document.querySelectorAll("[data-thread-title]")].some(element => {
    const bounds = element.getBoundingClientRect();
    return /^Review thread [5-9][0-9]$/.test(element.textContent ?? "") && bounds.top < 600 && bounds.bottom > 0;
  }));
  assert.equal(await page.evaluate(() => window.scrollY), 0);
  await page.evaluate(() => window.review.mount("grid"));
  // At 3 columns each group is ~820px tall, so Workspace 3 starts around
  // 2460px; scroll keeps its header inside the 600px-tall container.
  await page.locator("section").evaluate(element => { element.scrollTop = 2000; });
  await page.waitForFunction(() => [...document.querySelectorAll("h2")].some(element => {
    const bounds = element.getBoundingClientRect();
    const viewport = element.closest("[data-page-scroll-container]")?.getBoundingClientRect();
    return element.textContent === "Workspace 3" && viewport && bounds.top < viewport.bottom && bounds.bottom > viewport.top;
  }));
  console.log("PASS virtual list follows the PageStage scroll container in compact and grid modes");

  await reset("image");
  const providerSelect = page.getByRole("combobox", { name: "图像供应商" });
  await page.waitForFunction(() => document.querySelector('[role="combobox"][aria-label="图像供应商"]')?.textContent.includes("Provider image"));
  const selectImageProvider = async name => {
    await providerSelect.click();
    await page.getByRole("option", { name, exact: true }).click();
  };
  await selectImageProvider("独立配置 / 手动填写连接凭据");
  await page.locator('input[type="password"]').waitFor();
  assert.ok((await providerSelect.textContent()).includes("独立配置"));
  await page.locator('input[placeholder="https://example.com"]').fill("https://manual.invalid");
  await selectImageProvider("Provider image");
  await selectImageProvider("独立配置 / 手动填写连接凭据");
  assert.equal(await page.locator('input[placeholder="https://example.com"]').inputValue(), "https://manual.invalid");
  console.log("PASS explicit manual image provider selection remains selected and preserves its address");

  await reset("image");
  await selectImageProvider("独立配置 / 手动填写连接凭据");
  await page.evaluate(() => { window.review.imageKeySaveHold = true; });
  await page.locator('input[placeholder="https://example.com"]').fill("https://site-a.invalid");
  await page.locator('input[type="password"]').fill("sk-shared-fixture");
  await page.locator('textarea[placeholder^="描述画面主体"]').fill("Manual site A");
  await page.locator('[data-image-generate]').click();
  await page.waitForFunction(() => window.review.imageRequests.length === 1);
  assert.equal(await page.evaluate(() => window.review.imageRequests[0].apiKey), "sk-shared-fixture");
  await page.locator('input[placeholder="https://example.com"]').fill("https://site-b.invalid");
  await page.locator('input[type="password"]').fill("sk-shared-fixture");
  await page.evaluate(() => window.review.imageResolvers[0]());
  await page.locator('[data-generation-job-status="completed"]').waitFor();
  assert.equal(await page.locator('input[type="password"]').inputValue(), "sk-shared-fixture");
  assert.equal(await page.getByRole("button", { name: "忘记此站点的密钥" }).count(), 0);
  // Resolve only the old site's saves. A blur may independently save site B;
  // that success is allowed to clear its own key and is a different scenario.
  await page.evaluate(() => window.review.imageKeySaveResolvers.filter(resolve => resolve.site === "https://site-a.invalid").forEach(resolve => resolve()));
  await page.waitForFunction(() => window.review.imageKeySaveResolvedSites?.includes("https://site-a.invalid"));
  assert.equal(await page.locator('input[type="password"]').inputValue(), "sk-shared-fixture");
  assert.equal(await page.getByRole("button", { name: "忘记此站点的密钥" }).count(), 0);
  console.log("PASS late generation and key saves cannot change credentials for a different manual site");

  await reset("image");
  const imagePrompt = page.locator('textarea[placeholder^="描述画面主体"]');
  const imageSubmit = page.locator('[data-image-generate]');
  await providerSelect.click();
  await page.getByRole("option", { name: "Provider image", exact: true }).click();
  await imagePrompt.fill("First canvas");
  await imageSubmit.click();
  await imagePrompt.fill("Second canvas");
  await imageSubmit.click();
  await imagePrompt.fill("Third canvas");
  await imageSubmit.click();
  await imagePrompt.fill("Cancelled canvas");
  await imageSubmit.click();
  await page.waitForFunction(() => window.review.imageRequests.length === 2);
  assert.equal(await page.locator('[data-generation-job-status="queued"]').count(), 2);
  await page.locator('[data-generation-job-status="queued"]').filter({ hasText: "Cancelled canvas" })
    .getByRole("button", { name: "取消排队" }).click();
  assert.equal(await page.locator('[data-generation-job-status]').count(), 3);
  await page.evaluate(() => window.review.imageResolvers[1]());
  await page.waitForFunction(() => window.review.imageRequests.length === 3);
  assert.equal(await page.locator('[data-generation-job-status="running"]').filter({ hasText: "First canvas" }).count(), 1);
  await page.locator('[data-generation-job-status="completed"]').filter({ hasText: "Second canvas" }).locator("img").waitFor();
  await page.evaluate(() => window.review.imageResolvers[0]());
  await page.evaluate(() => window.review.imageResolvers[2]());
  await page.waitForFunction(() => window.review.gallerySaves === 3);
  assert.equal(await page.locator('[data-generation-job-status="completed"]').count(), 3);
  console.log("PASS image jobs run concurrently, keep separate canvases, queue and cancel independently");

  await reset("image");
  await providerSelect.click();
  await page.getByRole("option", { name: "Provider image", exact: true }).click();
  await page.evaluate(() => { window.review.failGallerySave = true; });
  await imagePrompt.fill("Retry canvas");
  await imageSubmit.click();
  await page.evaluate(() => window.review.imageResolvers[0]());
  const retryCanvas = page.locator('[data-generation-job-status="completed"]').filter({ hasText: "Retry canvas" });
  await retryCanvas.getByRole("button", { name: "重试保存" }).waitFor();
  await page.evaluate(() => { window.review.failGallerySave = false; });
  await retryCanvas.getByRole("button", { name: "重试保存" }).click();
  await page.waitForFunction(() => window.review.gallerySaves === 2);
  await retryCanvas.getByText("已存入本地画册").waitFor();
  await page.evaluate(() => { window.review.failOpenPath = true; });
  await page.getByRole("button", { name: "打开画册目录", exact: true }).first().click();
  await page.waitForFunction(() => window.review.openedPath === "C:/synthetic");
  assert.equal(await page.evaluate(() => window.review.openedPath), "C:/synthetic");
  assert.deepEqual(errors, []);
  console.log("PASS a failed gallery save remains on its canvas and can be retried");

  await reset("image");
  await providerSelect.click();
  await page.getByRole("option", { name: "Provider image", exact: true }).click();
  await imagePrompt.fill("Stopped canvas");
  await imageSubmit.click();
  const stoppedCanvas = page.locator('[data-generation-job-status="running"]').filter({ hasText: "Stopped canvas" });
  await stoppedCanvas.getByRole("button", { name: "取消生成" }).click();
  await page.locator('[data-generation-job-status="cancelled"]').filter({ hasText: "Stopped canvas" }).waitFor();
  assert.equal(await page.evaluate(() => window.review.cancelledJobs.length), 1);
  assert.equal(await page.locator('[data-generation-job-status="failed"]').count(), 0);
  console.log("PASS a running image job can be cancelled and shows as cancelled, not failed");

  await reset("image");
  await providerSelect.click();
  await page.getByRole("option", { name: "Provider image", exact: true }).click();
  await imagePrompt.fill("Two images on one canvas");
  await imageSubmit.click();
  await page.evaluate(() => window.review.imageResolvers[0]([window.review.pixel, window.review.pixel]));
  const imagePair = page.locator('[data-generation-job-status="completed"] .ink-result-images img');
  await imagePair.nth(1).waitFor();
  const imageLayout = await page.locator('[data-generation-job-status="completed"]').evaluate(job => {
    const parent = job.parentElement;
    const images = [...job.querySelectorAll(".ink-result-images img")].map(image => image.getBoundingClientRect());
    return { jobWidth: job.getBoundingClientRect().width, parentWidth: parent.getBoundingClientRect().width,
      firstTop: images[0].top, secondTop: images[1].top, firstLeft: images[0].left, secondLeft: images[1].left };
  });
  assert.ok(imageLayout.jobWidth >= imageLayout.parentWidth - 2, JSON.stringify(imageLayout));
  assert.ok(Math.abs(imageLayout.firstTop - imageLayout.secondTop) < 2 && imageLayout.secondLeft > imageLayout.firstLeft,
    JSON.stringify(imageLayout));
  if (captureDir) {
    await page.locator(".ink-jobs-grid").evaluate(element => element.scrollIntoView({ block: "center" }));
    await page.screenshot({ path: path.join(captureDir, "image-pair.png") });
  }
  console.log("PASS one image canvas fills the result panel and displays two images side by side");
  await testImageDownload({ page, reset });

  await reset("groups");
  await page.getByRole("heading", { name: "relay.invalid" }).waitFor();
  await page.getByRole("button", { name: "下一个站点配置" }).click();
  await page.locator(".relay-member-name", { hasText: "Relay Beta" }).waitFor();
  await page.getByText("beta-model", { exact: true }).waitFor();
  assert.equal(await page.locator("[data-relay-card-face]").first().evaluate(element => getComputedStyle(element).animationName), "relay-card-flip-next");
  const cardHeightsBefore = await page.locator("[data-relay-deck]").evaluateAll(cards => cards.map(card => card.getBoundingClientRect().height));
  await page.getByRole("button", { name: "展开同中转配置" }).click();
  const cardHeightsAfter = await page.locator("[data-relay-deck]").evaluateAll(cards => cards.map(card => card.getBoundingClientRect().height));
  assert.equal(cardHeightsBefore.length, 2);
  assert.ok(cardHeightsAfter[0] > cardHeightsBefore[0] + 40, JSON.stringify({ cardHeightsBefore, cardHeightsAfter }));
  assert.ok(Math.abs(cardHeightsAfter[1] - cardHeightsBefore[1]) < 2, JSON.stringify({ cardHeightsBefore, cardHeightsAfter }));
  if (captureDir) {
    await page.getByRole("heading", { name: "relay.invalid" }).evaluate(element => element.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: path.join(captureDir, "group-expanded.png") });
  }
  await page.getByRole("button", { name: "收起层叠堆叠" }).first().click();
  assert.equal(await page.locator("[data-relay-card-face]").first().evaluate(element => getComputedStyle(element).animationName), "none", "Collapsing must not replay a completed flip");
  assert.match(await page.locator(".relay-member-name").textContent(), /Relay Beta/);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "group-collapsed.png") });
  await page.getByRole("button", { name: "上一个站点配置" }).click();
  await page.locator(".relay-member-name", { hasText: "Relay Alpha" }).waitFor();
  assert.equal(await page.locator("[data-relay-card-face]").first().evaluate(element => getComputedStyle(element).animationName), "relay-card-flip-prev");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.getByRole("button", { name: "下一个站点配置" }).click();
  await page.locator(".relay-member-name", { hasText: "Relay Beta" }).waitFor();
  assert.equal(await page.locator("[data-relay-card-face]").first().evaluate(element => getComputedStyle(element).animationName), "none");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  console.log("PASS grouped relay switches members and expands without stretching independent cards");

  await reset("groups");
  await page.getByRole("button", { name: "查询余额 Relay Alpha", exact: true }).click();
  const balanceDialog = page.getByRole("dialog");
  await balanceDialog.waitFor();
  await balanceDialog.getByRole("button", { name: "刷新余额", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceRequests.length === 1);
  await page.evaluate(() => window.review.balanceResolvers.at(-1)());
  await balanceDialog.getByText("USD", { exact: true }).first().waitFor();
  await balanceDialog.getByRole("button", { name: "刷新余额", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceRequests.length === 2);
  await page.evaluate(() => window.review.balanceRejectors.at(-1)("newapi.errors.auth"));
  await balanceDialog.getByText("查询未成功，保留上次查询结果。", { exact: true }).waitFor();
  assert.ok((await balanceDialog.locator("strong").textContent()).includes("9"));
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "balance-dialog.png") });
  if (!await balanceDialog.locator("details").getAttribute("open").then(value => value !== null)) await balanceDialog.locator("summary").click();
  await balanceDialog.locator("#balance-user-id").fill("42");
  await balanceDialog.locator("#balance-access-token").fill("synthetic-token");
  await balanceDialog.getByRole("button", { name: "保存并查询", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceCredentialSaves.length === 1 && window.review.balanceRequests.length === 3);
  assert.deepEqual(await page.evaluate(() => window.review.balanceCredentialSaves[0]), { id: "group-a", expectedBaseUrl: "https://relay.invalid/v1", userId: 42, accessToken: "synthetic-token" });
  await page.evaluate(() => window.review.balanceResolvers.at(-1)());
  await balanceDialog.getByRole("button", { name: "刷新余额", exact: true }).waitFor({ state: "visible" });
  assert.equal(await balanceDialog.locator("#balance-access-token").inputValue(), "");
  await balanceDialog.getByRole("checkbox", { name: "自定义余额脚本", exact: true }).check();
  const scriptText = await balanceDialog.getByRole("textbox", { name: "自定义余额脚本", exact: true }).inputValue();
  assert.match(scriptText, /extractor/);
  assert.equal(await balanceDialog.getByRole("button", { name: "刷新余额", exact: true }).isDisabled(), true);
  await balanceDialog.getByRole("button", { name: "保存并查询", exact: true }).first().click();
  await page.waitForFunction(() => window.review.savedBalanceScript && window.review.balanceRequests.length === 4);
  assert.equal(await page.evaluate(() => window.review.savedBalanceScript.script), scriptText);
  await page.evaluate(() => window.review.balanceResolvers.at(-1)());
  await balanceDialog.getByText("USD", { exact: true }).first().waitFor();
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "balance-script.png") });

  await page.keyboard.press("Escape");
  await balanceDialog.waitFor({ state: "hidden" });
  assert.match(await page.locator(".relay-member-name").textContent(), /Relay Alpha/);
  assert.equal(await page.locator("[data-relay-card-face]").first().evaluate(element => getComputedStyle(element).animationName), "none");
  await page.getByRole("button", { name: "下一个站点配置" }).click();
  await page.locator(".relay-member-name", { hasText: "Relay Beta" }).waitFor();
  await page.getByRole("button", { name: "查询余额 Relay Beta", exact: true }).click();
  await balanceDialog.getByText("与「Relay Alpha」共享余额。", { exact: true }).waitFor();
  await balanceDialog.getByRole("button", { name: "单独查询" }).click();
  await balanceDialog.getByRole("button", { name: "使用共享余额" }).waitFor();
  await page.keyboard.press("Escape");
  await balanceDialog.waitFor({ state: "hidden" });
  await reset("provider");
  assert.equal(await page.locator("#newapi-provider-userId, #newapi-provider-accessToken").count(), 0);
  console.log("PASS card balance dialog saves credentials, preserves last good result and does not flip the stack");

  await testOAuthLogin({ page, reset, captureDir });

  await reset("accounts");
  const accountCard = page.locator("article").filter({ has: page.getByText("Alpha", { exact: true }) }).first();
  const showEmail = accountCard.getByRole("button", { name: "显示邮箱" });
  await showEmail.waitFor();
  assert.equal(await accountCard.getByRole("heading", { name: "alpha@example.invalid" }).count(), 0);
  assert.match(await accountCard.locator("h3").innerText(), /^a•+a@example\.invalid$/);
  await showEmail.click();
  assert.equal(await accountCard.locator("h3").innerText(), "alpha@example.invalid");
  await accountCard.getByRole("button", { name: "隐藏邮箱" }).click();
  assert.equal(await accountCard.getByRole("heading", { name: "alpha@example.invalid" }).count(), 0);
  console.log("PASS account email is masked by default and toggles without leaking through the title");

  await reset("overview");
  await page.getByText("http://127.0.0.1:7890").waitFor();
  await page.waitForFunction(() => document.querySelectorAll(".overview-capability-value strong")[2]?.textContent === "39");
  assert.equal(await page.locator(".overview-capability-meta").nth(2).innerText(), await page.evaluate(() => window.review.t("workspace.skillsSummary", { count: 2 })));
  assert.deepEqual(await page.evaluate(() => [...new Set(window.review.skillsReadCalls)].sort()), ["claudeCode", "codex"], "Overview includes both skill sources");
  const capabilityWidths = await page.locator(".overview-capability").evaluateAll(cards => cards.map(card => card.getBoundingClientRect().width));
  assert.equal(capabilityWidths.length, 4);
  assert.ok(Math.max(...capabilityWidths) - Math.min(...capabilityWidths) < 2, JSON.stringify(capabilityWidths));
  assert.equal(await page.locator(".overview-quick-card").count(), 5);
  assert.equal(await page.locator(".overview-quick-card").first().evaluate(element =>
    Boolean(element.closest("section")?.compareDocumentPosition(document.querySelector(".overview-system")) & Node.DOCUMENT_POSITION_FOLLOWING)), true);
  for (const [index, route] of ["accounts", "imageWorkbench", "threads", "maintenance", "settings"].entries()) {
    await page.locator(".overview-quick-card").nth(index).click();
    assert.equal(await page.evaluate(() => window.review.navigatedTo), route);
  }
  if (captureDir) {
    await page.locator(".overview-quick-card").first().scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(captureDir, "overview-modules.png") });
  }
  await page.getByRole("button", { name: "代理设置" }).click();
  assert.equal(await page.evaluate(() => window.review.proxySettingsOpened), true);
  if (captureDir) {
    await page.waitForTimeout(550);
    await page.screenshot({ path: path.join(captureDir, "overview-proxy.png") });
  }
  console.log("PASS overview uses four equal capability cards and exposes configured API proxy settings");

  await page.setViewportSize({ width: 1440, height: 900 });
  await reset("usage");
  await page.locator(".usage-today-card").first().waitFor();
  assert.equal(await page.locator(".usage-today-card").count(), 2);
  assert.equal(await page.locator(".usage-source-card .usage-metric-orange").count(), 2);
  const usageWidth = await page.locator(".usage-trend-card").evaluate(element => ({
    card: element.getBoundingClientRect().width, parent: element.parentElement.getBoundingClientRect().width,
  }));
  assert.ok(usageWidth.card >= usageWidth.parent - 44, JSON.stringify(usageWidth));
  if (captureDir) {
    await page.screenshot({ path: path.join(captureDir, "usage-today.png") });
    await page.locator(".usage-trend-card").evaluate(element => element.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: path.join(captureDir, "usage-trend.png") });
  }
  await page.locator(".usage-trend-svg").first().hover({ position: { x: 420, y: 100 } });
  assert.equal(await page.locator(".usage-model-chip").count(), 3);
  await page.getByRole("tab", { name: "请求明细" }).click();
  await page.getByText("1140 ms").waitFor();
  await page.getByText("320 ms").waitFor();
  assert.equal(await page.locator(".usage-request-table tbody tr").count(), 1);
  if (captureDir) {
    await page.locator(".usage-relay-panel").evaluate(element => element.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: path.join(captureDir, "usage-requests.png") });
  }
  await page.getByRole("tab", { name: "供应商与额度" }).click();
  await page.getByText("12.5 USD").waitFor();
  await page.getByRole("button", { name: "刷新余额 Relay Alpha" }).click();
  await page.waitForFunction(() => window.review.balanceRequests.includes("group-a"));
  await page.evaluate(() => window.review.balanceResolvers.at(-1)());
  await page.getByText("9 USD").waitFor();
  await page.getByRole("tab", { name: "模型" }).click();
  await page.locator(".usage-relay-body .usage-model-row").first().hover();
  assert.equal(await page.locator(".usage-relay-body .usage-donut-segment").first().getAttribute("stroke-width"), "16");
  await page.setViewportSize({ width: 390, height: 844 });
  await reset("usage");
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  console.log("PASS usage dashboard prioritizes today, gives trend full width, and switches relay details with interactive model segments");
  await page.setViewportSize({ width: 1200, height: 700 });

  await reset("provider");
  const labels = await page.evaluate(() => window.review.labels);
  const keyInput = page.locator('#newapi-provider-apiKey');
  assert.equal(await keyInput.inputValue(), '');
  await page.getByRole('button', { name: 'show', exact: true }).click();
  await page.evaluate(() => window.review.resolveApiKey());
  await page.waitForFunction(() => document.querySelector('#newapi-provider-apiKey').type === 'text');
  assert.equal(await keyInput.inputValue(), 'synthetic-key-A');
  await page.getByRole('button', { name: 'hide', exact: true }).click();
  assert.equal(await keyInput.getAttribute('type'), 'password');
  await keyInput.fill('synthetic-replacement');
  await page.getByRole('button', { name: 'show', exact: true }).click();
  assert.equal(await keyInput.inputValue(), 'synthetic-replacement');
  assert.deepEqual(await page.evaluate(() => window.review.revealedIds), ['A']);
  await reset('provider');
  await page.getByRole('button', { name: 'show', exact: true }).click();
  await page.getByRole('button', { name: labels.cancel, exact: true }).click();
  await page.locator('#open-b').click();
  await page.evaluate(() => window.review.resolveApiKey());
  assert.equal(await keyInput.inputValue(), '');
  assert.equal(await keyInput.getAttribute('type'), 'password');
  await page.getByRole('button', { name: 'show', exact: true }).click();
  await page.evaluate(() => window.review.rejectApiKey());
  await page.getByText(labels.apiKeyMissing, { exact: true }).waitFor();
  assert.equal(await keyInput.inputValue(), '');
  await page.getByRole('button', { name: 'show', exact: true }).click();
  await page.evaluate(() => window.review.resolveApiKey());
  await page.waitForFunction(() => document.querySelector('#newapi-provider-apiKey').value === 'synthetic-key-B');
  console.log('PASS API key reveal is explicit, hides correctly, preserves replacements, ignores stale dialog responses and retries errors');
  await reset('provider');
  // A model discovered during refresh must stay in the pending list until the
  // user explicitly includes it. Refreshes also keep the existing default.
  await page.getByRole("button", { name: labels.fetchModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "model-A", displayName: "model-A", contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream" },
    { id: "newly-fetched-model", displayName: "newly-fetched-model", contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream" },
  ]));
  await page.getByText("newly-fetched-model", { exact: true }).waitFor();
  assert.equal(await page.locator('[data-pending-models]').getByText('newly-fetched-model', { exact: true }).count(), 1);
  assert.equal(await page.getByRole('button', { name: /newly-fetched-model/ }).count(), 0, 'Pending models have no default or editing controls');
  assert.equal(await page.getByRole("checkbox", { name: labels.includeFetchedModel, exact: true }).isChecked(), false);
  assert.equal(await page.getByRole("button", { name: labels.defaultModelA, exact: true }).getAttribute("aria-pressed"), "true");
  // Unchecking or removing the default is allowed and clears the default
  // instead of silently moving it to another model.
  await page.getByRole("checkbox", { name: labels.includeModelA, exact: true }).uncheck();
  assert.equal(await page.locator('[data-model-default-empty]').count(), 1);
  assert.equal(await page.getByRole("button", { name: labels.defaultModelA, exact: true }).count(), 0);
  await page.getByRole("checkbox", { name: labels.includeModelA, exact: true }).check();
  await page.getByRole("button", { name: labels.defaultModelA, exact: true }).click();
  await page.getByRole("button", { name: labels.defaultModelA, exact: true }).click();
  assert.equal(await page.getByRole("button", { name: labels.defaultModelA, exact: true }).getAttribute("aria-pressed"), "false");
  await page.getByRole("button", { name: labels.clearModels, exact: true }).click();
  assert.equal(await page.locator('[data-model-default-empty]').count(), 1);
  assert.equal(await page.getByRole('checkbox', { checked: true }).count(), 0);
  await page.getByRole('button', { name: labels.clearPendingModels, exact: true }).click();
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.equal(await page.evaluate(() => window.review.saved.model), '');
  assert.deepEqual(await page.evaluate(() => window.review.saved.models), []);
  console.log("PASS refresh keeps new models pending, and default model can be cleared without replacement");
  await reset("provider");
  await page.getByRole("button", { name: labels.fetchModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "match-one", endpointTypes: ["responses"], included: true },
    { id: "match-two", endpointTypes: ["responses"], included: true },
    { id: "other-model", endpointTypes: ["responses"], included: true },
  ]));
  await page.locator('[data-pending-models]').getByText("other-model", { exact: true }).waitFor();
  const search = page.getByPlaceholder(labels.addModelPlaceholder, { exact: true });
  await search.fill("match-");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "provider-model-search.png"), animations: "disabled" });
  await page.getByRole("button", { name: labels.selectMatchingModels, exact: true }).click();
  await search.fill("");
  assert.equal(await page.locator('[data-pending-models]').getByText("other-model", { exact: true }).count(), 1);
  assert.equal(await page.locator('[data-pending-models]').getByText("match-one", { exact: true }).count(), 0);
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.deepEqual(await page.evaluate(() => window.review.saved.models.filter(model => model.included).map(model => model.id).sort()), ["match-one", "match-two", "model-A"]);
  console.log("PASS bulk model inclusion respects search and preserves unrelated pending models");


  await reset("provider");
  await page.getByRole("button", { name: labels.fetchModels, exact: true }).click();
  await page.getByRole("button", { name: labels.cancel, exact: true }).click();
  await page.locator("#open-b").click();
  await page.evaluate(() => window.review.resolveModels());
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.deepEqual(await page.evaluate(() => ({ id: window.review.saved.id, models: window.review.saved.models.map(model => model.id) })),
    { id: "B", models: ["model-B"] });
  await reset("provider");
  await page.getByRole("button", { name: labels.fetchModels, exact: true }).click();
  await page.locator("#newapi-provider-baseUrl").fill("https://changed.invalid");
  await page.evaluate(() => window.review.resolveModels());
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.deepEqual(await page.evaluate(() => window.review.saved.models.map(model => model.id)), ["model-A"]);
  console.log("PASS model fetch results cannot cross provider dialogs or changed connection settings");

  await reset("provider");
  await page.getByRole("spinbutton", { name: labels.modelContext, exact: true }).fill("100000");
  await page.getByText(labels.modelFieldOverrides, { exact: true }).click();
  await page.getByRole("spinbutton", { name: labels.modelInput, exact: true }).fill("80000");
  await page.getByRole("spinbutton", { name: labels.modelOutput, exact: true }).fill("4096");
  await page.getByRole("textbox", { name: labels.modelReasoning, exact: true }).fill("low, high");
  await page.getByRole("textbox", { name: labels.modelDefaultEffort, exact: true }).fill("high");
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.deepEqual(await page.evaluate(() => window.review.saved.models[0].overrides), {
    contextWindow: 100000, inputLimit: 80000, maxOutputTokens: 4096,
    reasoning: [{ effort: "low", description: "low" }, { effort: "high", description: "high" }], defaultEffort: "high",
  });
  assert.equal(await page.evaluate(() => window.review.saved.models[0].metadataSource), "upstream");
  await page.locator("#reopen-saved").click();
  await page.getByRole("spinbutton", { name: labels.modelContext, exact: true }).fill("");
  await page.getByText(labels.modelFieldOverrides, { exact: true }).click();
  await page.getByRole("spinbutton", { name: labels.modelInput, exact: true }).fill("");
  await page.getByRole("spinbutton", { name: labels.modelOutput, exact: true }).fill("");
  await page.getByRole("textbox", { name: labels.modelReasoning, exact: true }).fill("");
  await page.getByRole("textbox", { name: labels.modelDefaultEffort, exact: true }).fill("");
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => window.review.saved.models[0].overrides.contextWindow === null);
  assert.deepEqual(await page.evaluate(() => window.review.saved.models[0].overrides), {
    contextWindow: null, inputLimit: null, maxOutputTokens: null, reasoning: null, defaultEffort: null,
  });
  assert.equal(await page.evaluate(() => window.review.saved.models[0].metadataSource), "upstream");
  console.log("PASS per-field overrides preserve upstream provenance and clearing/reopening restores automatic values");

  await reset("mixed");
  const chatProtocol = page.getByRole("combobox", { name: labels.chatProtocol, exact: true });
  assert.ok((await chatProtocol.textContent()).includes(labels.chat));
  assert.equal(await page.getByRole("button", { name: labels.defaultResponses, exact: true }).getAttribute("aria-pressed"), "true");
  assert.equal(await page.getByRole("button", { name: labels.defaultChat, exact: true }).isEnabled(), true);
  await chatProtocol.click();
  await page.getByRole("option", { name: labels.responses, exact: true }).click();
  // The provider default is a radio group of described protocol cards.
  await page.locator("#newapi-provider-protocol").getByRole("radio", { name: labels.anthropic, exact: true }).click();
  assert.equal(await page.getByRole("button", { name: labels.defaultResponses, exact: true }).getAttribute("aria-pressed"), "true");
  assert.equal(await chatProtocol.textContent(), labels.responses);
  assert.ok((await page.getByRole("combobox", { name: labels.inheritedProtocol, exact: true }).textContent()).includes(labels.anthropic));
  await page.getByRole("button", { name: labels.defaultChat, exact: true }).click();
  await page.getByRole("button", { name: labels.fetchModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "chat-model", displayName: "refreshed", contextWindow: 64000, maxOutputTokens: null, endpointTypes: ["chat_completions"], included: false, metadataSource: "upstream", protocol: "chatCompletions", supportedReasoningLevels: [{ effort: "low", description: "Provider low" }], defaultReasoningLevel: "low" },
    { id: "inherited-model", displayName: "inherited-model", contextWindow: null, maxOutputTokens: null, endpointTypes: [], included: true, metadataSource: "upstream", protocol: "responses", supportedReasoningLevels: [], defaultReasoningLevel: null },
  ]));
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.deepEqual(await page.evaluate(() => ({ protocol: window.review.saved.protocol, model: window.review.saved.model,
    overrides: Object.fromEntries(window.review.saved.models.map(model => [model.id, model.protocol ?? null])),
    chatIncluded: window.review.saved.models.find(model => model.id === "chat-model").included })),
  { protocol: "anthropicMessages", model: "chat-model", overrides: { "chat-model": "responses", "inherited-model": null, "responses-model": "responses", "anthropic-model": "anthropicMessages" }, chatIncluded: true });
  console.log("PASS model protocols inherit independently, cross-protocol defaults work, and refresh preserves overrides");
  await page.locator("#reopen-saved").click();
  await page.getByRole("combobox", { name: labels.chatProtocol, exact: true }).click();
  const inheritOption = page.getByRole("option").filter({ hasText: /^Automatic:|^自动：/ });
  assert.ok((await inheritOption.textContent()).includes(labels.anthropic), "inherit option must show provider default, not model override");
  await inheritOption.click();
  await page.locator("#newapi-provider-protocol").getByRole("radio", { name: labels.responses, exact: true }).click();
  assert.ok((await page.getByRole("combobox", { name: labels.chatProtocol, exact: true }).textContent()).includes(labels.responses));
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => window.review.saved.models.find(model => model.id === "chat-model").protocol === null);
  assert.equal(await page.evaluate(() => window.review.saved.protocol), "responses");
  console.log("PASS a per-model protocol override can be cleared and follows later provider default changes");
  assert.deepEqual(await page.evaluate(() => window.review.saved.models.slice(0, 2).map(model => ({
    levels: model.supportedReasoningLevels, default: model.defaultReasoningLevel,
  }))), [
    { levels: [{ effort: "low", description: "Provider low" }], default: "low" },
    { levels: [], default: null },
  ]);
  const refreshedReasoning = await page.evaluate(() => {
    const previous = { ...window.review.saved.models[0], overrides: { contextWindow: 200000 } };
    const fetched = { ...previous, metadataSource: "upstream", contextWindow: 64000, supportedReasoningLevels: [], defaultReasoningLevel: null };
    const refreshed = window.review.mergeModelCatalog([previous], [fetched])[0];
    return { context: refreshed.contextWindow, override: refreshed.overrides.contextWindow, levels: refreshed.supportedReasoningLevels, default: refreshed.defaultReasoningLevel };
  });
  assert.deepEqual(refreshedReasoning, { context: 64000, override: 200000, levels: [], default: null });
  console.log("PASS provider reasoning survives fetch, refresh, manual context overrides and the save payload, including explicit empty levels");

  await reset("access");
  await page.getByText(labels.modelCount, { exact: true }).waitFor();
  const codexAccessCard = page.locator('[aria-labelledby="newapi-routing-title"]');
  await page.evaluate(() => { window.review.navigatedTo = null; });
  await codexAccessCard.locator("[data-relay-link]").click();
  assert.equal(await page.evaluate(() => window.review.navigatedTo), "relays", "the Codex target node links to the relay models page");
  const selectedResponsesModel = await page.evaluate(() => window.review.t("newapi.accessSelectedModel", {
    provider: window.review.getState().providers[0].name,
    model: "responses-model", protocol: window.review.t("newapi.protocolShortResponses"),
  }));
  const relayTarget = codexAccessCard.locator('[data-codex-target="relay"]');
  const relayTargetLabel = await page.evaluate(() => window.review.t("newapi.providersSection"));
  assert.equal(await relayTarget.innerText(), relayTargetLabel);
  assert.equal(await codexAccessCard.getByText(selectedResponsesModel, { exact: true }).count(), 0,
    "router mode represents the model catalog rather than its pinned default");
  assert.equal(await page.getByRole("switch", { name: labels.accountless, exact: true }).count(), 1);
  assert.equal(await page.getByRole("switch", { name: labels.accountless, exact: true }).isEnabled(), true);
  await page.getByRole("switch", { name: labels.accountless, exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.accountless);
  assert.equal(await page.evaluate(() => window.review.getState().router.enabled), false);
  await page.getByRole("switch", { name: labels.accountless, exact: true }).click();
  await page.waitForFunction(() => !window.review.getState().router.accountless);
  await page.locator("#codex-access-mode").click();
  assert.equal(await page.getByRole("option", { name: labels.accessNative, exact: true }).isEnabled(), true);
  await page.getByRole("option", { name: labels.accessNative, exact: true }).click();
  await page.getByText(labels.nativeNoRouter, { exact: true }).waitFor();
  assert.equal(await codexAccessCard.getByText(selectedResponsesModel, { exact: true }).count(), 1);
  const nativeTarget = codexAccessCard.locator('[data-codex-target="native"]');
  assert.match(await nativeTarget.innerText(), /Provider mixed/);
  assert.match(await nativeTarget.innerText(), /responses-model/);
  assert.equal(await page.getByRole("switch", { name: labels.accountless, exact: true }).count(), 0);
  assert.equal(await page.getByText(labels.nativeAuthHint, { exact: true }).count(), 1);
  await page.getByRole("button", { name: labels.restartCodex, exact: true }).click();
  await page.getByRole("heading", { name: labels.restartConfirmTitle, exact: true }).waitFor();
  await page.getByRole("button", { name: labels.restartCodex, exact: true }).last().click();
  await page.waitForFunction(() => window.review.restartCalls.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.restartCalls), ["codex"]);
  await page.getByRole("switch", { name: labels.codexAccess, exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.enabled);
  assert.deepEqual(await page.evaluate(() => ({ modes: window.review.accessCalls,
    protocol: window.review.getState().providers[0].protocol, model: window.review.getState().providers[0].model,
    network: window.review.getState().providers[0].networkMode })),
  { modes: ["native"], protocol: "chatCompletions", model: "responses-model", network: "direct" });
  await page.locator("#codex-access-mode").click();
  await page.getByRole("option", { name: labels.accessRouter, exact: true }).click();
  await page.getByRole("switch", { name: labels.accountless, exact: true }).waitFor();

  await reset("incompatible");
  await relayTarget.waitFor();
  assert.equal(await relayTarget.innerText(), relayTargetLabel,
    "multiple enabled relays retain one abstract model-catalog target");
  assert.equal(await codexAccessCard.locator('[data-router-relay]').count(), 0);
  assert.doesNotMatch(await relayTarget.innerText(), /Provider mixed|Provider alternate|chat-model/);
  assert.equal(await page.getByText(labels.nativeProtocolHint, { exact: true }).count(), 0);
  await page.locator("#codex-access-mode").click();
  assert.equal(await page.getByRole("option", { name: labels.accessNative, exact: true }).getAttribute("aria-disabled"), "true");
  assert.deepEqual(await page.evaluate(() => window.review.accessCalls), []);
  assert.equal(await page.evaluate(() => window.review.getState().providers[0].model), "chat-model");
  await reset("native-invalid");
  await page.getByText(labels.nativeProtocolHint, { exact: true }).waitFor();
  assert.equal(await page.getByRole("switch", { name: labels.codexAccess, exact: true }).isDisabled(), true);
  assert.equal(await page.getByRole("switch", { name: labels.accountless, exact: true }).count(), 0);
  console.log("PASS Codex access mode is independent of protocol/network and rejects incompatible native selection");

  await reset("access");
  await page.evaluate(() => {
    window.review.aggregateModels = [{ id: "codex-target-pool", publicModelId: "public-model",
      displayName: "Codex target pool", enabled: true,
      members: [{ id: "target-member", providerId: "mixed", actualModelId: "responses-model",
        enabled: true, priority: 0, weight: 1 }],
      strategy: "roundRobin", affinity: "session", retryPolicy: { maxAttempts: 2, failureThreshold: 3 },
      compression: { mode: "inherit" } }];
    window.review.routerPatch = { aggregateModelId: "codex-target-pool" };
    window.review.mount("access");
  });
  const aggregateTarget = codexAccessCard.locator('[data-codex-target="aggregate"]');
  await aggregateTarget.waitFor();
  assert.equal(await aggregateTarget.innerText(), await page.evaluate(() => window.review.t("aggregation.target")));
  assert.equal(await codexAccessCard.locator("[data-relay-link]").count(), 0,
    "the aggregation marker must not navigate to an unrelated relay page");
  assert.doesNotMatch(await aggregateTarget.innerText(), /Codex target pool|public-model|Provider mixed/);
  assert.equal(await codexAccessCard.getByText(selectedResponsesModel, { exact: true }).count(), 0);
  await codexAccessCard.locator("#codex-aggregate-model").click();
  const ordinaryRelayOption = await page.evaluate(() => window.review.t("aggregation.singleChannel"));
  await page.getByRole("option", { name: ordinaryRelayOption, exact: true }).click();
  await relayTarget.waitFor();
  assert.equal(await relayTarget.innerText(), relayTargetLabel);
  console.log("PASS Codex routing uses distinct abstract relay-model and aggregation targets while native access keeps its destination");

  await reset("access");
  await page.getByRole("button", { name: "刷新余额", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceRequests.length === 1);
  assert.equal(await page.getByRole("switch", { name: labels.codexAccess, exact: true }).isEnabled(), true);
  await page.getByRole("switch", { name: labels.codexAccess, exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.enabled);
  await page.getByRole("button", { name: "导入 / 导出", exact: true }).click();
  assert.equal(await page.getByRole("menuitem", { name: "导出中转", exact: true }).isEnabled(), true);
  await page.keyboard.press("Escape");
  await page.evaluate(() => window.review.balanceResolvers[0]());
  await page.getByText("$9").waitFor();
  console.log("PASS balance refresh stays usable alongside routing and export controls");

  // Threads keep their provider: turning routing off is one call, no dialog.
  await page.evaluate(() => { window.review.routingCalls = []; });
  await page.getByRole("switch", { name: labels.codexAccess, exact: true }).click();
  await page.waitForFunction(() => window.review.routingCalls.length === 1);
  assert.equal(await page.getByRole("alertdialog").count(), 0);
  assert.equal(await page.evaluate(() => window.review.getState().router.enabled), false);
  assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [{ operation: "enabled", enabled: false }]);
  console.log("PASS turning routing off is a single backend call without thread migration prompts");

  await reset("test");
  await page.getByRole("combobox").waitFor();
  await page.getByRole("button", { name: labels.refreshModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "responses-model", displayName: "responses-model", contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream", protocol: "chatCompletions" },
  ]));
  await page.getByRole("combobox").selectOption("responses-model");
  assert.match(await page.getByRole("option", { name: /responses-model/ }).textContent(), /responses/);
  await page.getByRole("combobox").selectOption("anthropic-model");
  await page.getByRole("button", { name: labels.sendTest, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.tested));
  assert.deepEqual(await page.evaluate(() => window.review.tested), { id: "mixed", model: "anthropic-model" });
  assert.equal(await page.getByRole("button", { name: labels.sendTest, exact: true }).isDisabled(), true);
  await page.getByText("HTTP 200", { exact: true }).waitFor();
  await page.evaluate(() => window.review.finishProbe());
  await page.getByRole("log").getByText("Hello from the streaming model.").waitFor();
  assert.equal(await page.getByRole("dialog").isVisible(), true);
  if (captureDir) {
    await page.screenshot({ path: path.join(captureDir, "stream-probe.png") });
    await page.setViewportSize({ width: 960, height: 640 });
    await page.evaluate(() => document.documentElement.classList.add("dark"));
    await page.screenshot({ path: path.join(captureDir, "stream-probe-dark.png") });
    await page.evaluate(() => document.documentElement.classList.remove("dark"));
    await page.setViewportSize({ width: 1200, height: 700 });
  }
  await page.getByRole("button", { name: labels.sendTest, exact: true }).click();
  await page.evaluate(() => window.review.failProbe());
  await page.getByRole("alert").waitFor();
  assert.equal(await page.getByRole("button", { name: labels.sendTest, exact: true }).isEnabled(), true);
  console.log("PASS connectivity tests stream real stages, retain results and recover after timeout");

  for (const [width, height, action] of [
    [1200, 700, "close"], [995, 640, "cancel"], [390, 560, "escape"], [640, 320, "apply"],
  ]) {
    await page.setViewportSize({ width, height });
    await reset("recovery-long");
    const recovery = page.getByRole("alertdialog");
    const apply = recovery.getByRole("button", { name: labels.recoveryConfirm, exact: true });
    await apply.waitFor();
    const layout = await recovery.evaluate(element => {
      const rect = element.getBoundingClientRect();
      const buttons = Array.from(element.querySelectorAll("button"));
      return {
        withinViewport: rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight,
        noHorizontalOverflow: element.scrollWidth <= element.clientWidth &&
          Array.from(element.querySelectorAll("div")).every(child => child.scrollWidth <= child.clientWidth + 1),
        buttonsReachable: buttons.every(button => {
          const box = button.getBoundingClientRect();
          return box.top >= rect.top && box.bottom <= rect.bottom && box.left >= rect.left && box.right <= rect.right &&
            button.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
        }),
      };
    });
    assert.deepEqual(layout, { withinViewport: true, noHorizontalOverflow: true, buttonsReachable: true }, `${width}x${height} recovery layout`);
    const body = recovery.locator(":scope > .overflow-y-auto");
    assert.equal(await body.evaluate(element => element.scrollHeight > element.clientHeight), true);
    const before = await apply.boundingBox();
    await body.evaluate(element => { element.scrollTop = element.scrollHeight; });
    const after = await apply.boundingBox();
    assert.equal(after.y, before.y, "recovery actions stay fixed while the details scroll");
    if (captureDir) await page.screenshot({ path: path.join(captureDir, `thread-recovery-${width}x${height}.png`), animations: "disabled" });
    if (action === "escape") await page.keyboard.press("Escape");
    else await recovery.getByRole("button", {
      name: action === "apply" ? labels.recoveryConfirm : action === "cancel" ? labels.cancel : labels.close, exact: true,
    }).click();
    await recovery.waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.review.recoveryApplyCalls), action === "apply" ? 1 : 0);
  }
  console.log("PASS long recovery titles and paths fit desktop/narrow/short windows; close, cancel, Escape and repair remain reachable");

  await page.setViewportSize({ width: 1200, height: 700 });
  await reset("recovery-long");
  const recovery = page.getByRole("alertdialog");
  const recoveryFailure = "Synthetic recovery failed: active writer process C:/synthetic/" + "busy-writer-".repeat(50);
  await page.evaluate(message => {
    window.review.holdRecoveryApply = true;
    window.review.recoveryApplyError = message;
  }, recoveryFailure);
  await recovery.getByRole("button", { name: labels.recoveryConfirm, exact: true }).click();
  assert.equal(await recovery.getByRole("button", { name: labels.recoveryApplying, exact: true }).isDisabled(), true);
  assert.equal(await recovery.getByRole("button", { name: labels.close, exact: true }).isDisabled(), true);
  await page.keyboard.press("Escape");
  assert.equal(await recovery.count(), 1, "pending repairs cannot be dismissed mid-write");
  await page.evaluate(() => window.review.finishRecoveryApply());
  await recovery.getByRole("button", { name: labels.recoveryConfirm, exact: true }).waitFor();
  assert.equal(await recovery.getByRole("alert").textContent(), recoveryFailure);
  await page.setViewportSize({ width: 390, height: 320 });
  assert.equal(await recovery.getByRole("alert").evaluate(element => element.scrollHeight > element.clientHeight), true);
  for (const name of [labels.close, labels.cancel, labels.recoveryConfirm]) {
    const rect = await recovery.getByRole("button", { name, exact: true }).boundingBox();
    assert.ok(rect && rect.y >= 0 && rect.y + rect.height <= 320, `long repair errors must not hide ${name}: ${JSON.stringify(rect)}`);
  }
  await page.evaluate(() => { window.review.holdRecoveryApply = false; window.review.recoveryApplyError = null; });
  await recovery.getByRole("button", { name: labels.recoveryConfirm, exact: true }).click();
  await recovery.waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.review.recoveryApplyCalls), 2);
  assert.deepEqual(await page.evaluate(() => window.review.recoveryApplyArguments), { threadId: "t0", revision: "recovery-revision" });
  console.log("PASS recovery errors survive preview refresh, permit retry, and keep in-flight writes protected");

  await reset("recovery-router");
  await page.getByText(labels.routerAdoptionDescription, { exact: true }).waitFor();
  const adopt = page.getByRole("button", { name: labels.routerAdoptionConfirm, exact: true });
  const adoptRect = await adopt.boundingBox();
  assert.ok(adoptRect && adoptRect.y >= 0 && adoptRect.y + adoptRect.height <= 320,
    "one-time adoption action must remain reachable in a short viewport");
  await adopt.click();
  await page.waitForFunction(() => window.review.routerAdoptionCalls === 1 && window.review.recoveredCalls === 1);
  assert.deepEqual(await page.evaluate(() => window.review.recoveryApplyArguments), { threadId: "t0", revision: "recovery-revision" });
  console.log("PASS Router adoption explicitly confirms restart and uses the dedicated command with the frozen revision");

  for (const flag of ["recoveryLaunchWarning", "recoveryRefreshError"]) {
    await reset("recovery-router");
    await page.evaluate(key => { window.review[key] = true; }, flag);
    await page.getByRole("button", { name: labels.routerAdoptionConfirm, exact: true }).click();
    await page.getByRole("alertdialog").waitFor({ state: "detached" });
    assert.equal(await page.evaluate(() => window.review.routerAdoptionCalls), 1);
    assert.equal(await page.evaluate(() => window.review.recoveredCalls), 1);
  }
  console.log("PASS successful adoption stays successful when reopening Codex or refreshing the list fails");

  await page.setViewportSize({ width: 1200, height: 700 });
  await reset("recovery-missing");
  await page.getByRole("heading", { name: labels.recoveryTitle, exact: true }).waitFor();
  await page.getByText(labels.recoveryMissingStatus, { exact: true }).waitFor();
  await page.getByText("model_provider", { exact: true }).waitFor();
  await page.getByRole("button", { name: labels.recoveryCatalogConfirm, exact: true }).click();
  await page.waitForFunction(() => window.review.recoveryApplyCalls === 1 && window.review.recoveredCalls === 1);
  console.log("PASS missing-catalog recovery exposes frozen fields and applies through the confirmed preview");

  await reset("recovery-journal");
  await page.getByText(labels.recoveryJournalStatus, { exact: true }).waitFor();
  await page.getByText(labels.recoveryCoordinationTitle, { exact: true }).waitFor();
  await page.getByRole("button", { name: labels.recoveryOpenDirectory, exact: true }).click();
  await page.waitForFunction(() => window.review.openedPath === "C:/synthetic/recovery");
  assert.equal(await page.getByRole("button", { name: labels.recoveryCatalogConfirm, exact: true }).count(), 0);
  console.log("PASS blocked recovery shows manual coordination details and opens the recovery directory");

  await reset("recovery-official");
  await page.getByText(labels.recoveryOfficialStatus, { exact: true }).waitFor();
  assert.equal(await page.getByText("Local metadata cannot identify the ChatGPT account that owns the server thread", { exact: true }).count(), 0);
  assert.equal(await page.getByRole("button", { name: labels.recoveryCatalogConfirm, exact: true }).count(), 0);
  assert.equal(await page.evaluate(() => window.review.recoveryApplyCalls), 0);
  console.log("PASS official threads disclose unverified server-account ownership and remain read-only");

  await reset("thread-queue-recovery");
  const queueRecoveryBanner = page.getByRole("status").filter({ hasText: "删除队列包含无法读取的记录" });
  await queueRecoveryBanner.getByRole("button", { name: labels.clearDeletionQueue, exact: true }).click();
  await queueRecoveryBanner.waitFor({ state: "detached" });
  console.log("PASS a damaged pending-deletion queue exposes a working metadata-only recovery action");

  await reset("thread-actions");
  assert.equal(await page.evaluate(() => window.review.recoveryApplyCalls), 0);
  await page.getByRole("checkbox", { name: "Review thread 0", exact: true }).click();
  await page.getByRole("button", { name: labels.deleteSelected, exact: true }).click();
  await confirmAlert(labels.delete);
  await page.waitForFunction(() => window.review.deleteCalls.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.deleteCalls[0]), ["t0"]);
  await page.getByRole("button", { name: labels.deleteThread, exact: true }).last().click();
  await confirmAlert(labels.delete);
  await page.waitForFunction(() => window.review.deleteCalls.length === 2);
  assert.deepEqual(await page.evaluate(() => window.review.deleteCalls[1]), ["t1"]);
  console.log("PASS thread page supports batch and single deletion");

  await reset("thread-actions");
  const threadCheckbox = page.getByRole("checkbox", { name: "Review thread 0", exact: true });
  await threadCheckbox.focus();
  await page.keyboard.press("Space");
  assert.equal(await threadCheckbox.getAttribute("aria-checked"), "true");
  await page.keyboard.press("Enter");
  assert.equal(await threadCheckbox.getAttribute("aria-checked"), "false");
  console.log("PASS thread selection works from the keyboard and exposes its checked state");

  await reset("thread-actions");
  await page.evaluate(() => { window.review.queueDeletes = true; });
  await page.getByRole("button", { name: labels.deleteThread, exact: true }).first().click();
  await confirmAlert(labels.delete);
  await page.getByText(/1 个会话等待删除/).waitFor();
  await page.screenshot({ path: path.join(screenshotDir, "pending-thread-delete.png"), animations: "disabled" });
  await page.getByRole("button", { name: "取消待删除", exact: true }).click();
  await page.getByText(/1 个会话等待删除/).waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => window.review.pendingDeletionKeys), []);
  console.log("PASS queued thread deletion is visible and can be cancelled from the page");

  await reset("clients-conflict");
  assert.equal(await page.getByRole("button", { name: labels.changeClient, exact: true }).isEnabled(), true);
  assert.equal(await page.getByRole("button", { name: labels.clientRestore, exact: true }).isEnabled(), true);
  await page.getByRole("button", { name: labels.changeClient, exact: true }).click();
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.getByRole("button", { name: labels.clientApplyConflict, exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.previewReplacesConflicts), true);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "client-conflict-preview.png"), animations: "disabled" });
  await page.getByRole("button", { name: labels.clientApplyConflict, exact: true }).click();
  await page.getByRole("dialog").waitFor({ state: "hidden" });
  assert.equal(await page.evaluate(() => window.review.appliedClient.replaceConflicts), true);
  assert.equal(await page.evaluate(() => window.review.releasedClientCount), 0);
  assert.equal(await page.getByRole("button", { name: labels.changeClient, exact: true }).isEnabled(), true);
  console.log("PASS conflicting client configuration opens directly and applies a confirmed preview without releasing management");

  for (const [mode, action, configure] of [
    ["clients-conflict", labels.clientRestore, labels.configure],
    ["clients-conflict", labels.clientRebaseline, labels.changeClient],
    ["clients-existing", labels.clientReapply, labels.changeClient],
  ]) {
    await reset(mode);
    await page.getByRole("button", { name: action, exact: true }).click();
    await confirmAlert(action);
    const configureButton = page.getByRole("button", { name: configure, exact: true });
    await configureButton.waitFor();
    assert.equal(await configureButton.isEnabled(), true);
    await configureButton.click();
    await page.getByRole("button", { name: labels.preview, exact: true }).click();
    await page.getByRole("button", { name: labels.clientApply, exact: true }).click();
    await page.getByRole("dialog").waitFor({ state: "hidden" });
    assert.equal(await page.evaluate(() => window.review.appliedClient.replaceConflicts), false);
    assert.equal(await page.evaluate(() => window.review.releasedClientCount), 0);
    assert.equal(await page.getByRole("button", { name: labels.changeClient, exact: true }).isEnabled(), true);
  }
  console.log("PASS restore, rebaseline and reapply all leave client configuration directly usable without releasing management");
  await testClaudeCliOfficial({ page, reset, captureDir });

  await reset("clients");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  const screenshots = path.join(screenshotDir, "client-config");
  await mkdir(screenshots, { recursive: true });
  const screenshotClient = async (name, scrollTop = 0) => {
    await page.getByRole("listbox").waitFor({ state: "hidden" });
    await page.getByRole("dialog").evaluate(async (element, scrollTop) => {
      element.scrollTop = 0;
      element.querySelector("[data-client-config-scroll]")?.scrollTo(0, scrollTop);
      await document.fonts.ready;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    }, scrollTop);
    await page.screenshot({ path: path.join(screenshots, name), animations: "disabled" });
  };
  assert.equal(await page.getByRole("button", { name: labels.clientNative, exact: true }).getAttribute("aria-pressed"), "true");
  assert.equal(await page.locator("#client-model").textContent(), "anthropic-model");
  await page.locator("#client-model").click();
  assert.equal(await page.getByRole("option", { name: "responses-model", exact: true }).count(), 0);
  await page.keyboard.press("Escape");
  await screenshotClient("claude-native-light.png");
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.previewed));
  assert.deepEqual(await page.evaluate(() => ({ mode: window.review.previewed.accessMode, ids: window.review.previewed.providerIds, model: window.review.previewed.model })),
    { mode: "native", ids: ["mixed"], model: "anthropic-model" });
  await page.getByRole("button", { name: labels.clientRouter, exact: true }).click();
  assert.equal(await page.getByRole("button", { name: labels.clientApply, exact: true }).count(), 0);
  await page.locator("#client-model").click();
  await page.getByRole("option", { name: "responses-model", exact: true }).click();
  await page.getByText(labels.clientAdvanced, { exact: true }).click();
  await page.locator("#client-role-sonnet").click();
  await page.getByRole("option", { name: "chat-model", exact: true }).click();
  await page.locator("#client-auth").click();
  await page.getByRole("option", { name: "ANTHROPIC_API_KEY", exact: true }).click();
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await screenshotClient("claude-router-dark.png");
  assert.equal(await page.getByRole("button", { name: labels.preview, exact: true }).evaluate(element => {
    const button = element.getBoundingClientRect();
    const dialog = element.closest('[role="dialog"]').getBoundingClientRect();
    return button.top >= dialog.top && button.bottom <= dialog.bottom && button.bottom <= innerHeight;
  }), true);
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.accessMode === "router");
  assert.equal(await page.evaluate(() => window.review.previewed.claudeModels.sonnet), "chat-model");
  assert.equal(await page.evaluate(() => window.review.previewed.authField), "apiKey");
  await page.getByRole("button", { name: labels.clientApply, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.appliedClient));
  assert.equal(await page.evaluate(() => window.review.appliedClient.request.model), "responses-model");
  assert.deepEqual(await page.evaluate(() => window.review.accessCalls), []);
  await reset("clients-existing");
  await page.getByRole("button", { name: labels.changeClient, exact: true }).click();
  assert.equal(await page.locator("#client-model").textContent(), "responses-model");
  assert.equal(await page.getByRole("button", { name: labels.clientRouter, exact: true }).getAttribute("aria-pressed"), "true");
  await page.getByText(labels.clientAdvanced, { exact: true }).click();
  assert.equal(await page.locator("#client-role-sonnet").textContent(), "chat-model");
  assert.equal(await page.locator("#client-auth").textContent(), "ANTHROPIC_API_KEY");
  console.log("PASS Claude defaults to native, filters models, supports independent routing, invalidates preview and restores advanced choices");

  await reset("clients-versions");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  const versionIds = ["claude-opus-5-5", "claude-fable-5-1", "claude-opus-4-8"];
  const selectedVersion = id => page.locator('input[data-client-model-id="' + id + '"]');
  for (const id of versionIds) assert.equal(await selectedVersion(id).isChecked(), true);
  assert.ok((await page.locator("#client-role-opus").textContent()).includes("Opus 5.5"));
  assert.ok((await page.locator("#client-role-fable").textContent()).includes("Fable 5.1"));
  await page.locator("#client-role-opus").click();
  await page.getByRole("option", { name: "Opus 4.8", exact: true }).click();
  await screenshotClient("claude-models-light.png");
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await screenshotClient("claude-models-dark.png", Number.MAX_SAFE_INTEGER);
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.claudeModelIds?.length === 3);
  assert.deepEqual(await page.evaluate(() => window.review.previewed.claudeModelIds), versionIds);
  assert.equal(await page.evaluate(() => window.review.previewed.claudeModels.opus), "claude-opus-4-8");
  await page.locator("#client-model").click();
  await page.getByRole("option", { name: "Opus 4.8", exact: true }).click();
  assert.equal(await page.getByRole("button", { name: labels.clientApply, exact: true }).count(), 0);
  await selectedVersion("claude-opus-4-8").uncheck();
  assert.equal(await page.locator("#client-model").textContent(), "Opus 5.5");
  assert.ok((await page.locator("#client-role-opus").textContent()).includes("Opus 5.5"));
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.claudeModelIds?.length === 2);
  assert.deepEqual(await page.evaluate(() => window.review.previewed.claudeModelIds), versionIds.slice(0, 2));
  assert.equal(await page.evaluate(() => window.review.previewed.claudeModels.opus), "");
  await selectedVersion("claude-opus-5-5").uncheck();
  assert.equal(await page.locator("#client-model").textContent(), "Fable 5.1");
  assert.equal(await selectedVersion("claude-fable-5-1").isDisabled(), false);
  await page.getByRole("button", { name: labels.clientDeselectAllModels, exact: true }).click();
  for (const id of versionIds) assert.equal(await selectedVersion(id).isChecked(), false);
  assert.equal(await page.getByRole("button", { name: labels.preview, exact: true }).isDisabled(), true);
  await selectedVersion("claude-fable-5-1").check();
  await page.getByRole("button", { name: labels.clientSelectAllModels, exact: true }).click();
  for (const id of versionIds) assert.equal(await selectedVersion(id).isChecked(), true);
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.claudeModelIds?.length === 3 && window.review.previewed.model === "claude-fable-5-1");
  await page.getByRole("button", { name: labels.clientApply, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.appliedClient));
  assert.deepEqual(await page.evaluate(() => window.review.appliedClient.request.claudeModelIds), versionIds);

  await reset("clients-versions-existing");
  await page.getByRole("button", { name: labels.changeClient, exact: true }).click();
  assert.equal(await selectedVersion("claude-opus-5-5").isChecked(), true);
  assert.equal(await selectedVersion("claude-opus-4-8").isChecked(), true);
  assert.equal(await selectedVersion("claude-fable-5-1").isChecked(), false);
  assert.equal(await page.locator("#client-model").textContent(), "Opus 4.8");
  assert.equal(await page.locator("#client-role-opus").textContent(), "Opus 4.8");
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.model === "claude-opus-4-8");
  assert.deepEqual(await page.evaluate(() => window.review.previewed.claudeModelIds), [versionIds[0], versionIds[2]]);

  await reset("clients-versions");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await selectedVersion("claude-fable-5-1").uncheck();
  await page.getByRole("button", { name: labels.clientRouter, exact: true }).click();
  assert.equal(await selectedVersion("claude-fable-5-1").isChecked(), false);
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.claudeModelIds?.length === 2);
  assert.deepEqual(await page.evaluate(() => window.review.previewed.claudeModelIds), versionIds.filter(id => id !== "claude-fable-5-1"));
  console.log("PASS explicit Claude model subsets survive access-mode changes");

  await reset("clients-versions");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await page.getByRole("button", { name: labels.clientRouter, exact: true }).click();
  // The fixture only has Opus and Fable models. Give the remaining roles a
  // model before testing their 1M switches, which correctly disable otherwise.
  for (const role of ["sonnet", "haiku", "subagent"]) {
    await page.locator("#client-role-" + role).click();
    await page.getByRole("option", { name: "Opus 5.5", exact: true }).click();
  }
  const startupOneM = page.getByRole("checkbox", { name: labels.oneMStartup, exact: true });
  const oneMCheckboxes = [labels.oneMStartup, ...labels.oneMRoles].map(name => page.getByRole("checkbox", { name, exact: true }));
  // [1m] marks follow the resolver: models without catalog context default to
  // unmarked, and the explicit opt-in decides and survives model/mode changes.
  for (const checkbox of oneMCheckboxes) assert.equal(await checkbox.isChecked(), false);
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.claudeOneM?.startup === false);
  // Limits stay automatic unless the user types them: no family-derived 1M.
  assert.equal(await page.evaluate(() => window.review.previewed.contextWindow ?? null), null);
  assert.equal(await page.evaluate(() => window.review.previewed.claudeOneM.sonnet), false);
  for (const checkbox of oneMCheckboxes) await checkbox.check();
  await page.locator("#client-model").click();
  await page.getByRole("option", { name: "Fable 5.1", exact: true }).click();
  assert.equal(await startupOneM.isChecked(), true);
  await page.getByRole("button", { name: labels.clientNative, exact: true }).click();
  await page.getByRole("button", { name: labels.clientRouter, exact: true }).click();
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.model === "claude-fable-5-1");
  assert.equal(await page.evaluate(() => window.review.previewed.claudeOneM.startup), true);
  assert.equal(await page.evaluate(() => Object.values(window.review.previewed.claudeOneM).every(value => value === true)), true);
  for (const checkbox of oneMCheckboxes) await checkbox.uncheck();
  await selectedVersion("claude-fable-5-1").uncheck();
  for (const checkbox of oneMCheckboxes) assert.equal(await checkbox.isChecked(), false);
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.getByRole("button", { name: labels.clientApply, exact: true }).click();
  await page.getByRole("button", { name: labels.changeClient, exact: true }).click();
  for (const checkbox of oneMCheckboxes) assert.equal(await checkbox.isChecked(), false);
  console.log("PASS Claude 1M choices default to unmarked without catalog data and explicit on/off values survive model/mode/subset changes, apply and reopen");

  const defaultChecks = await page.evaluate(() => {
    const { clientDefaults, claudeOneMDefaults, mergeModelCatalog } = window.review;
    const id = "claude-haiku-4-5";
    const manual = { id, displayName: id, contextWindow: null, maxOutputTokens: null, overrides: { contextWindow: 200000, maxOutputTokens: 4096 }, metadataSource: "local", included: true, endpointTypes: [] };
    const fetched = { ...manual, contextWindow: 1000000, maxOutputTokens: 64000, metadataSource: "local" };
    const merged = mergeModelCatalog([manual], [fetched])[0];
    return {
      haiku: clientDefaults(undefined, id).contextWindow,
      oldClaude: clientDefaults(undefined, "claude-sonnet-4").contextWindow,
      other: clientDefaults(undefined, "unknown-model").contextWindow,
      manual: clientDefaults({ model: id, models: [merged] }, id),
      legacy: claudeOneMDefaults({ startup: false, sonnet: false, opus: false, fable: false, subagent: false }),
    };
  });
  assert.deepEqual(defaultChecks, { haiku: null, oldClaude: null, other: null,
    manual: { contextWindow: 200000, maxOutputTokens: 4096 },
    legacy: { startup: false, sonnet: false, opus: false, fable: false, haiku: false, subagent: false } });
  console.log("PASS limits stay automatic without model-name guesses, saved values survive catalog refresh, old profile false values survive adding Haiku");

  const staleProfile = { id: "stale-profile", name: "Stale profile", request: {
    targetId: "claudeCode", providerId: "versions", model: "claude-opus-5-5", contextWindow: 200000,
    maxOutputTokens: 8192, accessMode: "router", providerIds: ["versions"],
    claudeModelIds: ["claude-opus-5-5", "removed-model"],
    claudeModels: { sonnet: "", opus: "", fable: "", haiku: "", subagent: "" },
  } };
  await page.evaluate(profile => localStorage.setItem("agentharbor.claude-code-profiles.v1", JSON.stringify([profile])), staleProfile);
  await reset("clients-versions");
  await page.evaluate(() => { window.review.appliedClient = null; });
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await page.locator("#claude-saved-profile").click();
  await page.getByRole("option", { name: "Stale profile · Provider versions", exact: true }).click();
  await page.getByRole("alert").filter({ hasText: labels.profileModelsUnavailable }).waitFor();
  assert.equal(await page.evaluate(() => window.review.appliedClient), null);
  await page.evaluate(() => localStorage.removeItem("agentharbor.claude-code-profiles.v1"));
  console.log("PASS a saved Claude profile with removed models is blocked without applying a changed selection");

  await page.setViewportSize({ width: 390, height: 844 });
  await reset("clients-versions");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  assert.equal(await page.getByRole("dialog").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await screenshotClient("claude-models-narrow.png");
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS Claude configures multiple versions together, repairs removed defaults and roles, and restores selected models");

  await reset("clients-versions");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await page.locator("#client-model").click();
  await page.getByRole("option", { name: "Fable 5.1", exact: true }).click();
  for (const checkbox of oneMCheckboxes) await checkbox.uncheck();
  await page.getByRole("button", { name: "保存当前配置", exact: true }).click();
  await page.getByRole("textbox", { name: "配置名称" }).fill("Regression profile");
  await page.getByRole("button", { name: "保存配置", exact: true }).click();
  await page.getByRole("button", { name: labels.cancel, exact: true }).click();
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await screenshotClient("claude-saved-profile-light.png");
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.getByRole("dialog").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await screenshotClient("claude-saved-profile-narrow.png");
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.evaluate(() => { window.review.appliedClient = null; });
  await page.locator("#claude-saved-profile").click();
  await page.getByRole("option", { name: "Regression profile · Provider versions", exact: true }).click();
  // Loading a profile only previews it; nothing is written until Apply.
  await page.getByRole("button", { name: labels.clientApply, exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.appliedClient), null);
  for (const checkbox of oneMCheckboxes) assert.equal(await checkbox.isChecked(), false);
  await page.getByRole("button", { name: labels.clientApply, exact: true }).click();
  await page.waitForFunction(() => window.review.appliedClient?.request.model === "claude-fable-5-1");
  assert.equal(await page.evaluate(() => Object.values(window.review.appliedClient.request.claudeOneM).every(value => value === false)), true);
  assert.equal(await page.getByRole("dialog").count(), 0);
  await reset("clients-versions");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await page.getByRole("button", { name: labels.manageProfiles, exact: true }).click();
  await page.getByRole("button", { name: labels.deleteProfile, exact: true }).click();
  await confirmAlert(labels.delete);
  await page.waitForFunction(() => JSON.parse(localStorage.getItem("agentharbor.claude-code-profiles.v1") ?? "[]").length === 0);
  console.log("PASS saved Claude profiles apply, can be deleted, and stale entries remain blocked");

  await reset("clients-opencode");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await page.locator("details").filter({ hasText: "Provider mixed" }).locator("summary").click();
  for (const id of ["chat-model", "anthropic-model", "inherited-model"]) {
    await page.getByRole("checkbox", { name: "配置模型 " + id }).uncheck();
  }
  await screenshotClient("opencode-model-subset-light.png");
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.getByRole("dialog").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await screenshotClient("opencode-model-subset-narrow.png");
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.getByRole("button", { name: labels.clientRouter, exact: true }).click();
  await page.getByRole("button", { name: labels.clientNative, exact: true }).click();
  await page.getByRole("button", { name: /Provider alternate/ }).click();
  await page.getByRole("button", { name: /Provider mixed/ }).click();
  await page.getByRole("button", { name: /Provider mixed/ }).click();
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.providerModelIds?.mixed?.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.previewed.providerModelIds.mixed), ["responses-model"]);
  console.log("PASS OpenCode keeps the selected model subset across access mode changes");

  await reset("clients-opencode-stale");
  await page.evaluate(() => { window.review.previewed = null; });
  await page.getByRole("button", { name: labels.changeClient, exact: true }).click();
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.getByRole("alert").filter({ hasText: labels.openCodeModelsUnavailable }).waitFor();
  assert.equal(await page.evaluate(() => window.review.previewed), null);
  await page.locator("details").filter({ hasText: "Provider mixed" }).locator("summary").click();
  await page.getByRole("checkbox", { name: "配置模型 anthropic-model" }).check();
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.providerModelIds?.mixed?.includes("anthropic-model"));
  assert.deepEqual(await page.evaluate(() => window.review.previewed.providerModelIds.mixed), ["responses-model", "anthropic-model"]);
  console.log("PASS stale OpenCode model IDs require an explicit selection update before preview");

  await reset("clients-opencode");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  await page.getByRole("button", { name: /Provider alternate/ }).click();
  await page.locator("#client-model").click();
  await page.getByRole("option", { name: "Provider alternate / model-alternate", exact: true }).click();
  await page.getByText(labels.clientAdvanced, { exact: true }).click();
  await page.locator("#client-context").fill("64000");
  await page.locator("#client-output").fill("4096");
  await screenshotClient("opencode-cards-light.png");
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.previewed));
  assert.deepEqual(await page.evaluate(() => ({ mode: window.review.previewed.accessMode, ids: window.review.previewed.providerIds,
    provider: window.review.previewed.providerId, model: window.review.previewed.model, context: window.review.previewed.contextWindow })),
    { mode: "native", ids: ["mixed", "alternate"], provider: "alternate", model: "model-alternate", context: 64000 });
  await page.getByRole("button", { name: /Provider alternate/ }).click();
  assert.equal(await page.getByRole("button", { name: labels.clientApply, exact: true }).count(), 0);
  assert.equal(await page.locator("#client-model").textContent(), "responses-model");
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.providerIds.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.previewed.providerIds), ["mixed"]);
  console.log("PASS OpenCode selects multiple cards, remembers limits and replaces a removed default provider");

  await page.setViewportSize({ width: 390, height: 844 });
  await reset("clients");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  assert.equal(await page.getByRole("dialog").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await screenshotClient("claude-native-narrow.png");
  await page.setViewportSize({ width: 1200, height: 700 });

  await reset("image");
  await page.getByRole("button", { name: labels.galleryTitle, exact: true }).click();
  await page.waitForFunction(() => {
    const stage = document.querySelector('[data-page-scroll-container]');
    const gallery = document.querySelector('[data-gallery-section]');
    return stage && gallery && Math.abs(gallery.getBoundingClientRect().top - stage.getBoundingClientRect().top - 24) < 5;
  });
  await page.waitForFunction(() => window.review.thumbnailCalls.length > 0);
  await page.screenshot({ path: path.join(screenshotDir, "image-gallery-location.png"), animations: "disabled" });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.locator("[data-gallery-section]").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await page.locator("[data-gallery-section]").evaluate(element => {
    const stage = element.closest("[data-page-scroll-container]");
    stage.scrollTop += element.getBoundingClientRect().top - stage.getBoundingClientRect().top - 16;
  });
  await page.screenshot({ path: path.join(screenshotDir, "image-gallery-location-narrow.png"), animations: "disabled" });
  await page.setViewportSize({ width: 1200, height: 700 });
  assert.ok(await page.evaluate(() => window.review.thumbnailCalls.length < 100));
  assert.equal(await page.evaluate(() => window.review.originalCalls.length), 0);
  const galleryTop = await page.locator('button[title="gallery-0.png"]').evaluate(element =>
    element.getBoundingClientRect().top + element.closest("section").scrollTop);
  for (let offset = 0; offset <= 5000; offset += 400) {
    await page.locator("section").evaluate((element, scroll) => { element.scrollTop = scroll; }, galleryTop + offset);
    await page.waitForTimeout(60);
  }
  await page.locator('button[title="gallery-0.png"]').scrollIntoViewIfNeeded();
  await page.waitForFunction(() => window.review.thumbnailCalls.filter(path => path.endsWith("gallery-0.png")).length >= 2);
  assert.ok(await page.locator('img[alt^="gallery-"]').count() < 48);
  assert.equal(await page.evaluate(() => window.review.originalCalls.length), 0);
  await page.getByRole("button", { name: "收起画册", exact: true }).click();
  await page.evaluate(() => { window.review.holdThumbnails = true; });
  await page.getByRole("button", { name: labels.galleryTitle, exact: true }).click();
  await page.getByRole("heading", { name: labels.galleryTitle, exact: true }).scrollIntoViewIfNeeded();
  await page.waitForFunction(() => window.review.thumbnailResolvers.length > 0);
  const oldCallCount = await page.evaluate(() => window.review.thumbnailCalls.length);
  await page.getByRole("button", { name: "收起画册", exact: true }).click();
  await page.evaluate(() => { window.review.thumbnailResolvers.forEach(resolve => resolve()); window.review.holdThumbnails = false; });
  assert.equal(await page.locator('img[alt^="gallery-"]').count(), 0);
  await page.getByRole("button", { name: labels.galleryTitle, exact: true }).click();
  await page.getByRole("heading", { name: labels.galleryTitle, exact: true }).scrollIntoViewIfNeeded();
  await page.waitForFunction(count => window.review.thumbnailCalls.length > count, oldCallCount);
  await page.evaluate(() => { window.review.holdOriginal = true; });
  await page.locator('button[title="gallery-0.png"]').click();
  await page.waitForFunction(() => window.review.originalCalls.length === 1);
  await page.getByRole("button", { name: "收起画册", exact: true }).click();
  await page.evaluate(() => window.review.resolveOriginal());
  await page.waitForTimeout(50);
  assert.equal(await page.getByRole("dialog", { name: "成图放大预览", exact: true }).count(), 0);
  await page.evaluate(() => { window.review.holdOriginal = false; });
  await page.getByRole("button", { name: labels.galleryTitle, exact: true }).click();
  await page.locator('button[title="gallery-0.png"]').click();
  const galleryPreview = page.getByRole("dialog", { name: "成图放大预览", exact: true });
  await galleryPreview.waitFor();
  assert.deepEqual(await galleryPreview.evaluate(element => ({
    parentIsBody: element.parentElement === document.body,
    top: Math.round(element.getBoundingClientRect().top),
    height: Math.round(element.getBoundingClientRect().height),
    viewportHeight: window.innerHeight,
  })), { parentIsBody: true, top: 0, height: 700, viewportHeight: 700 });
  await page.getByRole("button", { name: "关闭预览" }).click();
  console.log("PASS gallery scrolls into view, previews above the page, and ignores late image responses");
  await testAccountThreadNavigation({ page, reset, captureDir });
  await testThreadDetails({ page, reset, captureDir });
  await testClaudeSessions({ page, reset, captureDir });
  await testClaudeSupport({ page, reset, captureDir });
  await testPiIntegration({ page, reset, captureDir });
  await testThreadUiFixes({ page, reset, captureDir });
  await testAggregation({ page, reset, captureDir });
  await testCodexSubagents({ page, reset, captureDir });
  await testMagpieOptimizations({ page, reset, captureDir });
  await testPriceCache({ page, reset, captureDir });
  await testCapabilityClear({ page, reset, captureDir });
  await testProviderFeedback({ page, reset, captureDir });
  await testProviderModelResolutions({ page, reset, captureDir });
  await testTerminalProxy({ page, reset, captureDir });
  await testCodexRoutingTransitions({ page, reset });
  await testRelayDeck({ page, reset, captureDir });
  await testUiRefresh({ page, reset, captureDir });
  await testClaudeDesktopCard({ page, reset, captureDir });
  await testSkillsActivation({ page, reset, captureDir });
  await reset("warnings");
  await page.getByText("Review thread 0", { exact: true }).waitFor();
  const warning = page.getByRole("status");
  await warning.locator("summary").click();
  assert.equal(await warning.getByText("Skipped synthetic-corrupt-session.jsonl: invalid UTF-8", { exact: true }).isVisible(), true);
  assert.equal(await page.getByText("Review thread 0", { exact: true }).isVisible(), true);
  console.log("PASS unreadable conversation warnings remain visible alongside usable conversations");
  }
  assert.deepEqual(errors, [], "Unexpected browser errors");
} finally {
  if (browser) await browser.close();
  await new Promise(resolve => server.close(resolve));
}
