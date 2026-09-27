// Run after npm run build. No real Tauri backend, profiles, or external network.
// Optional: --playwright-module=/path/to/playwright-core or PLAYWRIGHT_MODULE_PATH.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { mkdir, readFile, readdir, stat } from "node:fs/promises";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
  bundle: true, write: false, format: "iife", platform: "browser", alias: { "@": path.join(root, "src") },
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
const captureDir = process.env.FRONTEND_QA_CAPTURE_DIR;
if (captureDir) await mkdir(captureDir, { recursive: true });
let browser;
try {
  const executablePath = argument("--browser-executable") ?? process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
  browser = await playwright.chromium.launch({ headless: true, ...(executablePath ? { executablePath } : { channel: "chromium" }) });
  const page = await browser.newPage({ viewport: { width: 1200, height: 700 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("dialog", async dialog => { await dialog.accept(); });
  await page.route("**/*", route => route.request().url().startsWith(address + "/") ? route.continue() : route.abort());
  const reset = async mode => {
    await page.goto(address);
    await page.waitForFunction(() => Boolean(window.review));
    await page.evaluate(value => window.review.mount(value), mode);
  };

  await reset("compact");
  await page.getByText("Review thread 0", { exact: true }).waitFor();
  await page.locator("section").evaluate(element => { element.scrollTop = 2800; });
  await page.waitForFunction(() => [...document.querySelectorAll("p")].some(element => {
    const bounds = element.getBoundingClientRect();
    return /^Review thread [5-9][0-9]$/.test(element.textContent ?? "") && bounds.top < 600 && bounds.bottom > 0;
  }));
  assert.equal(await page.evaluate(() => window.scrollY), 0);
  await page.evaluate(() => window.review.mount("grid"));
  await page.locator("section").evaluate(element => { element.scrollTop = 1800; });
  await page.waitForFunction(() => [...document.querySelectorAll("h2")].some(element =>
    element.textContent === "Workspace 3" && element.closest("[aria-hidden]")?.getAttribute("aria-hidden") === "false"));
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
  console.log("PASS a failed gallery save remains on its canvas and can be retried");

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

  await reset("groups");
  await page.getByRole("heading", { name: "relay.invalid" }).waitFor();
  await page.getByRole("button", { name: "下一个站点配置" }).click();
  await page.locator(".relay-member-name", { hasText: "Relay Beta" }).waitFor();
  await page.getByText("beta-model", { exact: true }).waitFor();
  assert.equal(await page.locator("[data-relay-card-face]").first().evaluate(element => getComputedStyle(element).animationName), "relay-card-flip-next");
  const cardHeightsBefore = await page.locator(".group.relative.flex.min-w-0.flex-col").evaluateAll(cards => cards.map(card => card.getBoundingClientRect().height));
  await page.getByRole("button", { name: "展开同中转配置" }).click();
  const cardHeightsAfter = await page.locator(".group.relative.flex.min-w-0.flex-col").evaluateAll(cards => cards.map(card => card.getBoundingClientRect().height));
  assert.equal(cardHeightsBefore.length, 2);
  assert.ok(cardHeightsAfter[0] > cardHeightsBefore[0] + 40, JSON.stringify({ cardHeightsBefore, cardHeightsAfter }));
  assert.ok(Math.abs(cardHeightsAfter[1] - cardHeightsBefore[1]) < 2, JSON.stringify({ cardHeightsBefore, cardHeightsAfter }));
  if (captureDir) {
    await page.getByRole("heading", { name: "relay.invalid" }).evaluate(element => element.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: path.join(captureDir, "group-expanded.png") });
  }
  await page.getByRole("button", { name: "收起层叠堆叠" }).first().click();
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

  await reset("overview");
  await page.getByText("http://127.0.0.1:7890").waitFor();
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

  await reset("mixed");
  const chatProtocol = page.getByRole("combobox", { name: labels.chatProtocol, exact: true });
  assert.equal(await chatProtocol.textContent(), labels.inheritChat);
  assert.equal(await page.getByRole("button", { name: labels.defaultResponses, exact: true }).getAttribute("aria-pressed"), "true");
  assert.equal(await page.getByRole("button", { name: labels.defaultChat, exact: true }).isEnabled(), true);
  await chatProtocol.click();
  await page.getByRole("option", { name: labels.responses, exact: true }).click();
  await page.locator("#newapi-provider-protocol").click();
  await page.getByRole("option", { name: labels.anthropic, exact: true }).click();
  assert.equal(await page.getByRole("button", { name: labels.defaultResponses, exact: true }).getAttribute("aria-pressed"), "true");
  assert.equal(await chatProtocol.textContent(), labels.responses);
  assert.equal(await page.getByRole("combobox", { name: labels.inheritedProtocol, exact: true }).textContent(), labels.inheritAnthropic);
  await page.getByRole("button", { name: labels.defaultChat, exact: true }).click();
  await page.getByRole("button", { name: labels.fetchModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "chat-model", displayName: "refreshed", contextWindow: 64000, maxOutputTokens: null, endpointTypes: ["chat_completions"], included: false, metadataSource: "upstream", protocol: "chatCompletions" },
    { id: "inherited-model", displayName: "inherited-model", contextWindow: null, maxOutputTokens: null, endpointTypes: [], included: true, metadataSource: "upstream", protocol: "responses" },
  ]));
  await page.locator('button[type="submit"]').click();
  await page.waitForFunction(() => Boolean(window.review.saved));
  assert.deepEqual(await page.evaluate(() => ({ protocol: window.review.saved.protocol, model: window.review.saved.model,
    overrides: Object.fromEntries(window.review.saved.models.map(model => [model.id, model.protocol ?? null])),
    chatIncluded: window.review.saved.models.find(model => model.id === "chat-model").included })),
  { protocol: "anthropicMessages", model: "chat-model", overrides: { "chat-model": "responses", "inherited-model": null, "responses-model": "responses", "anthropic-model": "anthropicMessages" }, chatIncluded: true });
  console.log("PASS model protocols inherit independently, cross-protocol defaults work, and refresh preserves overrides");

  await reset("access");
  await page.getByText(labels.modelCount, { exact: true }).waitFor();
  assert.equal(await page.getByText("Responses", { exact: true }).count(), 1);
  assert.equal(await page.getByRole("switch", { name: labels.accountless, exact: true }).count(), 1);
  await page.locator("#codex-access-mode").click();
  assert.equal(await page.getByRole("option", { name: labels.accessNative, exact: true }).isEnabled(), true);
  await page.getByRole("option", { name: labels.accessNative, exact: true }).click();
  await page.getByText(labels.nativeNoRouter, { exact: true }).waitFor();
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
  await page.getByText(labels.nativeProtocolHint, { exact: true }).waitFor();
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

  await reset("test");
  await page.getByRole("button", { name: /anthropic-model/ }).waitFor();
  await page.getByRole("button", { name: labels.refreshModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "responses-model", displayName: "responses-model", contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream", protocol: "chatCompletions" },
  ]));
  await page.getByRole("button", { name: /responses-model/ }).click();
  assert.match(await page.getByRole("button", { name: /responses-model/ }).textContent(), /Responses/);
  await page.getByRole("button", { name: /anthropic-model/ }).click();
  await page.getByRole("button", { name: labels.sendTest, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.tested));
  assert.deepEqual(await page.evaluate(() => window.review.tested), { id: "mixed", model: "anthropic-model" });
  console.log("PASS connectivity tests include every protocol and preserve model overrides after refresh");

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

  await reset("thread-actions");
  await page.getByRole("button", { name: labels.repairAll, exact: true }).click();
  await page.waitForFunction(() => window.review.recoveryApplyCalls === 1);
  await page.getByText("Review thread 0", { exact: true }).click();
  await page.getByRole("button", { name: labels.deleteSelected, exact: true }).click();
  await page.waitForFunction(() => window.review.deleteCalls.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.deleteCalls[0]), ["t0"]);
  await page.getByRole("button", { name: labels.deleteThread, exact: true }).last().click();
  await page.waitForFunction(() => window.review.deleteCalls.length === 2);
  assert.deepEqual(await page.evaluate(() => window.review.deleteCalls[1]), ["t1"]);
  console.log("PASS thread page repairs all eligible threads and supports batch and single deletion");

  await reset("thread-actions");
  await page.evaluate(() => { window.review.queueDeletes = true; });
  await page.getByRole("button", { name: labels.deleteThread, exact: true }).first().click();
  await page.getByText(/1 个会话等待删除/).waitFor();
  await page.screenshot({ path: path.join(root, ".playwright-cli", "pending-thread-delete.png"), animations: "disabled" });
  await page.getByRole("button", { name: "取消待删除", exact: true }).click();
  await page.getByText(/1 个会话等待删除/).waitFor({ state: "hidden" });
  assert.deepEqual(await page.evaluate(() => window.review.pendingDeletionKeys), []);
  console.log("PASS queued thread deletion is visible and can be cancelled from the page");

  await reset("clients");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  const screenshots = path.join(root, ".playwright-cli", "client-config");
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
  assert.equal(await selectedVersion("claude-fable-5-1").isDisabled(), true);
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
  await page.waitForFunction(() => window.review.appliedClient?.request.model === "claude-fable-5-1");
  assert.equal(await page.getByRole("dialog").count(), 0);
  await page.evaluate(() => localStorage.removeItem("agentharbor.claude-code-profiles.v1"));
  console.log("PASS selecting a saved Claude profile previews and applies it immediately");

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
  await page.screenshot({ path: path.join(root, ".playwright-cli", "image-gallery-location.png"), animations: "disabled" });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.locator("[data-gallery-section]").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await page.locator("[data-gallery-section]").evaluate(element => {
    const stage = element.closest("[data-page-scroll-container]");
    stage.scrollTop += element.getBoundingClientRect().top - stage.getBoundingClientRect().top - 16;
  });
  await page.screenshot({ path: path.join(root, ".playwright-cli", "image-gallery-location-narrow.png"), animations: "disabled" });
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
  await reset("warnings");
  await page.getByText("Review thread 0", { exact: true }).waitFor();
  const warning = page.getByRole("status");
  await warning.locator("summary").click();
  assert.equal(await warning.getByText("Skipped synthetic-corrupt-session.jsonl: invalid UTF-8", { exact: true }).isVisible(), true);
  assert.equal(await page.getByText("Review thread 0", { exact: true }).isVisible(), true);
  console.log("PASS unreadable conversation warnings remain visible alongside usable conversations");
  assert.deepEqual(errors, [], "Unexpected browser errors");
} finally {
  if (browser) await browser.close();
  await new Promise(resolve => server.close(resolve));
}
