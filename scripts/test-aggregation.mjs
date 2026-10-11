import assert from "node:assert/strict";
import path from "node:path";

const PUBLIC_MODEL = "deepseek-v4.1-flash";
const FAST_WIRE_MODEL = "vendor/deepseek-v4.1-flash-fast";
const SECOND_WIRE_MODEL = "partner/deepseek-v4.1-flash";
const POOL_NAME = "DeepSeek Pool";
const PROFILE_NAME = "Aggregate regression profile";

// Only application DTOs and local browser storage are used. No provider keys,
// real Desktop configuration, or external endpoints enter this suite.
async function seed(page, reset, mode, options = {}) {
  await reset("compact");
  await page.evaluate(({ mode, options, publicModel, fastWireModel, secondWireModel, poolName }) => {
    const review = window.review;
    const base = review.getState().providers[0];
    const model = (id, protocol = "responses") => ({ ...base.models[0], id, displayName: id, included: true,
      contextWindow: 1_000_000, protocol, endpointTypes: [protocol], catalogRef: { providerId: "deepseek", modelId: publicModel } });
    review.providerFixtures = [
      { ...base, id: "relay-a", name: "Relay A", baseUrl: "https://relay-a.invalid/v1", enabled: true, hasApiKey: true,
        protocol: "responses", model: fastWireModel, models: [model(fastWireModel), model("relay-a-extra")] },
      { ...base, id: "relay-b", name: "Relay B", baseUrl: "https://relay-b.invalid/v1", enabled: true, hasApiKey: true,
        protocol: "anthropicMessages", model: secondWireModel, models: [model(secondWireModel, "anthropicMessages")] },
    ];
    if (options.suggestions) {
      const candidateModel = (id, catalogId) => ({ ...base.models[0], id, displayName: id, included: true,
        catalogRef: catalogId ? { providerId: "deepseek", modelId: catalogId } : null });
      review.providerFixtures = [
        { ...base, id: "unrelated", name: "Unrelated", model: "shared-wire", models: [candidateModel("shared-wire", "unrelated-explicit")] },
        { ...base, id: "selected", name: "Selected", model: "shared-wire", models: [candidateModel("shared-wire", "selected-explicit"), candidateModel("auto-only", null)] },
      ];
      review.capabilityVariants = [
        { providerId: "unrelated", upstreamModel: "shared-wire", matched: { providerId: "deepseek", modelId: "unrelated-explicit" } },
        { providerId: "unrelated", upstreamModel: "other-auto", matched: { providerId: "deepseek", modelId: "unrelated-matched" } },
        { providerId: "selected", upstreamModel: "shared-wire", matched: { providerId: "deepseek", modelId: "selected-explicit" } },
        { providerId: "selected", upstreamModel: "auto-only", matched: { providerId: "deepseek", modelId: "selected-matched" } },
        { providerId: "selected", upstreamModel: "unmatched-wire", matched: null },
      ];
    }
    const aggregate = { id: "pool-main", publicModelId: publicModel, displayName: poolName, enabled: true,
      members: [
        { id: "member-fast", providerId: "relay-a", actualModelId: fastWireModel, enabled: true, priority: 0, weight: 3 },
        { id: "member-native", providerId: "relay-b", actualModelId: secondWireModel, enabled: true, priority: 0, weight: 1 },
      ], strategy: "weightedRoundRobin", affinity: "session", retryPolicy: { maxAttempts: 2, failureThreshold: 3 }, compression: { mode: "inherit" } };
    review.aggregateModels = options.empty ? [] : [aggregate];
    review.aggregateSaves = [];
    review.aggregateCodexSelections = [];
    review.routerPatch = { enabled: false, accessMode: "router", aggregateModelId: options.codexTarget ?? null };
    if (mode.startsWith("clients")) {
      const targetId = mode.startsWith("clients-opencode") ? "opencode" : "claudeCode";
      review.clientStatusFixtures = [{ targetId, name: targetId === "opencode" ? "OpenCode" : "Claude Code",
        path: "C:/synthetic/aggregate-config.json", protocols: targetId === "opencode"
          ? ["responses", "chatCompletions", "anthropicMessages"] : ["anthropicMessages"],
        configured: !!options.savedRequest, conflict: false, error: null,
        providerId: options.savedRequest?.providerId ?? null, model: options.savedRequest?.model ?? null,
        request: options.savedRequest ?? null }];
    }
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { review.copiedAggregate = text; } } });
    review.mount(mode);
  }, { mode, options, publicModel: PUBLIC_MODEL, fastWireModel: FAST_WIRE_MODEL, secondWireModel: SECOND_WIRE_MODEL, poolName: POOL_NAME });
}

export async function testAggregation({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const manager = () => page.locator('section[aria-labelledby="aggregation-title"]');
  const choose = async (selector, name) => {
    await selector.click();
    await page.getByRole("option", { name, exact: true }).click();
    await page.getByRole("option", { name, exact: true }).waitFor({ state: "hidden" });
  };

  await seed(page, reset, "access");
  const copyId = manager().getByRole("button", { name: await tr("aggregation.copyPublicId"), exact: true });
  await page.evaluate(() => {
    navigator.clipboard.writeText = async () => { throw new Error("Synthetic clipboard denied"); };
  });
  await copyId.click();
  await manager().getByRole("alert").getByText("Synthetic clipboard denied", { exact: true }).waitFor();
  assert.equal(await copyId.locator(".lucide-check").count(), 0, "a rejected clipboard write never reports success");
  await page.evaluate(() => {
    navigator.clipboard.writeText = async text => { window.review.copiedAggregate = text; };
  });
  await copyId.click();
  await page.waitForFunction(() => window.review.copiedAggregate === "deepseek-v4.1-flash");
  await copyId.locator(".lucide-check").waitFor();
  await page.evaluate(() => { navigator.clipboard.writeText = async () => { throw new Error("Synthetic clipboard retry denied"); }; });
  await copyId.click();
  await manager().getByRole("alert").getByText("Synthetic clipboard retry denied", { exact: true }).waitFor();
  assert.equal(await copyId.locator(".lucide-check").count(), 0, "a failed retry clears the previous copy-success indicator");
  assert.equal(await manager().locator("[data-route-member]").count(), 2, "each upstream is displayed once");
  assert.equal(await manager().locator("[data-route-member] [data-route-member]").count(), 0);

  await manager().getByRole("button", { name: await tr("common.edit"), exact: true }).click();
  await page.evaluate(() => { window.review.holdAggregateSave = true; });
  await manager().locator("form").getByRole("button", { name: await tr("common.save"), exact: true }).click();
  await page.waitForFunction(() => typeof window.review.completeAggregateSave === "function");
  assert.equal(await manager().locator("form").getByRole("switch", { name: await tr("aggregation.enabled"), exact: true }).isDisabled(), true,
    "the aggregate enable toggle is locked while its saved draft is pending");
  await page.evaluate(() => { window.review.holdAggregateSave = false; window.review.completeAggregateSave(); });
  await manager().locator("form").waitFor({ state: "detached" });
  console.log("PASS injection UI handles clipboard failures and pending saves without false success or duplicated channel lists");

  const pipeline = page.locator('[aria-labelledby="newapi-routing-title"]');
  await page.evaluate(async () => {
    window.review.frontendClient.setQueryData(["newapi-state"], { ...window.review.getState(), activeProviderId: "relay-a" });
    window.review.getState().activeProviderId = "relay-a";
  });
  await choose(pipeline.locator("#codex-access-mode"), await tr("newapi.accessNative"));
  await pipeline.getByText("https://relay-a.invalid/v1", { exact: true }).waitFor();
  assert.equal(await pipeline.getByText("chatgpt.com / api.openai.com", { exact: true }).count(), 0);
  assert.equal(await pipeline.getByText("/codex/v1/responses", { exact: true }).count(), 0, "native access bypasses the local gateway");
  await choose(pipeline.locator("#codex-access-mode"), await tr("newapi.accessRouter"));

  // Long manual IDs, localized labels, theme and zoom-size layouts use only synthetic DTOs.
  await page.evaluate(async () => {
    window.review.aggregateModels[0].publicModelId = "manual/" + "very-long-model-id-".repeat(12);
    window.review.aggregateModels[0].displayName = "Long display name ".repeat(12);
    await window.review.frontendClient.invalidateQueries({ queryKey: ["model-aggregation"] });
  });
  for (const { width, height, theme, language } of [
    { width: 1440, height: 900, theme: "light", language: "zh" },
    { width: 960, height: 800, theme: "dark", language: "en" },
    { width: 768, height: 900, theme: "light", language: "en" },
    { width: 390, height: 844, theme: "dark", language: "zh" },
  ]) {
    await page.setViewportSize({ width, height });
    await page.evaluate(async ({ theme, language }) => {
      document.documentElement.classList.toggle("dark", theme === "dark");
      await window.review.setLanguage(language);
    }, { theme, language });
    const overflow = await page.evaluate(() => [...document.querySelectorAll(
      '[aria-labelledby="newapi-routing-title"], [data-client-console], [aria-labelledby="claude-desktop-title"], [aria-labelledby="aggregation-title"]'
    )].filter(element => element.scrollWidth > element.clientWidth + 1).map(element => element.getAttribute("aria-labelledby") ?? "console"));
    assert.deepEqual(overflow, [], `injection sections fit ${width}px ${language} ${theme}`);
    if (captureDir) {
      await page.locator("[data-page-scroll-container]").evaluate((element, height) => {
        element.parentElement.style.height = height + "px";
      }, height);
      await page.locator("[data-page-scroll-container]").evaluate(element => { element.scrollTop = 0; });
      await page.screenshot({ path: path.join(captureDir, `injection-top-${width}-${theme}.png`), animations: "disabled" });
      await page.locator("[data-client-console]").evaluate(element => element.scrollIntoView({ block: "start" }));
      await page.screenshot({ path: path.join(captureDir, `injection-clients-${width}-${theme}.png`), animations: "disabled" });
      await manager().evaluate(element => element.scrollIntoView({ block: "start" }));
      await page.screenshot({ path: path.join(captureDir, `injection-aggregate-${width}-${theme}.png`), animations: "disabled" });
      await manager().getByRole("button", { name: await tr("common.edit"), exact: true }).click();
      await manager().locator("form").evaluate(element => element.scrollIntoView({ block: "start" }));
      await page.screenshot({ path: path.join(captureDir, `injection-editor-${width}-${theme}.png`), animations: "disabled" });
      await manager().locator("form").getByRole("button", { name: await tr("common.cancel"), exact: true }).click();
    }
  }
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.evaluate(async () => { document.documentElement.classList.remove("dark"); await window.review.setLanguage("zh"); });
  console.log("PASS native topology, long model IDs and bilingual responsive theme layouts");

  await seed(page, reset, "access", { empty: true, suggestions: true });
  await manager().getByRole("button", { name: await tr("aggregation.add"), exact: true }).click();
  const suggestionIds = () => manager().locator("#aggregate-official-models option").evaluateAll(options => options.map(option => option.value));
  await page.waitForFunction(() => document.querySelectorAll("#aggregate-official-models option").length === 4);
  const allSuggestions = ["unrelated-explicit", "selected-explicit", "unrelated-matched", "selected-matched"];
  assert.deepEqual(await suggestionIds(), allSuggestions, "all cards supply unique explicit and automatic matches");
  const suggestionAddMember = manager().getByRole("button", { name: await tr("aggregation.addMember"), exact: true });
  await suggestionAddMember.click();
  const suggestionMembers = manager().locator("[data-aggregate-member]");
  await choose(suggestionMembers.nth(0).locator('[id^="member-provider-"]'), "Selected");
  assert.deepEqual(await suggestionIds(), ["selected-explicit", "unrelated-explicit", "unrelated-matched", "selected-matched"],
    "a same-named model on another card is unrelated to the selected member");
  await suggestionAddMember.click();
  await choose(suggestionMembers.nth(1).locator('[id^="member-provider-"]'), "Selected");
  await choose(suggestionMembers.nth(1).locator('[id^="member-model-"]'), "auto-only");
  assert.deepEqual(await suggestionIds(), ["selected-explicit", "selected-matched", "unrelated-explicit", "unrelated-matched"],
    "member matches move first without filtering or duplicating other candidates");
  await suggestionMembers.nth(0).getByRole("button", { name: await tr("aggregation.removeMember", { index: 1 }), exact: true }).click();
  assert.deepEqual(await suggestionIds(), ["selected-matched", "unrelated-explicit", "selected-explicit", "unrelated-matched"]);
  await manager().locator("#aggregate-public-id").fill("handwritten-public-model");
  await manager().locator("form").getByRole("button", { name: await tr("common.save"), exact: true }).click();
  await page.waitForFunction(() => window.review.aggregateSaves.length === 1);
  assert.deepEqual(await page.evaluate(() => ({ publicModelId: window.review.aggregateSaves[0].publicModelId,
    canonicalRef: window.review.aggregateSaves[0].canonicalRef, actualModelId: window.review.aggregateSaves[0].members[0].actualModelId })),
  { publicModelId: "handwritten-public-model", canonicalRef: null, actualModelId: "auto-only" });
  console.log("PASS public model suggestions prioritize exact member matches while retaining all cards, deduplication and handwritten identities");

  await seed(page, reset, "access", { empty: true });
  await manager().getByRole("button", { name: await tr("aggregation.add"), exact: true }).click();
  await manager().locator("#aggregate-public-id").fill(PUBLIC_MODEL);
  await manager().locator("#aggregate-display-name").fill(POOL_NAME);
  const addMember = manager().getByRole("button", { name: await tr("aggregation.addMember"), exact: true });
  await addMember.click();
  await addMember.click();
  const members = manager().locator("[data-aggregate-member]");
  await choose(members.nth(1).locator('[id^="member-provider-"]'), "Relay B");
  await choose(members.nth(1).locator('[id^="member-model-"]'), SECOND_WIRE_MODEL);
  assert.equal(await members.locator('[id^="member-weight-"]').count(), 0, "weights only matter, and show, for weighted rotation");
  await manager().locator("#aggregate-strategy").getByRole("radio", { name: await tr("aggregation.weightedRoundRobin") }).click();
  await members.nth(0).locator('[id^="member-weight-"]').fill("3");
  assert.equal(await members.nth(1).getByRole("button", { name: await tr("aggregation.moveUp", { index: 2 }), exact: true }).isDisabled(), true,
    "the primary tier has nothing above it");
  await members.nth(1).getByRole("button", { name: await tr("aggregation.moveDown", { index: 2 }), exact: true }).click();
  await manager().locator('[data-tier-lane="1"] [data-aggregate-member]').waitFor();
  assert.equal(await members.nth(1).getByRole("button", { name: await tr("aggregation.moveDown", { index: 2 }), exact: true }).isDisabled(), true,
    "a member alone in the last tier cannot start another");
  await choose(manager().locator("#aggregate-first-token"), await tr("aggregation.seconds", { seconds: 30 }));
  assert.equal(await manager().locator("#aggregate-failure-threshold").innerText(), "3");
  await choose(manager().locator("#aggregate-failure-threshold"), "4");
  await choose(manager().locator("#aggregate-attempts"), "1");
  await manager().getByText(await tr("aggregation.firstTokenNeedsRetry"), { exact: true }).waitFor();
  await choose(manager().locator("#aggregate-compression"), await tr("newapi.compression.percent"));
  await manager().getByRole("spinbutton", { name: await tr("newapi.compression.modelLevel"), exact: true }).fill("85");
  await manager().locator("form").getByRole("button", { name: await tr("common.save"), exact: true }).click();
  await page.waitForFunction(() => window.review.aggregateSaves.length === 1);
  const saved = await page.evaluate(() => window.review.aggregateSaves[0]);
  assert.equal(saved.publicModelId, PUBLIC_MODEL);
  assert.equal(saved.strategy, "weightedRoundRobin");
  assert.deepEqual(saved.retryPolicy, { maxAttempts: 1, failureThreshold: 4 });
  assert.equal(saved.firstTokenTimeoutSecs, 30);
  assert.deepEqual(saved.compression, { mode: "percent", percent: 85 });
  assert.deepEqual(saved.members.map(({ providerId, actualModelId, priority, weight }) => ({ providerId, actualModelId, priority, weight })), [
    { providerId: "relay-a", actualModelId: FAST_WIRE_MODEL, priority: 0, weight: 3 },
    { providerId: "relay-b", actualModelId: SECOND_WIRE_MODEL, priority: 1, weight: 1 },
  ]);
  assert.equal(new Set(saved.members.map(member => member.id)).size, 2);
  assert.ok(saved.members.every(member => Object.keys(member).sort().join(",") === "actualModelId,enabled,id,priority,providerId,weight"),
    "members save exact references and never copy card credentials");
  await manager().getByRole("button", { name: await tr("common.edit"), exact: true }).click();
  assert.equal(await manager().locator("#aggregate-public-id").isDisabled(), true, "public identity remains stable after creation");
  assert.equal(await manager().locator("#aggregate-failure-threshold").innerText(), "4");
  assert.equal(await members.nth(0).locator('[id^="member-model-"]').innerText(), FAST_WIRE_MODEL);
  assert.equal(await manager().getByRole("spinbutton", { name: await tr("newapi.compression.modelLevel"), exact: true }).inputValue(), "85");
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await manager().evaluate(element => element.scrollWidth <= element.clientWidth), true, "aggregation editor does not overflow on a phone-sized viewport");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "aggregation-editor-mobile.png"), fullPage: true });
  await page.setViewportSize({ width: 1200, height: 700 });
  await manager().locator("form").getByRole("button", { name: await tr("common.cancel"), exact: true }).click();
  console.log("PASS aggregation editor persists exact upstream identities, stable member IDs, priorities and weights");


  const codex = page.locator('[aria-labelledby="newapi-routing-title"]');
  await choose(codex.locator("#codex-aggregate-model"), POOL_NAME + " · " + PUBLIC_MODEL);
  await page.waitForFunction(() => window.review.getState().router.aggregateModelId !== null);
  const aggregateMarker = codex.locator('[data-codex-target="aggregate"]');
  assert.equal(await aggregateMarker.innerText(), await tr("aggregation.target"));
  assert.equal(await codex.locator("[data-relay-link]").count(), 0);
  assert.deepEqual(await page.evaluate(() => window.review.aggregateCodexSelections), [saved.id]);
  await codex.locator("#codex-access-mode").click();
  assert.equal(await page.getByRole("option", { name: await tr("newapi.accessNative"), exact: true }).getAttribute("aria-disabled"), "true");
  await page.keyboard.press("Escape");
  await codex.getByRole("switch", { name: await tr("newapi.codexAccess"), exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.enabled);
  await choose(codex.locator("#codex-aggregate-model"), await tr("aggregation.singleChannel"));
  await page.waitForFunction(() => window.review.getState().router.aggregateModelId === null);
  assert.equal(await codex.locator('[data-codex-target="relay"]').innerText(), await tr("newapi.providersSection"));
  assert.deepEqual(await page.evaluate(() => window.review.aggregateCodexSelections), [saved.id, null]);
  console.log("PASS Codex selects the aggregation catalog and returns to relay models with distinct abstract targets");

  await seed(page, reset, "access", { codexTarget: "deleted-pool" });
  await codex.locator("#codex-aggregate-model").waitFor();
  assert.equal(await codex.getByRole("switch", { name: await tr("newapi.codexAccess"), exact: true }).isDisabled(), true,
    "an unavailable explicit Codex target cannot silently enable the ordinary provider");
  await page.evaluate(async () => {
    window.review.aggregateModels[0].enabled = false;
    await window.review.frontendClient.invalidateQueries({ queryKey: ["model-aggregation"] });
  });
  await manager().getByRole("button", { name: await tr("common.delete"), exact: true }).click();
  const confirmation = page.getByRole("alertdialog");
  await confirmation.waitFor();
  assert.equal(await page.evaluate(() => (window.review.aggregateDeletes ?? []).length), 0);
  await confirmation.getByRole("button", { name: await tr("common.delete"), exact: true }).click();
  await page.waitForFunction(() => window.review.aggregateDeletes.length === 1);
  console.log("PASS unavailable targets block injection/export and aggregate deletion requires the application confirmation");

  await seed(page, reset, "clients");
  await page.evaluate(() => localStorage.removeItem("agentharbor.claude-code-profiles.v1"));
  await page.getByRole("button", { name: await tr("clients.configure"), exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  await dialog.getByRole("checkbox", { name: await tr("clients.includeModel", { model: POOL_NAME }), exact: true }).check();
  await choose(dialog.locator("#aggregate-claude-opus"), POOL_NAME);
  await dialog.getByRole("checkbox", { name: await tr("clients.oneMStartup"), exact: true }).check();
  await dialog.getByRole("button", { name: await tr("clients.saveProfile"), exact: true }).click();
  await dialog.getByRole("textbox", { name: await tr("clients.profileName"), exact: true }).fill(PROFILE_NAME);
  await dialog.getByRole("button", { name: await tr("clients.confirmSaveProfile"), exact: true }).click();
  const profile = await page.evaluate(() => JSON.parse(localStorage.getItem("agentharbor.claude-code-profiles.v1"))[0]);
  assert.deepEqual(profile.request.modelSelection.defaultTarget, { kind: "aggregate", aggregateId: "pool-main" });
  assert.deepEqual(profile.request.modelSelection.availableTargets, [{ kind: "aggregate", aggregateId: "pool-main" }]);
  assert.equal(profile.request.claudeModels.opus, PUBLIC_MODEL);
  assert.equal(profile.request.claudeOneM.startup, true);
  await dialog.getByRole("button", { name: "Relay B", exact: false }).click();
  await choose(dialog.locator("#claude-saved-profile"), PROFILE_NAME + " · " + POOL_NAME);
  await page.waitForFunction(() => window.review.previewed?.modelSelection?.defaultTarget?.kind === "aggregate");
  const restored = await page.evaluate(() => window.review.previewed);
  assert.deepEqual(restored.modelSelection, profile.request.modelSelection);
  assert.equal(restored.claudeOneM.startup, true);
  await dialog.getByRole("button", { name: await tr("clients.apply"), exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  const clientsCard = page.locator('section[id^="client-config-"]');
  await clientsCard.getByRole("button", { name: await tr("clients.reapply"), exact: true }).waitFor();
  assert.equal(await clientsCard.getByRole("alert").count(), 0, "saved aggregate target is recognized as a configured client");
  if (captureDir) {
    await clientsCard.evaluate(element => element.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: path.join(captureDir, "injection-saved-client.png"), animations: "disabled" });
  }
  await clientsCard.getByRole("button", { name: await tr("clients.change"), exact: true }).click();
  assert.equal(await dialog.getByRole("checkbox", { name: await tr("clients.includeModel", { model: POOL_NAME }), exact: true }).isChecked(), true);
  assert.equal(await dialog.getByRole("checkbox", { name: await tr("clients.oneMStartup"), exact: true }).isChecked(), true);
  console.log("PASS Claude Code aggregation profiles save, reload, apply and reopen without losing roles or 1M choices");

  await seed(page, reset, "clients", { empty: true });
  await page.getByRole("button", { name: await tr("clients.configure"), exact: true }).click();
  await choose(dialog.locator("#claude-saved-profile"), PROFILE_NAME + " · " + await tr("aggregation.unavailable"));
  await dialog.getByRole("alert").getByText(await tr("clients.errors.profileModelsUnavailable"), { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.previewed ?? null), null,
    "a missing saved aggregate never gets rewritten into an ordinary provider request");
  console.log("PASS saved Claude aggregation profiles reject removed targets without a fallback or preview write");

  await seed(page, reset, "clients-opencode");
  await page.getByRole("button", { name: await tr("clients.configure"), exact: true }).click();
  await dialog.getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  const subsets = dialog.getByRole("region", { name: await tr("clients.modelSubset"), exact: true });
  await subsets.locator("summary").first().click();
  await subsets.getByRole("checkbox", { name: await tr("clients.includeModel", { model: "relay-a-extra" }), exact: true }).uncheck();
  await dialog.getByRole("checkbox", { name: await tr("clients.includeModel", { model: POOL_NAME }), exact: true }).check();
  await choose(dialog.locator("#client-model"), await tr("aggregation.target") + " / " + POOL_NAME);
  // Adding a second ordinary provider must not reset the explicit aggregate default.
  await dialog.getByRole("button", { name: "Relay B", exact: false }).click();
  assert.equal(await dialog.locator("#client-model").innerText(), await tr("aggregation.target") + " / " + POOL_NAME);
  await choose(dialog.locator("#client-aggregate-protocol"), "Chat");
  await dialog.getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
  await page.waitForFunction(() => !!window.review.previewed);
  const openCodeRequest = await page.evaluate(() => window.review.previewed);
  assert.deepEqual(openCodeRequest.modelSelection.defaultTarget, { kind: "aggregate", aggregateId: "pool-main" });
  assert.equal(openCodeRequest.aggregateIngressProtocol, "chatCompletions");
  assert.deepEqual(openCodeRequest.providerModelIds, { "relay-a": [FAST_WIRE_MODEL], "relay-b": [SECOND_WIRE_MODEL] });
  assert.deepEqual(openCodeRequest.modelSelection.availableTargets, [
    { kind: "provider", providerId: "relay-a", modelId: FAST_WIRE_MODEL },
    { kind: "provider", providerId: "relay-b", modelId: SECOND_WIRE_MODEL },
    { kind: "aggregate", aggregateId: "pool-main" },
  ]);
  await dialog.getByRole("button", { name: await tr("clients.apply"), exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  await clientsCard.getByRole("button", { name: await tr("clients.change"), exact: true }).click();
  assert.equal(await dialog.locator("#client-aggregate-protocol").innerText(), "Chat");
  assert.equal(await dialog.locator("#client-model").innerText(), await tr("aggregation.target") + " / " + POOL_NAME);
  assert.equal(await dialog.getByRole("checkbox", { name: await tr("clients.includeModel", { model: POOL_NAME }), exact: true }).isChecked(), true);
  console.log("PASS OpenCode combines exact provider subsets and aggregation while retaining its explicit default across save/reopen");

  await seed(page, reset, "compact");
  await page.evaluate(publicModel => {
    const review = window.review;
    const roots = { normalRoot: "C:/synthetic/Claude", thirdPartyRoot: "C:/synthetic/Claude-3p" };
    const paths = { ...roots, normalConfig: roots.normalRoot + "/claude_desktop_config.json",
      thirdPartyConfig: roots.thirdPartyRoot + "/claude_desktop_config.json", profilePath: roots.thirdPartyRoot + "/configLibrary/profile.json",
      metaPath: roots.thirdPartyRoot + "/configLibrary/_meta.json", isolated: true };
    const detection = { platform: "windows", installed: true, version: "fixture", versionSource: "claudeDesktop.versionSources.package", notes: [] };
    const candidate = { modelId: publicModel, label: publicModel, displayName: publicModel, protocol: "anthropicMessages",
      protocolSource: "aggregate", protocolVerified: true, included: true, supports1m: true, contextWindow: 1_000_000,
      contextSource: "catalog", selectable: true, excludeReason: null };
    const candidates = { paths, pathCandidates: [{ id: "synthetic", ...roots, normalExists: true, thirdPartyExists: true, hasInstallEvidence: true }],
      selectedPathId: "synthetic", detector: detection, providers: [], routeProviders: [], selectedProviderId: null,
      routeRoles: [{ role: "sonnet", roleAlias: "claude-sonnet-4-6" }, { role: "opus", roleAlias: "claude-opus-4-7" }, { role: "haiku", roleAlias: "claude-haiku-4-5" }],
      routeAggregates: [{ aggregateId: "pool-main", name: "DeepSeek Pool", enabled: true, eligible: true, reason: null, model: candidate }] };
    const status = { detection, paths, mode: "official", managementState: "unmanaged", isManaged: false,
      providerId: null, providerName: null, providerMissing: false, models: [], modelChoices: [], routeChoices: [], routeViews: [],
      staleReasons: [], conflicts: [], recoveryState: null, pendingRecovery: false, warnings: [], lastAppliedAt: null,
      restartRequired: false, appRunning: false, restartCapable: false, launchMethod: null, backupCount: 0,
      actions: [{ action: "applyDirect", enabled: true, disabledReason: null }, { action: "applyRouter", enabled: true, disabledReason: null }] };
    review.desktopAggregatePreviews = [];
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command === "get_claude_desktop_status") return structuredClone(status);
      if (command === "list_claude_desktop_candidates") return structuredClone(candidates);
      if (command === "preview_claude_desktop_change") {
        review.desktopAggregatePreviews.push(structuredClone(args));
        return { revision: "synthetic-aggregate-preview", action: args.action, providerId: null, providerName: null,
          models: args.request.routes.map(route => ({ modelId: route.roleAlias, label: route.modelId, oneM: route.oneMMode === "auto" })),
          routeViews: [], router: { boundPort: 43127, routeRevision: 1, gatewayTokenVersion: 1, listenerReady: true },
          messagesUrl: "http://127.0.0.1:43127/desktop/v1/messages", modelsUrl: "http://127.0.0.1:43127/desktop/v1/models",
          files: [{ role: "profile", path: paths.profilePath, kind: "update", managedChanges: [{ path: "inferenceModels", before: null,
            after: JSON.stringify(args.request.routes.map(route => ({ name: route.roleAlias, supports1m: route.oneMMode === "auto" }))) }], note: null }],
          warnings: [], conflicts: [], requiresRestart: true, externalProfileCount: 0, managedEntryPresent: false, currentAppliedId: null, appliesTo: "Claude Desktop" };
      }
      if (command === "cancel_claude_desktop_preview") return true;
      throw new Error("Unexpected synthetic Desktop command " + command);
    };
    review.mount("access");
  }, PUBLIC_MODEL);
  const desktop = page.locator('section[aria-labelledby="claude-desktop-title"]');
  await page.locator('[data-client-tab="claudeDesktop"]').click();
  await desktop.getByRole("button", { name: await tr("claudeDesktop.card.configure"), exact: true }).click();
  await dialog.getByRole("button", { name: await tr("claudeDesktop.routing.modeRouter"), exact: true }).click();
  await choose(dialog.locator("#desktop-route-opus-provider"), await tr("aggregation.target") + " · " + POOL_NAME);
  assert.equal(await dialog.locator("#desktop-route-opus-model").isDisabled(), true);
  assert.equal(await dialog.locator("#desktop-route-opus-model").innerText(), PUBLIC_MODEL);
  await dialog.locator("#desktop-route-opus-onem").check();
  await dialog.getByRole("button", { name: await tr("claudeDesktop.dialog.preview"), exact: true }).click();
  await page.waitForFunction(() => window.review.desktopAggregatePreviews.length === 1);
  const desktopRequest = await page.evaluate(() => window.review.desktopAggregatePreviews[0]);
  assert.equal(desktopRequest.action, "applyRouter");
  assert.deepEqual(desktopRequest.request.routes[0].targetRef, { kind: "aggregate", aggregateId: "pool-main" });
  assert.equal(desktopRequest.request.routes[0].modelId, PUBLIC_MODEL);
  assert.equal(desktopRequest.request.routes[0].oneMMode, "auto");
  assert.equal(desktopRequest.request.routes[0].providerId, "");
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await dialog.evaluate(element => element.scrollWidth <= element.clientWidth), true, "Desktop aggregate mapping stays within the dialog");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "aggregation-desktop-mobile.png"), fullPage: true });
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS Desktop uses a typed aggregate route, keeps public identity and accepts its 1M checkbox with a bounded mobile layout");
}
