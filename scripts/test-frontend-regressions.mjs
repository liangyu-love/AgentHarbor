// Run after npm run build. No real Tauri backend, profiles, or external network.
// Optional: --playwright-module=/path/to/playwright-core or PLAYWRIGHT_MODULE_PATH.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFile, readdir, stat } from "node:fs/promises";
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
let browser;
try {
  const executablePath = argument("--browser-executable") ?? process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH;
  browser = await playwright.chromium.launch({ headless: true, ...(executablePath ? { executablePath } : { channel: "chromium" }) });
  const page = await browser.newPage({ viewport: { width: 1200, height: 700 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
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
  const providerSelect = page.locator('select[aria-label="中转驿站供应商"]');
  await page.waitForFunction(() => document.querySelector('select[aria-label="中转驿站供应商"]')?.value === "image");
  await providerSelect.selectOption("");
  await page.locator('input[type="password"]').waitFor();
  assert.equal(await providerSelect.inputValue(), "");
  await page.locator('input[placeholder="https://example.com"]').fill("https://manual.invalid");
  await providerSelect.selectOption("image");
  await providerSelect.selectOption("");
  assert.equal(await page.locator('input[placeholder="https://example.com"]').inputValue(), "https://manual.invalid");
  console.log("PASS explicit manual image provider selection remains selected and preserves its address");

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

  await reset("clients");
  await page.getByRole("button", { name: labels.configure, exact: true }).click();
  assert.equal(await page.locator("#client-provider").inputValue(), "mixed");
  assert.equal(await page.locator("#client-model").inputValue(), "anthropic-model");
  await page.getByRole("button", { name: labels.preview, exact: true }).click();
  await page.waitForFunction(() => Boolean(window.review.previewed));
  assert.equal(await page.evaluate(() => window.review.previewed.model), "anthropic-model");
  await page.locator("#client-model").fill("responses-model");
  await page.getByText(labels.incompatibleClient, { exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: labels.preview, exact: true }).isDisabled(), true);
  await reset("clients-existing");
  await page.getByRole("button", { name: labels.changeClient, exact: true }).click();
  assert.equal(await page.locator("#client-model").inputValue(), "responses-model");
  assert.equal(await page.getByRole("button", { name: labels.preview, exact: true }).isDisabled(), true);
  console.log("PASS client configuration compatibility follows each selected model protocol");

  await reset("image");
  await page.getByRole("button", { name: labels.galleryTitle, exact: true }).click();
  await page.getByRole("heading", { name: labels.galleryTitle, exact: true }).scrollIntoViewIfNeeded();
  await page.waitForFunction(() => window.review.thumbnailCalls.length > 0);
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
  console.log("PASS gallery loads visible thumbnails with bounded caching and ignores responses after closing");
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
