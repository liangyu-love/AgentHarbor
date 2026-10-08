import assert from "node:assert/strict";
import path from "node:path";

export async function testPriceCache({ page, reset, captureDir }) {
  await reset("provider");
  await page.evaluate(() => {
    window.review.dialogProviderFixture = { ...window.review.getState().providers[0], priceMultiplier: 0.5 };
    window.review.mount("provider");
  });
  const dialog = page.getByRole("dialog");
  const multiplier = dialog.getByLabel("价格倍率（1 为原价）", { exact: true });
  await multiplier.waitFor();
  assert.equal(await multiplier.inputValue(), "0.5");
  await multiplier.fill("0");
  if (captureDir) await dialog.screenshot({ path: path.join(captureDir, "price-multiplier.png") });
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.saved?.priceMultiplier === 0);
  await page.locator("#reopen-saved").click();
  await multiplier.waitFor();
  assert.equal(await multiplier.inputValue(), "0");
  await multiplier.fill("1");
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.saved?.priceMultiplier === 1);
  console.log("PASS provider price multiplier persists zero and restores base rates with one");

  await reset("access");
  await page.evaluate(() => {
    window.review.capabilityVariants = [{ price: { currency: "USD", unit: "perMillionTokens", input: 2.5, output: 10,
      cacheRead: 0.25, cacheWrite: null, tiers: [], source: "catalogReference", multiplier: 0.5, complete: true } }];
    window.review.mount("access");
  });
  const catalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await catalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await catalog.locator("summary").click();
  assert.match(await catalog.innerText(), /倍率 ×0.5/);
  assert.match(await catalog.innerText(), /2.5 \/ 10/);
  console.log("PASS automatic model prices expose the applied channel multiplier and resulting rates");

  await reset("usage");
  await page.evaluate(() => {
    const stats = { reportedCount: 3, promptTokens: 1000, readTokens: 80, writeTokens: 10 };
    const totals = { requestCount: 4, successfulCount: 4, reportedCount: 4, inputTokens: 4910, outputTokens: 0,
      totalTokens: 5000, reasoningTokens: 0, cacheReadTokens: 80, cacheWriteTokens: 10, costMicros: 0,
      pricedCount: 4, unpricedCount: 0, partialCount: 0, fullyPricedCount: 4, coverageUnknownCount: 0 };
    const known = { ...totals, cacheUsage: stats };
    const base = { timestamp: 1790000000, date: "2026-10-08", providerId: "actual-member", providerName: "Actual relay member",
      model: "cache-hit", protocol: "responses", statusCode: 200, durationMs: 20, success: true, clientSource: "codex",
      usageReported: true, cacheReported: true, inputTokens: 10, outputTokens: 0, totalTokens: 100,
      reasoningTokens: 0, cacheReadTokens: 80, cacheWriteTokens: 10, costUsd: 0, pricingStatus: "priced",
      pricingSource: "catalogReference", pricingSnapshot: { multiplier: 0.5 } };
    window.review.relayPatch = {
      today: { ...totals, cacheUsage: { reportedCount: 1, promptTokens: 900, readTokens: 0, writeTokens: 0 } },
      lastSevenDays: totals, allTime: known,
      byProvider: [{ providerId: "actual-member", providerName: "Actual relay member", totals: known },
        { providerId: "unknown", providerName: "Unreported relay", totals }],
      byModel: [{ providerId: "actual-member", providerName: "Actual relay member", model: "cache-hit", totals: known },
        { providerId: "unknown", providerName: "Unreported relay", model: "unknown-model", totals }],
      recent: [base, { ...base, model: "cache-miss", inputTokens: 900, totalTokens: 900, cacheReadTokens: 0, cacheWriteTokens: 0 },
        { ...base, model: "unknown-model", cacheReported: false, inputTokens: 4000, totalTokens: 4000, cacheReadTokens: 0, cacheWriteTokens: 0 },
        { ...base, model: "empty-input", inputTokens: 0, totalTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }],
    };
    window.review.mount("usage");
  });
  const usage = page.locator(".usage-relay-panel");
  await usage.getByRole("tab", { name: "请求明细", exact: true }).click();
  const row = name => usage.locator("tbody tr").filter({ hasText: name });
  assert.match(await row("cache-hit").innerText(), /缓存命中率 80%/);
  assert.match(await row("cache-hit").innerText(), /读取 80 · 写入 10/);
  assert.match(await row("cache-miss").innerText(), /缓存命中率 0%/);
  assert.match(await row("unknown-model").innerText(), /缓存命中率 —/);
  assert.match(await row("empty-input").innerText(), /缓存命中率 —/);
  assert.match(await row("cache-hit").locator("td").nth(5).getAttribute("title"), /倍率 ×0.5/);
  if (captureDir) await usage.screenshot({ path: path.join(captureDir, "cache-request-detail.png") });
  await usage.getByRole("tab", { name: "缓存", exact: true }).click();
  const cache = usage.getByTestId("cache-usage-panel");
  await cache.waitFor();
  const rates = await cache.locator("strong").allTextContents();
  assert.deepEqual(rates, ["0%", "—", "8%"], "aggregate ratio is token weighted, excluding unreported requests");
  assert.match(await row("Actual relay member").innerText(), /8%/);
  assert.match(await row("Actual relay member").innerText(), /3 \/ 4/);
  assert.deepEqual(await row("Unreported relay").locator("td").allTextContents(), ["Unreported relay", "—", "—", "—", "—", "0 / 4"]);
  await cache.getByRole("tab", { name: "按模型", exact: true }).click();
  assert.match(await cache.innerText(), /cache-hit/);
  assert.match(await cache.innerText(), /unknown-model/);
  if (captureDir) await usage.screenshot({ path: path.join(captureDir, "cache-usage-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  assert.equal(await usage.evaluate(element => element.scrollWidth <= element.clientWidth), true);
  if (captureDir) await usage.screenshot({ path: path.join(captureDir, "cache-usage-mobile-dark.png") });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS cache request details distinguish hits, misses and unknowns, with weighted totals, actual-member grouping and narrow-screen containment");
}
