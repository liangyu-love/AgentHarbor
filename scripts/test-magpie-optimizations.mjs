import assert from "node:assert/strict";
import path from "node:path";

export async function testMagpieOptimizations({ page, reset, captureDir }) {
  await reset("access");
  const routes = page.getByRole("region", { name: "请求级路由诊断" });
  await routes.getByRole("button", { name: "请求级路由诊断", exact: true }).click();
  await routes.locator("summary").waitFor();
  await routes.locator("summary").click();
  await routes.getByText("客户端请求参数", { exact: true }).waitFor();
  assert.equal(await routes.getByText("reasoning.effort", { exact: true }).isVisible(), true);
  assert.equal(await routes.getByText("output_config.effort", { exact: true }).isVisible(), true);
  assert.match(await routes.innerText(), /claude-opus-5-5-kiro/);
  assert.match(await routes.innerText(), /claude-opus-5-5 → Synthetic channel/);
  assert.match(await routes.innerText(), /230 ms/);
  assert.match(await routes.innerText(), /尝试 ID: syntheti/);
  assert.match(await routes.innerText(), /fixture-vers/);
  const toggle = routes.getByRole("switch", { name: "同模型渠道故障切换" });
  assert.equal(await toggle.getAttribute("aria-checked"), "false");
  await toggle.click();
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="同模型渠道故障切换"]')?.getAttribute("aria-checked") === "true");
  const catalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await catalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  const modelSummary = catalog.locator("summary").filter({ hasText: "Synthetic channel" });
  await modelSummary.waitFor();
  await modelSummary.click();
  assert.equal(await catalog.getByText("My custom alias", { exact: false }).isVisible(), true);
  assert.match(await catalog.innerText(), /继续使用上一份有效目录/);
  assert.match(await catalog.innerText(), /1,000,000/);
  assert.match(await catalog.innerText(), /内置模型规则/);
  assert.match(await catalog.innerText(), /anthropic\/claude-opus-5-5/);
  assert.match(await catalog.innerText(), /reasoning_content/);
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
    )].every(element => /, 0\.82\)$/.test(getComputedStyle(element).backgroundColor)));
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
  console.log("PASS routing trace exposes requested/sent values, failover is opt-in, and capability sources survive stale cache");

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
  assert.match(await nativeCatalog.innerText(), /阈值 900,000/);
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
    const totals = (costMicros, pricedCount, partialCount = 0, unpricedCount = 0, coverageUnknownCount = 0) => ({
      totalTokens: 100000, inputTokens: 50000, outputTokens: 50000, cacheReadTokens: 0, cacheWriteTokens: 0,
      reasoningTokens: 0, requestCount: pricedCount + unpricedCount, successfulCount: pricedCount + unpricedCount,
      reportedCount: pricedCount + unpricedCount, costMicros, pricedCount, unpricedCount, partialCount,
      coverageUnknownCount, fullyPricedCount: pricedCount - partialCount - coverageUnknownCount,
    });
    window.review.relayPatch = {
      today: totals(1000000, 1, 1), lastSevenDays: totals(4000000, 3, 1, 1, 1),
      allTime: totals(4000000, 3, 1, 1, 1), week: totals(2000000, 2),
      daily: [{ date: "2026-10-02", totals: totals(1000000, 1, 1) }],
      byProvider: [{ providerId: "group-a", providerName: "Relay Alpha", totals: totals(4000000, 3, 1, 1, 1) }],
      byModel: [
        { providerId: "group-a", providerName: "Relay Alpha", model: "same-model", totals: totals(1000000, 1, 1) },
        { providerId: "other", providerName: "Other", model: "same-model", totals: totals(0, 0, 0, 1) },
        { providerId: "legacy", providerName: "Legacy", model: "legacy-model", totals: totals(3000000, 1, 0, 0, 1) },
        { providerId: "free", providerName: "Free", model: "free-model", totals: totals(0, 1) },
        { providerId: "unknown", providerName: "Unknown", model: "unknown-model", totals: totals(0, 0, 0, 1) },
      ],
    };
    window.review.mount("usage");
  });
  const usage = page.locator(".usage-relay-panel");
  await usage.getByText("1 笔部分计价", { exact: false }).first().waitFor();
  const sevenDays = usage.locator(".usage-relay-summary").filter({ hasText: "近七日预估费用" });
  assert.match(await sevenDays.innerText(), /\$4\.00/);
  assert.match(await sevenDays.innerText(), /1 笔部分计价/);
  assert.match(await sevenDays.innerText(), /1 笔历史计价完整性未知/);
  assert.match(await usage.locator(".usage-relay-day-details").innerText(), /1 笔部分计价/);
  await usage.getByRole("tab", { name: "模型", exact: true }).click();
  const sameModel = usage.locator(".usage-model-row").filter({ hasText: "same-model" });
  assert.match(await sameModel.innerText(), /1 笔部分计价/);
  assert.match(await sameModel.innerText(), /1 笔未计价/);
  assert.match(await usage.locator(".usage-model-row").filter({ hasText: "free-model" }).innerText(), /\$0/);
  assert.match(await usage.locator(".usage-model-row").filter({ hasText: "unknown-model" }).innerText(), /无法可靠计算成本/);
  await usage.getByRole("button", { name: "明细", exact: true }).click();
  assert.match(await usage.locator(".usage-model-detail-row").filter({ hasText: "same-model" }).innerText(), /1 笔部分计价/);
  await usage.getByRole("tab", { name: "供应商与额度" }).click();
  assert.match(await usage.locator(".usage-cost-line").innerText(), /1 笔部分计价/);
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  await usage.getByRole("tab", { name: "模型", exact: true }).click();
  assert.equal(await usage.evaluate(element => element.scrollWidth <= element.clientWidth), true, "pricing coverage must wrap within the model panel");
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS seven-day costs and partial/unknown coverage survive summaries, trends, merged model views and narrow screens");

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
  assert.equal(await desktopRoutes.locator("summary").count(), 2);
  await desktopRoutes.getByRole("combobox", { name: "客户端来源" }).click();
  await page.getByRole("option", { name: "Claude Desktop", exact: true }).click();
  await desktopRoutes.getByText("claude-sonnet-4-6 → deepseek-fixture", { exact: true }).waitFor();
  assert.equal(await desktopRoutes.locator("summary").count(), 1);
  assert.match(await desktopRoutes.locator("summary").innerText(), /claude-sonnet-4-6 → deepseek-fixture/);
  await desktopRoutes.locator("summary").click();
  await desktopRoutes.getByText("Claude Desktop 角色映射", { exact: true }).waitFor();
  assert.doesNotMatch(await desktopRoutes.innerText(), /desktop_role_mapping/,
    "Desktop mapping reasons must be localized in expanded request diagnostics");
  await desktopRoutes.getByRole("combobox", { name: "客户端来源" }).click();
  await page.getByRole("option", { name: "其他 / 未记录", exact: true }).click();
  await desktopRoutes.getByText("relay::claude-opus-5-5-kiro", { exact: true }).waitFor();
  assert.equal(await desktopRoutes.locator("summary").count(), 1);
  assert.match(await desktopRoutes.locator("summary").innerText(), /relay::claude-opus-5-5-kiro/);

  await reset("usage");
  await page.evaluate(() => {
    const base = { timestamp: 1790000000, date: "2026-10-05", providerId: "fixture", providerName: "Relay Fixture",
      model: "deepseek-fixture", protocol: "chatCompletions", statusCode: 200, durationMs: 20, success: true,
      usageReported: true, inputTokens: 3, outputTokens: 4, totalTokens: 7, reasoningTokens: 0,
      cacheReadTokens: 0, cacheWriteTokens: 0, costUsd: 0, pricingStatus: "priced", pricingSource: "override", catalogRevision: "fixture" };
    window.review.relayPatch = { recent: [base, { ...base, timestamp: 1790000001, clientSource: "claudeDesktop",
      requestedModel: "claude-sonnet-4-6", resolvedModel: "deepseek-fixture", routeRevision: 3 }] };
    window.review.mount("usage");
  });
  const desktopUsage = page.locator(".usage-relay-panel");
  await desktopUsage.getByRole("tab", { name: "请求明细" }).click();
  await desktopUsage.locator("tbody tr").nth(1).waitFor();
  assert.equal(await desktopUsage.locator("tbody tr").count(), 2);
  await desktopUsage.getByLabel("客户端来源", { exact: true }).selectOption("claudeDesktop");
  await desktopUsage.getByText("claude-sonnet-4-6 → deepseek-fixture", { exact: true }).waitFor();
  assert.equal(await desktopUsage.locator("tbody tr").count(), 1);
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /claude-sonnet-4-6 → deepseek-fixture/);
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /Claude Desktop/);
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /\$0/);
  await desktopUsage.getByLabel("客户端来源", { exact: true }).selectOption("other");
  await desktopUsage.getByText("未记录", { exact: true }).waitFor();
  assert.equal(await desktopUsage.locator("tbody tr").count(), 1);
  assert.match(await desktopUsage.locator("tbody tr").innerText(), /未记录/);
  console.log("PASS Desktop diagnostics and usage filter client source while retaining alias, actual billing model and zero cost");
}
