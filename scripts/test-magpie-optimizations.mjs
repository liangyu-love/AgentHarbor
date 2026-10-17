import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testMagpieOptimizations({ page, reset, captureDir }) {
  await reset("compact");
  await page.evaluate(() => {
    const base = window.review.getState().providers[0];
    window.review.providerFixtures = [
      { ...base, id: "command-chat", name: "Command Chat", protocol: "chatCompletions", webSearchCapable: true },
      { ...base, id: "command-messages", name: "Command Messages", protocol: "anthropicMessages", webSearchCapable: true },
      { ...base, id: "native-search", name: "Native search", model: "search-r", protocol: "responses", webSearchCapable: true, webSearchModels: ["search-r", "search-m"] },
      { ...base, id: "unsupported-search", name: "Other relay", webSearchCapable: false },
    ];
    // Codex Native does not disable Desktop's independent search settings.
    window.review.routerPatch = { accessMode: "native" };
    window.review.mount("access");
  });
  // WebSearch is shared by every routed client, so it sits under the client
  // console instead of in one client's panel and stays visible on every tab.
  await page.locator('[data-client-tab="cursor"]').click();
  const searchService = page.getByRole("combobox", { name: "搜索执行账号", exact: true });
  await searchService.waitFor();
  assert.equal(await page.locator("[data-client-console] > [data-client-shared] [data-web-search-settings]").count(), 1);
  assert.equal(await page.locator('[role="tabpanel"] [data-web-search-settings]').count(), 0);
  assert.equal(await page.locator("[data-route-diagnostics] [data-web-search-settings]").count(), 0);
  await page.locator('[data-client-tab="codex"]').click();
  assert.equal(await searchService.isVisible(), true);
  await eventually(async () => assert.equal(await searchService.isEnabled(), true));
  await searchService.click();
  await eventually(async () => assert.equal(await page.getByRole("option", { name: "Other relay", exact: true }).count(), 0));
  await page.getByRole("option", { name: "Command Messages", exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.webSearchProviderId === "command-messages");
  await searchService.click();
  await page.getByRole("option", { name: "Native search", exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.webSearchModelId === "search-r");
  const searchModel = page.getByRole("combobox", { name: "搜索模型", exact: true });
  await searchModel.click();
  await page.getByRole("option", { name: "search-m", exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.webSearchModelId === "search-m");
  await page.evaluate(() => {
    window.review.routerPatch = { ...window.review.routerPatch, webSearchProviderId: "native-search", webSearchModelId: "search-m" };
    window.review.mount("access");
  });
  await searchModel.getByText("search-m", { exact: true }).waitFor();
  if (captureDir) {
    await searchModel.scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(captureDir, "native-search-settings.png"), animations: "disabled" });
  }
  await searchService.click();
  await page.getByRole("option", { name: "未指定原生搜索兜底", exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.webSearchProviderId === null);
  assert.equal(await page.evaluate(() => window.review.getState().router.webSearchModelId), null);
  await page.evaluate(() => {
    window.review.providerFixtures = window.review.providerFixtures.filter(provider => provider.id === "command-chat");
    window.review.mount("access");
  });
  await searchService.waitFor();
  await page.evaluate(() => window.review.mount("compact"));
  await searchService.waitFor({ state: "detached" });
  await page.evaluate(() => { window.review.routerPatch = { webSearchProviderId: "removed-service" }; window.review.mount("access"); });
  await searchService.getByText("已选搜索服务不可用", { exact: true }).waitFor();
  await page.evaluate(() => { delete window.review.providerFixtures; delete window.review.routerPatch; });
  await page.evaluate(() => { window.review.providerFixtures = []; window.review.mount("access"); });
  await page.getByText("还没有能执行联网搜索的中转或账号；添加后可以在这里指定，供所有经路由的客户端使用。", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await searchService.isDisabled(), true, "nothing to choose without a search-capable relay"));
  await page.evaluate(() => { delete window.review.providerFixtures; });
  console.log("PASS search service selection accepts either model protocol, saves automatic mode and retains unavailable explicit choices");
  await reset("access");
  const routes = page.getByRole("region", { name: "请求级路由诊断" });
  await routes.getByRole("button", { name: "请求级路由诊断", exact: true }).click();
  await routes.locator("summary").waitFor();
  await routes.locator("summary").click();
  await routes.getByText("客户端请求参数", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await routes.getByText("reasoning.effort", { exact: true }).isVisible(), true));
  assert.equal(await routes.getByText("output_config.effort", { exact: true }).isVisible(), true);
  assert.match(await routes.innerText(), /claude-opus-5-5-kiro/);
  assert.match(await routes.innerText(), /claude-opus-5-5 → Synthetic channel/);
  assert.match(await routes.innerText(), /230 ms/);
  assert.match(await routes.innerText(), /尝试 ID: syntheti/);
  assert.match(await routes.innerText(), /fixture-vers/);
  assert.match(await routes.innerText(), /模型目录参考价/);
  assert.match(await routes.innerText(), /未发送搜索声明（兼容兜底）/);
  assert.doesNotMatch(await routes.innerText(), /catalogReference/);
  assert.equal(await routes.getByRole("switch", { name: "同模型渠道故障切换" }).count(), 0,
    "ordinary provider requests no longer expose hidden cross-card failover");
  await page.locator('section[aria-labelledby="aggregation-title"]').waitFor();
  const catalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await catalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  const modelSummary = catalog.locator("summary").filter({ hasText: "Synthetic channel" });
  await modelSummary.waitFor();
  await modelSummary.click();
  await eventually(async () => assert.equal(await catalog.getByText("My custom alias", { exact: false }).isVisible(), true));
  assert.match(await catalog.innerText(), /继续使用上一份有效目录/);
  assert.match(await catalog.innerText(), /1,000,000/);
  assert.match(await catalog.innerText(), /内置模型规则/);
  assert.match(await catalog.innerText(), /anthropic\/claude-opus-5-5/);
  assert.match(await catalog.innerText(), /reasoning_content/);
  assert.match(await catalog.innerText(), /模型目录参考价/);
  // The applied state distinguishes what clients already use from a pending sync.
  assert.match(await catalog.innerText(), /已生效/);
  assert.match(await catalog.innerText(), /阈值 900,000/);
  // Manual price overrides are saved per model and estimated cost only.
  await catalog.getByRole("button", { name: "编辑价格", exact: true }).click();
  await catalog.getByLabel("输入", { exact: true }).fill("1.5");
  await catalog.getByLabel("输出", { exact: true }).fill("7.5");
  await catalog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => (window.review.priceSaves ?? []).length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.priceSaves[0]), {
    providerId: "relay",
    modelId: "claude-opus-5-5-kiro",
    price: { input: 1.5, output: 7.5, cacheRead: null, cacheWrite: null },
  });
  await catalog.getByRole("textbox").fill("absent-model");
  await catalog.getByText("没有匹配的已配置模型。", { exact: true }).waitFor();
  await catalog.getByRole("textbox").fill("kiro");
  await modelSummary.waitFor();
  if (captureDir) { await routes.scrollIntoViewIfNeeded(); await page.screenshot({ path: path.join(captureDir, "routing-diagnostics.png"), fullPage: true }); }
  for (const dark of [false, true]) {
    await page.evaluate(dark => {
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.setAttribute("data-has-custom-bg", "true");
    }, dark);
    // These panels animate their surface color; inspect the settled wallpaper
    // surface rather than the first frame of the transition.
    await page.waitForFunction(() => [...document.querySelectorAll(
      'section[aria-label="请求级路由诊断"], section[aria-label="模型能力目录与匹配预览"]',
    )].flatMap(element => [element, ...element.querySelectorAll("details")])
      .every(element => /, 0\.82\)$/.test(getComputedStyle(element).backgroundColor)));
    for (const panel of [routes, catalog]) {
      const surfaces = await panel.evaluate(element => [element, ...element.querySelectorAll("details")]
        .map(surface => getComputedStyle(surface).backgroundColor));
      assert.ok(surfaces.every(color => /, 0\.82\)$/.test(color)),
        `Wallpaper surfaces retain their alpha in ${dark ? "dark" : "light"} mode: ${surfaces}`);
    }
  }
  await page.evaluate(() => document.documentElement.removeAttribute("data-has-custom-bg"));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await routes.scrollIntoViewIfNeeded();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true, "diagnostics should not create horizontal page overflow");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "routing-diagnostics-mobile-dark.png"), fullPage: true });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS routing trace exposes requested/sent values, old failover is removed, and capability sources survive stale cache");

  await reset("access");
  await page.evaluate(() => {
    window.review.capabilityVariants = [
      { compressionApplied: false, compressionScope: "nativeShared", compressionSharedApplied: true,
        compressionSharedThreshold: 90000, compressionAppliedThreshold: 90000, compressionLimitingModels: ["small"] },
      { providerId: "inactive", displayName: "Inactive model", compressionApplied: false, compressionScope: "inactive",
        compressionSharedThreshold: null, compressionAppliedThreshold: null, compressionLimitingModels: [] },
    ];
  });
  const nativeCatalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await nativeCatalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await nativeCatalog.locator("summary").first().waitFor();
  for (const summary of await nativeCatalog.locator("summary").all()) await summary.click();
  await eventually(async () => assert.match(await nativeCatalog.innerText(), /阈值 900,000/));
  assert.match(await nativeCatalog.innerText(), /原生共享阈值：90,000；当前配置：90,000/);
  assert.match(await nativeCatalog.innerText(), /由 small 限制/);
  assert.match(await nativeCatalog.innerText(), /当前客户端配置未应用此模型的压缩设置，仅显示建议值/);
  assert.equal(await nativeCatalog.getByText("待同步", { exact: true }).count(), 0);
  console.log("PASS native capability UI separates suggestions, actual shared limits, limiting models and inactive providers");

  await reset("access");
  await page.evaluate(() => {
    window.review.capabilityVariants = [
      ...["invalid", "unknown", "clientDefault"].map((state, index) => ({
        providerId: `blocked-${index}`, compressionApplied: false, compressionScope: "nativeShared",
        compressionSharedThreshold: null, compressionAppliedThreshold: null, compressionSharedApplied: false,
        compressionSharedPlan: { threshold: null, limitingModels: [], blockingModels: ["small"], state },
      })),
      { providerId: "inactive", compressionApplied: false, compressionScope: "inactive", compressionConfigActive: false,
        compressionAppliedThreshold: null, compressionWrittenThreshold: 900000 },
      { providerId: "audio", price: { currency: "USD", unit: "perMillionTokens", input: 1, output: 2,
        cacheRead: null, cacheWrite: null, reasoning: null, inputAudio: 10, outputAudio: 20,
        source: "catalog", sourceRef: null, updatedAt: null, complete: false, note: null,
        contextOver200k: { input: 3, output: 4, inputAudio: 30 }, tiers: [{ input: 5, output: 6, tier: { type: "context", size: 300000 } }] } },
    ];
  });
  const blockedCatalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await blockedCatalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await blockedCatalog.locator("summary").first().waitFor();
  for (const summary of await blockedCatalog.locator("summary").all()) await summary.click();
  const blockedText = await blockedCatalog.innerText();
  assert.match(blockedText, /有无效策略，未注入共享阈值/);
  assert.match(blockedText, /有未知预算，未注入共享阈值/);
  assert.match(blockedText, /要求由客户端管理，未注入共享阈值/);
  assert.match(blockedText, /目录文件记录阈值 900,000，当前配置未引用/);
  assert.match(blockedText, /音频输入 10 · 音频输出 20/);
  assert.match(blockedText, /当前未采集音频用量/);
  assert.match(blockedText, /输入达到 300,000 Token 的单价/);
  console.log("PASS blocked shared policies, inactive written catalogs and retained audio/conditional prices are explained in capability UI");

  await reset("usage");
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    const request = { statusCode: 200, success: true, durationMs: 10, modelMatchStatus: "unknown", credentialStatus: "recorded", matched: false };
    const entry = overrides => window.review.usageEntry({ timestamp: now - 120, client: "opencode", channel: "relay",
      providerId: "group-a", providerName: "Relay Alpha", inputTokens: 50000, outputTokens: 50000, totalTokens: 100000,
      estimated: false, request, ...overrides });
    window.review.usageEntries = [
      entry({ model: "same-model", costUsd: 1, pricingStatus: "partial" }),
      entry({ model: "same-model", providerId: "other", providerName: "Other", costUsd: null, pricingStatus: "unpriced" }),
      entry({ model: "legacy-model", providerId: "legacy", providerName: "Legacy", costUsd: 3, pricingStatus: "legacy" }),
      entry({ model: "free-model", providerId: "free", providerName: "Free", costUsd: 0, pricingStatus: "priced" }),
      entry({ model: "unknown-model", providerId: "unknown", providerName: "Unknown", costUsd: null, pricingStatus: "unpriced" }),
    ];
    window.review.mount("usage");
  });
  const costKpi = page.locator('[data-usage-kpi="cost"]');
  await eventually(async () => assert.match(await costKpi.innerText(), /\$4\.00/));
  assert.match(await costKpi.innerText(), /2 笔未计价/);
  assert.match(await costKpi.innerText(), /1 笔部分计价/);
  assert.match(await costKpi.innerText(), /1 笔历史计价完整性未知/);
  const models = page.getByRole("region", { name: "模型", exact: true });
  const sameModel = models.locator("[data-usage-rank]").filter({ hasText: "same-model" });
  assert.match(await sameModel.innerText(), /Relay Alpha · Other/);
  assert.match(await sameModel.innerText(), /1 笔未计价/);
  assert.match(await sameModel.innerText(), /1 笔部分计价/);
  assert.match(await models.locator("[data-usage-rank]").filter({ hasText: "free-model" }).innerText(), /\$0/);
  await page.getByRole("radio", { name: "Token", exact: true }).click();
  await eventually(async () => assert.match(await models.locator("[data-usage-rank]").filter({ hasText: "unknown-model" }).innerText(), /无法可靠计算成本/));
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  assert.equal(await models.evaluate(element => element.scrollWidth <= element.clientWidth), true, "pricing coverage must wrap within the model ranking");
  await page.evaluate(() => localStorage.removeItem("agentharbor.usage-metric"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS costs keep partial, unpriced and unknown coverage in the summary and per model, on narrow screens too");

  await reset("access");
  await page.evaluate(() => {
    const reference = { providerId: "anthropic", modelId: "claude-opus-4-7" };
    window.review.capabilityVariants = [{ upstreamModel: "claude-opus-4-7-fast", displayName: "Opus Fast",
      context: { value: 1_000_000, source: "catalogSpeedReference", sourceRef: reference, updatedAt: null },
      steps: [{ modelId: "claude-opus-4-7", step: "speedVariantReference", matched: [reference], note: "1M" }] }];
  });
  const fastCatalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await fastCatalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await fastCatalog.locator("summary").first().waitFor();
  await fastCatalog.locator("summary").first().click();
  await fastCatalog.getByText("基础型号目录上下文", { exact: true }).waitFor();
  await fastCatalog.getByText(/Fast 模式：参考基础型号的上下文上限/).waitFor();
  assert.doesNotMatch(await fastCatalog.innerText(), /catalogSpeedReference|speedVariantReference/,
    "Fast context provenance must display localized names rather than resolver codes");
  console.log("PASS Fast context references show localized source and match steps");

  await reset("access");
  await page.evaluate(() => { window.review.desktopDiagnostics = true; });
  const desktopRoutes = page.getByRole("region", { name: "请求级路由诊断" });
  await desktopRoutes.getByRole("button", { name: "请求级路由诊断", exact: true }).click();
  await desktopRoutes.getByText("claude-sonnet-4-6 → deepseek-fixture", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await desktopRoutes.locator("summary").count(), 2));
  await desktopRoutes.getByRole("combobox", { name: "客户端来源" }).click();
  await page.getByRole("option", { name: "Claude Desktop", exact: true }).click();
  await desktopRoutes.getByText("claude-sonnet-4-6 → deepseek-fixture", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await desktopRoutes.locator("summary").count(), 1));
  assert.match(await desktopRoutes.locator("summary").innerText(), /claude-sonnet-4-6 → deepseek-fixture/);
  await desktopRoutes.locator("summary").click();
  await desktopRoutes.getByText("Claude Desktop 角色映射", { exact: true }).waitFor();
  assert.doesNotMatch(await desktopRoutes.innerText(), /desktop_role_mapping/,
    "Desktop mapping reasons must be localized in expanded request diagnostics");
  await desktopRoutes.getByRole("combobox", { name: "客户端来源" }).click();
  await page.getByRole("option", { name: "未记录", exact: true }).click();
  await desktopRoutes.getByText("relay::claude-opus-5-5-kiro", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await desktopRoutes.locator("summary").count(), 1));
  assert.match(await desktopRoutes.locator("summary").innerText(), /relay::claude-opus-5-5-kiro/);

  await reset("usage");
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    const request = { statusCode: 200, success: true, durationMs: 20, modelMatchStatus: "unknown", credentialStatus: "recorded", matched: false };
    const base = { timestamp: now - 60, client: "unknown", channel: "relay", providerId: "fixture", providerName: "Relay Fixture",
      model: "deepseek-fixture", inputTokens: 3, outputTokens: 4, totalTokens: 7, costUsd: 0, pricingStatus: "priced",
      pricingSource: "catalogReference", estimated: false, request };
    window.review.usageEntries = [window.review.usageEntry(base), window.review.usageEntry({ ...base, timestamp: now - 30,
      client: "claudeDesktop", channel: "aggregate", requestedModel: "claude-sonnet-4-6" })];
    window.review.mount("usage");
  });
  const desktopUsage = page.locator("[data-usage-requests]");
  await desktopUsage.locator("tbody tr").nth(1).waitFor();
  await eventually(async () => assert.equal(await desktopUsage.locator("tbody tr").count(), 2));
  await desktopUsage.getByLabel("客户端来源", { exact: true }).selectOption("claudeDesktop");
  await desktopUsage.getByText("claude-sonnet-4-6 → deepseek-fixture", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await desktopUsage.locator("tbody tr").count(), 1));
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /Claude Desktop/);
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /\$0/);
  assert.equal(await desktopUsage.locator('td[title^="模型目录参考价"]').count(), 1,
    "usage records retain and localize the reference-price source");
  await desktopUsage.getByLabel("客户端来源", { exact: true }).selectOption("unknown");
  await desktopUsage.locator("tbody tr [data-usage-client]").filter({ hasText: /^未记录$/ }).waitFor();
  await eventually(async () => assert.equal(await desktopUsage.locator("tbody tr").count(), 1));
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /未记录/);
  console.log("PASS Desktop diagnostics and usage filter client source while retaining alias, actual billing model and zero cost");

  const sourceSamples = [
    ["codex", "Codex"], ["claudeCode", "Claude Code"], ["opencode", "OpenCode"],
    ["claudeDesktop", "Claude Desktop"], ["aggregate", "聚合网关"],
  ];
  await reset("access");
  await page.evaluate(samples => {
    const route = { timestamp: 1790000000, requested: {}, done: true, status: 200, durationMs: 10, attempts: [] };
    window.review.routeDiagnostics = [
      ...samples.map(([clientSource]) => ({ ...route, id: clientSource, clientSource, requestedModel: `source-${clientSource}` })),
      { ...route, id: "future", clientSource: "future-client", requestedModel: "source-future" },
      { ...route, id: "legacy", requestedModel: "source-legacy" },
    ];
    window.review.mount("access");
  }, sourceSamples);
  const sourceRoutes = page.getByRole("region", { name: "请求级路由诊断" });
  await sourceRoutes.getByRole("button", { name: "请求级路由诊断", exact: true }).click();
  await sourceRoutes.getByText("source-codex", { exact: true }).waitFor();
  for (const [value, label] of sourceSamples) {
    await sourceRoutes.getByRole("combobox", { name: "客户端来源" }).click();
    await page.getByRole("option", { name: label, exact: true }).click();
    await sourceRoutes.getByText(`source-${value}`, { exact: true }).waitFor();
    await eventually(async () => assert.equal(await sourceRoutes.locator("summary").count(), 1));
    assert.match(await sourceRoutes.locator("summary").innerText(), new RegExp(`source-${value}.*${label}`, "s"));
  }
  await sourceRoutes.getByRole("combobox", { name: "客户端来源" }).click();
  await page.getByRole("option", { name: "未记录", exact: true }).click();
  await sourceRoutes.getByText("source-future", { exact: true }).waitFor();
  await sourceRoutes.getByText("source-legacy", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await sourceRoutes.locator("summary").count(), 2, "missing and unrecognized client sources must share the unknown filter"));

  await reset("usage");
  await page.evaluate(samples => {
    const now = Math.floor(Date.now() / 1000);
    const request = { statusCode: 200, success: true, durationMs: 20, modelMatchStatus: "unknown", credentialStatus: "recorded", matched: false };
    const base = { channel: "relay", providerId: "fixture", providerName: "Relay Fixture", model: "deepseek-fixture",
      inputTokens: 3, outputTokens: 4, totalTokens: 7, costUsd: 0, pricingSource: "manual", estimated: false, request };
    // The ledger files missing and future client sources under one unknown client.
    window.review.usageEntries = [
      ...samples.map(([client], index) => window.review.usageEntry({ ...base, timestamp: now - 100 + index, client,
        requestedModel: `source-${client}` })),
      window.review.usageEntry({ ...base, timestamp: now - 10, client: "unknown" }),
      window.review.usageEntry({ ...base, timestamp: now - 5, client: "unknown" }),
    ];
    window.review.mount("usage");
  }, sourceSamples);
  const sourceUsage = page.locator("[data-usage-requests]");
  for (const [value, label] of sourceSamples) {
    await sourceUsage.getByLabel("客户端来源", { exact: true }).selectOption(value);
    await sourceUsage.locator("tbody tr").filter({ hasText: `source-${value}` }).waitFor();
    await eventually(async () => assert.equal(await sourceUsage.locator("tbody tr").count(), 1));
    assert.equal(await sourceUsage.locator("tbody tr [data-usage-client]").innerText(), label);
  }
  await sourceUsage.getByLabel("客户端来源", { exact: true }).selectOption("unknown");
  await sourceUsage.locator("tbody tr").nth(1).waitFor();
  await eventually(async () => assert.equal(await sourceUsage.locator("tbody tr").count(), 2));
  console.log("PASS request diagnostics and relay usage distinguish every client source and group missing or future values as unrecorded");

  await reset("access");
  await page.evaluate(() => {
    const common = { sent: {}, failureStage: null, error: null, durationMs: 15,
      catalogRevision: "fixture", matchKind: "normalized", pricingStatus: "unpriced", pricingSource: "",
      aggregateId: "aggregate-fixture", routeRevision: "revision-fixture" };
    const first = { ...common, attemptId: "deepseek-attempt-1", memberId: "member-a", providerId: "deepseek",
      providerName: "DeepSeek 官方", upstreamModel: "deepseek-flash", matchedModel: "deepseek-flash",
      protocol: "anthropicMessages", configuredProtocol: "anthropicMessages", effectiveProtocol: "anthropicMessages",
      status: 429, selectionReason: "aggregate_rotation", retryReason: null };
    const second = { ...common, attemptId: "command-attempt-2", memberId: "member-b", providerId: "command",
      providerName: "Command Code", upstreamModel: "deepseek/deepseek-v4.1-flash", matchedModel: "deepseek-v4.1-flash",
      protocol: "chatCompletions", configuredProtocol: "chatCompletions", effectiveProtocol: "chatCompletions",
      status: 200, selectionReason: "aggregate_retry",
      retryReason: "aggregate_rate_limited", modelObservation: { servedModel: "deepseek/deepseek-v4.1-flash", modelMatchStatus: "same" } };
    const searched = { ...second, attemptId: "search-attempt-1", selectionReason: "aggregate_rotation",
      retryReason: null, configuredProtocol: "responses", compatibilityPlan: "commandSearchViaChat" };
    const request = { timestamp: 1790000000,
      requestedModel: "deepseek-public", resolvedModel: second.upstreamModel, clientSource: "claudeDesktop",
      requested: {}, done: true, stage: "complete", status: 200, error: null, durationMs: 50 };
    window.review.routeDiagnostics = [
      { ...request, id: "aggregate-retry-request", attempts: [first, second] },
      { ...request, id: "aggregate-search-request", attempts: [searched, { ...searched,
        attemptId: "followup-attempt-3", selectionReason: "hosted_tool_followup" }] },
    ];
    window.review.mount("access");
  });
  const aggregateRoutes = page.getByRole("region", { name: "请求级路由诊断" });
  await aggregateRoutes.getByRole("button", { name: "请求级路由诊断", exact: true }).click();
  await aggregateRoutes.locator("summary").nth(1).waitFor();
  for (const summary of await aggregateRoutes.locator("summary").all()) await summary.click();
  const diagnosticText = await aggregateRoutes.innerText();
  for (const text of ["deepseek-public", "DeepSeek 官方", "Command Code", "deepseek/deepseek-v4.1-flash",
    "HTTP 429", "聚合渠道轮转", "聚合备用尝试", "首渠道返回 HTTP 429", "托管工具结果回填，同渠道续轮",
    "成员 ID: member-a", "成员 ID: member-b", "配置 / 执行协议: responses → chatCompletions",
    "尝试 ID: deepseek", "尝试 ID: command-", "尝试 ID: followup"]) {
    assert.ok(diagnosticText.includes(text), `missing aggregate diagnostic: ${text}`);
  }
  assert.equal(await aggregateRoutes.locator("summary").count(), 2,
    "each inbound request groups its own retries or hosted continuations as physical attempts");
  console.log("PASS aggregate diagnostics distinguish actual cards/models, retries, protocol adaptation and same-channel tool continuation");

  await reset("access");
  await page.evaluate(() => {
    window.review.capabilityVariants = [{ price: { currency: "USD", unit: "perMillionTokens",
      input: 0, output: 0, cacheRead: null, cacheWrite: null, tiers: [], contextOver200k: null,
      source: "manual", sourceRef: null, complete: true, note: null } }];
    window.review.mount("access");
  });
  const manualPrices = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await manualPrices.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await manualPrices.locator("summary").click();
  await manualPrices.getByRole("button", { name: "编辑价格", exact: true }).click();
  await eventually(async () => assert.equal(await manualPrices.getByLabel("输入", { exact: true }).inputValue(), "0"));
  assert.equal(await manualPrices.getByLabel("输出", { exact: true }).inputValue(), "0");
  await manualPrices.getByRole("button", { name: "恢复自动价格", exact: true }).click();
  await page.waitForFunction(() => window.review.priceSaves?.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.priceSaves[0]), {
    providerId: "relay", modelId: "claude-opus-5-5-kiro", price: null,
  });

  await reset("provider");
  await page.evaluate(() => {
    window.review.dialogProviderFixture = { ...window.review.getState().providers[0], priceCatalogProvider: "deepseek" };
    window.review.mount("provider");
  });
  const providerDialog = page.getByRole("dialog");
  await providerDialog.locator("#newapi-provider-name").fill("Renamed relay");
  await eventually(async () => assert.equal(await providerDialog.locator("#newapi-provider-price-catalog").count(), 0));
  await providerDialog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.saved?.name === "Renamed relay");
  assert.equal(await page.evaluate(() => window.review.saved.priceCatalogProvider), "deepseek",
    "unrelated edits preserve existing price bindings even though new bindings have no UI");
  console.log("PASS custom zero prices restore automatically and provider edits preserve hidden legacy price bindings");
}
