import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testProviderModelResolutions({ page, reset, captureDir }) {
  const row = id => page.locator("[data-model-row]").filter({ has: page.locator(`span[title="${id}"]`) });
  const context = id => row(id).getByRole("spinbutton").first();
  const catalog = id => row(id).locator('input[spellcheck="false"]').last();
  const inherit = id => row(id).locator('[data-model-resolution] select option[value="inherit"]');
  const digits = value => (value ?? "").replace(/\D/g, "");
  const ready = async (id = "model-A", kind = "exact") => {
    await row(id).locator(`[data-model-resolution="${kind}"]`).waitFor();
  };
  const labels = async () => page.evaluate(() => window.review.labels);
  const lastRequest = async () => page.evaluate(() => window.review.modelResolutionRequests.at(-1));
  const save = async () => {
    await page.locator('button[type="submit"]').click();
    await page.waitForFunction(() => Boolean(window.review.saved));
    return page.evaluate(() => window.review.saved);
  };

  await reset("provider");
  await ready();
  assert.equal(await context("model-A").inputValue(), "");
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "1000000");
  assert.equal(await catalog("model-A").inputValue(), "");
  assert.equal(await catalog("model-A").getAttribute("placeholder"), "fixture-official/model-A");
  assert.match(await inherit("model-A").innerText(), /90%/);
  assert.match(await inherit("model-A").innerText(), /720\s*K/i);
  const initialRequest = await lastRequest();
  assert.equal(initialRequest.providerId, "A");
  assert.equal(initialRequest.hasDraftApiKey, false);
  assert.equal(Object.hasOwn(initialRequest, "apiKey"), false);
  assert.equal(Object.hasOwn(initialRequest, "accessToken"), false);
  const automatic = await save();
  assert.deepEqual(automatic.models[0].overrides, {});
  assert.equal(automatic.models[0].catalogRef, null);
  assert.deepEqual(automatic.models[0].compression, { mode: "inherit" });
  assert.equal(automatic.chatSearch,"auto","older cards default to automatic contract matching");
  console.log("PASS model resolutions show the matched limits, catalog and input-budget threshold without saving automatic values as overrides");

  await reset("provider");
  await ready();
  await page.getByRole("button",{name:/高级设置/}).click();
  const searchContract = page.getByRole("combobox",{name:"跨协议搜索方式",exact:true});
  await searchContract.click();
  await page.getByRole("option",{name:"上游 MiMo Chat 搜索",exact:true}).click();
  const searchSaved = await save();
  assert.equal(searchSaved.chatSearch,"mimo");
  await page.locator("#reopen-saved").click();
  if (!(await searchContract.isVisible())) await page.getByRole("button",{name:/高级设置/}).click();
  await searchContract.getByText("上游 MiMo Chat 搜索",{exact:true}).waitFor();
  if (captureDir) {
    await searchContract.scrollIntoViewIfNeeded();
    await page.screenshot({path:path.join(captureDir,"provider-search-contract.png"),animations:"disabled"});
  }
  console.log("PASS the search contract saves and reopens without changing the selected model or protocol");

  await reset("provider");
  await ready();
  await page.getByRole("button",{name:/高级设置/}).click();
  await page.getByRole("button",{name:"检测搜索能力",exact:true}).click();
  await page.getByText("已实际执行搜索",{exact:true}).waitFor();
  await page.getByText("拒绝搜索声明",{exact:true}).waitFor();
  await page.getByText("2 个来源").waitFor();
  const probed = await page.evaluate(() => window.review.searchProbe);
  assert.ok(probed?.id && probed.model, "the probe uses the saved card and its selected model");
  await page.getByRole("button",{name:"使用建议：上游 OpenAI Chat 搜索",exact:true}).click();
  await page.getByRole("combobox",{name:"跨协议搜索方式",exact:true}).getByText("上游 OpenAI Chat 搜索",{exact:true}).waitFor();
  assert.equal((await save()).chatSearch,"openAi","a suggestion changes the draft only; saving applies it");
  console.log("PASS search capability check reports actual outcomes and applies a suggestion only to the draft");

  await reset("provider");
  await ready();
  const preset = row("model-A").getByRole("button", { name: /272K/ });
  assert.equal(await preset.getAttribute("aria-pressed"), "false");
  await preset.click();
  await eventually(async () => assert.equal(await context("model-A").inputValue(), "272000"));
  assert.equal(await preset.getAttribute("aria-pressed"), "true");
  const presetSaved = await save();
  assert.equal(presetSaved.models[0].overrides.contextWindow, 272000);
  assert.equal(presetSaved.models[0].contextWindow, null);
  assert.equal(presetSaved.models[0].overrides.inputLimit, undefined);
  assert.deepEqual(presetSaved.models[0].compression, { mode: "inherit" });
  await page.locator("#reopen-saved").click();
  await eventually(async () => assert.equal(await context("model-A").inputValue(), "272000"));
  await row("model-A").getByRole("button", { name: /自动|Auto/i, exact: true }).click();
  await ready();
  assert.equal(await context("model-A").inputValue(), "");
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "1000000");
  const restored = await save();
  assert.equal(restored.models[0].overrides.contextWindow, null);
  console.log("PASS the Codex 272K shortcut uses the existing model override, survives reopening, and Auto restores the 1M catalog default");

  await reset("provider");
  await ready();
  await page.evaluate(() => { window.review.modelResolutionPatches["model-A"] = { codexContext: { default: 500000, max: 872000 } }; });
  await context("model-A").fill("64000");
  const solPreset = row("model-A").getByRole("button", { name: /500K/ });
  await solPreset.waitFor();
  await eventually(async () => assert.match(await solPreset.getAttribute("title"), /500,000.*872,000/));
  assert.equal(await row("model-A").getByRole("button", { name: /272K/ }).count(), 0);
  await solPreset.click();
  await eventually(async () => assert.equal(await context("model-A").inputValue(), "500000"));
  assert.equal(await solPreset.getAttribute("aria-pressed"), "true");
  await page.evaluate(() => { window.review.modelResolutionPatches = {}; });
  console.log("PASS the Codex context shortcut follows Codex's own default for the model (GPT-6.1 Sol 500K) and names its maximum");

  await reset("provider");
  await ready();
  await context("model-A").fill("64000");
  await page.waitForFunction(() => window.review.modelResolutionRequests.at(-1)?.models[0]?.overrides?.contextWindow === 64000);
  await eventually(async () => assert.equal(await context("model-A").inputValue(), "64000"));
  await context("model-A").fill("");
  await page.waitForFunction(() => window.review.modelResolutionRequests.at(-1)?.models[0]?.overrides?.contextWindow === null);
  await ready();
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "1000000");
  await page.evaluate(() => {
    window.review.modelResolutionPatches["model-A"] = { matchKind: "invalid", matched: null,
      context: { value: null, source: "unknown", sourceRef: null, updatedAt: null } };
  });
  await catalog("model-A").fill("missing/model");
  await ready("model-A", "invalid");
  assert.equal(await catalog("model-A").inputValue(), "missing/model");
  assert.notEqual(await catalog("model-A").getAttribute("placeholder"), "fixture-official/model-A");
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "");
  await page.evaluate(() => { window.review.modelResolutionPatches = {}; });
  await catalog("model-A").fill("");
  await ready();
  await catalog("model-A").fill(" fixture-official / model-A ");
  await page.waitForFunction(() => window.review.modelResolutionRequests.at(-1)?.models[0]?.catalogRef?.providerId === "fixture-official"
    && window.review.modelResolutionRequests.at(-1)?.models[0]?.catalogRef?.modelId === "model-A");
  await eventually(async () => assert.equal(await catalog("model-A").inputValue(), "fixture-official/model-A"));
  await catalog("model-A").fill("");
  await ready();
  const cleared = await save();
  assert.equal(cleared.models[0].overrides.contextWindow, null);
  assert.equal(cleared.models[0].catalogRef, null);
  console.log("PASS clearing a manual limit restores resolution and an invalid catalog reference cannot retain a previous match");

  await reset("provider");
  await ready();
  const text = await labels();
  await page.getByRole("button", { name: text.fetchModels, exact: true }).click();
  await page.evaluate(() => window.review.resolveModels([
    { id: "model-A", displayName: "model-A", contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream" },
    { id: "newly-fetched-model", displayName: "newly-fetched-model", contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream" },
    ...Array.from({ length: 600 }, (_, index) => ({ id: `pending-${index}`, displayName: `pending-${index}`,
      contextWindow: null, maxOutputTokens: null, endpointTypes: ["responses"], included: true, metadataSource: "upstream" })),
  ]));
  await page.locator("[data-pending-models]").getByText("newly-fetched-model", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await row("newly-fetched-model").count(), 0));
  assert.deepEqual((await lastRequest()).models.map(item => item.id), ["model-A"]);
  await page.getByRole("checkbox", { name: text.includeFetchedModel, exact: true }).check();
  await ready("newly-fetched-model");
  assert.equal(digits(await context("newly-fetched-model").getAttribute("placeholder")), "1000000");
  assert.deepEqual((await lastRequest()).models.map(item => item.id), ["model-A", "newly-fetched-model"]);
  await page.getByPlaceholder(text.addModelPlaceholder, { exact: true }).fill("unknown-model");
  await page.getByRole("button", { name: text.addModel, exact: true }).click();
  await ready("unknown-model", "unmatched");
  assert.equal(digits(await context("unknown-model").getAttribute("placeholder")), "");
  assert.notEqual(await catalog("unknown-model").getAttribute("placeholder"), "fixture-official/unknown-model");
  assert.match(await inherit("unknown-model").innerText(), /默认|Default/i);
  const draft = await lastRequest();
  assert.equal(draft.models.find(item => item.id === "newly-fetched-model").included, true);
  assert.ok(draft.models.some(item => item.id === "unknown-model"));
  console.log("PASS 600 pending models stay outside resolution requests, newly included drafts resolve, and unknown models identify fallback compression");

  await reset("compact");
  await page.evaluate(() => {
    const item = window.review.getState().providers[0];
    const model = { ...item.models[0], id: "model-A", displayName: "model-A", included: true };
    window.review.dialogProviderFixture = { ...item, id: "A", model: "model-A", models: [model],
      compression: { mode: "tokens", tokens: 100000 } };
    window.review.modelResolutionPatches["model-A"] = { projection: { compression: {
      mode: "tokens", percent: null, tokens: 100000, source: "provider", threshold: 100000 } } };
    window.review.mount("provider");
  });
  await ready();
  assert.match(await inherit("model-A").innerText(), /100\s*K/i);
  assert.doesNotMatch(await inherit("model-A").innerText(), /90%/);
  assert.deepEqual((await lastRequest()).compression, { mode: "tokens", tokens: 100000 });
  await page.evaluate(() => { window.review.modelResolutionPatches = {}; });
  await page.locator("#newapi-provider-compression").click();
  await page.getByRole("option", { name: /跟随上级|Inherit/i }).click();
  await page.waitForFunction(() => window.review.modelResolutionRequests.at(-1)?.compression?.mode === "inherit");
  await ready();
  await eventually(async () => assert.match(await inherit("model-A").innerText(), /90%/));
  assert.match(await inherit("model-A").innerText(), /720\s*K/i);
  const compressionSaved = await save();
  assert.deepEqual(compressionSaved.compression, { mode: "inherit" });
  assert.deepEqual(compressionSaved.models[0].compression, { mode: "inherit" });
  console.log("PASS model inheritance displays the provider policy and updates to the global policy when the provider override is cleared");

  const heldProvider = async () => {
    await reset("compact");
    await page.evaluate(() => { window.review.holdModelResolutions = true; window.review.mount("provider"); });
    await page.waitForFunction(() => window.review.modelResolutionRequests.some(request => request.providerId === "A" && request.models.some(model => model.id === "model-A")));
  };
  const currentIndex = () => page.evaluate(() => window.review.modelResolutionRequests.length - 1);
  await heldProvider();
  const failedRequest = await currentIndex();
  await page.evaluate(index => window.review.modelResolutionRejectors[index]("synthetic preview failure"), failedRequest);
  await row("model-A").locator('[data-model-resolution="error"]').waitFor();
  const retryLabels = await labels();
  await page.getByRole("button", { name: retryLabels.modelResolutionRetry, exact: true }).click();
  await page.waitForFunction(index => window.review.modelResolutionRequests.length > index + 1, failedRequest);
  await page.evaluate(() => window.review.modelResolutionResolvers.at(-1)());
  await ready();
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "1000000");
  console.log("PASS a failed draft resolution can be retried without losing editable model settings");

  await heldProvider();
  const original = await currentIndex();
  await page.evaluate(() => { window.review.modelResolutionPatches["model-A"] = {
    context: { value: 2000000, source: "catalog", sourceRef: { providerId: "fixture-official", modelId: "model-A" }, updatedAt: null } }; });
  await page.locator("#newapi-provider-baseUrl").fill("https://changed.invalid");
  await page.waitForFunction(index => window.review.modelResolutionRequests.length > index + 1, original);
  const newest = await currentIndex();
  await page.evaluate(index => window.review.modelResolutionResolvers[index](), newest);
  await ready();
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "2000000");
  await page.evaluate(index => window.review.modelResolutionResolvers[index](), original);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "2000000");
  await page.locator("#newapi-provider-baseUrl").fill("https://closing.invalid");
  await page.waitForFunction(index => window.review.modelResolutionRequests.length > index + 1, newest);
  const closingA = await currentIndex();
  await page.getByRole("button", { name: (await labels()).cancel, exact: true }).click();
  await page.locator("#open-b").click();
  await page.waitForFunction(() => window.review.modelResolutionRequests.at(-1)?.providerId === "B");
  const bRequest = await currentIndex();
  await page.evaluate(index => window.review.modelResolutionResolvers[index](), closingA);
  await page.evaluate(index => window.review.modelResolutionResolvers[index](), bRequest);
  await ready("model-B");
  assert.equal(await catalog("model-B").getAttribute("placeholder"), "fixture-official/model-B");
  assert.equal(await row("model-A").count(), 0);
  await reset("provider");
  await ready();
  const beforeReopen = await currentIndex();
  await page.evaluate(() => {
    window.review.holdModelResolutions = true;
    window.review.modelResolutionPatches["model-A"] = { projection: { compression: { percent: 80, threshold: 640000 } } };
  });
  await page.getByRole("button", { name: (await labels()).cancel, exact: true }).click();
  await page.locator("#reopen-saved").click();
  await row("model-A").locator('[data-model-resolution="loading"]').waitFor();
  assert.equal(digits(await context("model-A").getAttribute("placeholder")), "");
  assert.doesNotMatch(await inherit("model-A").innerText(), /720\s*K/i);
  await page.waitForFunction(index => window.review.modelResolutionRequests.length > index + 1, beforeReopen);
  await page.evaluate(() => window.review.modelResolutionResolvers.at(-1)());
  await ready();
  assert.match(await inherit("model-A").innerText(), /80%/);
  assert.match(await inherit("model-A").innerText(), /640\s*K/i);
  console.log("PASS old draft and provider responses are isolated, and reopening the same provider refreshes a changed global policy without showing cached values");

  await reset("provider");
  await ready();
  for (const [width, height, dark] of [[1200, 700, false], [720, 900, true]]) {
    await page.setViewportSize({ width, height });
    await page.evaluate(dark => document.documentElement.classList.toggle("dark", dark), dark);
    await row("model-A").scrollIntoViewIfNeeded();
    const bounds = await row("model-A").evaluate(element => ({
      left: element.getBoundingClientRect().left, right: element.getBoundingClientRect().right,
      viewport: window.innerWidth, scrollWidth: element.scrollWidth, clientWidth: element.clientWidth,
    }));
    assert.ok(bounds.left >= -1 && bounds.right <= bounds.viewport + 1, JSON.stringify(bounds));
    assert.ok(bounds.scrollWidth <= bounds.clientWidth + 1, JSON.stringify(bounds));
    if (captureDir) await page.screenshot({ path: path.join(captureDir, `provider-model-resolutions-${width}-${dark ? "dark" : "light"}.png`), animations: "disabled" });
  }
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  console.log("PASS resolved model controls stay within the dialog at desktop and narrow widths in light and dark themes");
}
