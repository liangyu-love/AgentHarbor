import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testPriceCache({ page, reset, captureDir }) {
  await reset("provider");
  await page.evaluate(() => {
    window.review.dialogProviderFixture = { ...window.review.getState().providers[0], priceMultiplier: 0.5 };
    window.review.mount("provider");
  });
  const dialog = page.getByRole("dialog");
  const multiplier = dialog.getByLabel("价格倍率（1 为原价）", { exact: true });
  await multiplier.waitFor();
  await eventually(async () => assert.equal(await multiplier.inputValue(), "0.5"));
  await multiplier.fill("0");
  if (captureDir) await dialog.screenshot({ path: path.join(captureDir, "price-multiplier.png") });
  await dialog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.saved?.priceMultiplier === 0);
  await page.locator("#reopen-saved").click();
  await multiplier.waitFor();
  await eventually(async () => assert.equal(await multiplier.inputValue(), "0"));
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
  await eventually(async () => assert.match(await catalog.innerText(), /倍率 ×0.5/));
  assert.match(await catalog.innerText(), /2.5 \/ 10/);
  console.log("PASS automatic model prices expose the applied channel multiplier and resulting rates");

  await reset("usage");
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    const request = { statusCode: 200, success: true, durationMs: 20, modelMatchStatus: "unknown", credentialStatus: "recorded", matched: false };
    const base = { timestamp: now - 60, client: "codex", channel: "relay", providerId: "actual-member", providerName: "Actual relay member",
      model: "cache-hit", inputTokens: 10, outputTokens: 0, cacheReadTokens: 80, cacheWriteTokens: 10, totalTokens: 100, costUsd: 0,
      pricingStatus: "priced", pricingSource: "catalogReference", multiplier: 0.5, estimated: false, request };
    window.review.usageEntries = [base,
      { ...base, model: "cache-miss", inputTokens: 900, totalTokens: 900, cacheReadTokens: 0, cacheWriteTokens: 0 },
      { ...base, model: "unknown-model", cacheReported: false, inputTokens: 4000, totalTokens: 4000, cacheReadTokens: 0, cacheWriteTokens: 0 },
      { ...base, model: "empty-input", inputTokens: 0, totalTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 },
      { ...base, providerId: "unreported", providerName: "Unreported relay", model: "unreported-model", cacheReported: false,
        inputTokens: 500, totalTokens: 500, cacheReadTokens: 0, cacheWriteTokens: 0 },
    ].map(entry => window.review.usageEntry(entry));
    window.review.mount("usage");
  });
  const usage = page.locator("[data-usage-requests]");
  const row = name => usage.locator("tbody tr").filter({ hasText: name });
  await row("cache-hit").waitFor();
  assert.match(await row("cache-hit").innerText(), /缓存命中率 80%/);
  assert.match(await row("cache-hit").innerText(), /读取 80 · 写入 10/);
  assert.match(await row("cache-miss").innerText(), /缓存命中率 0%/);
  assert.match(await row("unknown-model").innerText(), /缓存命中率 —/);
  assert.match(await row("empty-input").innerText(), /缓存命中率 —/);
  assert.match(await row("cache-hit").locator("td").nth(5).getAttribute("title"), /倍率 ×0.5/);
  if (captureDir) await usage.screenshot({ path: path.join(captureDir, "cache-request-detail.png") });
  // Aggregate ratios weigh tokens and leave requests without cache counters out.
  await eventually(async () => assert.match(await page.locator('[data-usage-kpi="cache"]').innerText(), /8%/));
  const channels = page.getByRole("region", { name: "渠道与供应商", exact: true });
  assert.match(await channels.locator('[data-usage-rank="relay:actual-member"]').innerText(), /缓存命中率 8%/);
  assert.doesNotMatch(await channels.locator('[data-usage-rank="relay:unreported"]').innerText(), /缓存命中率/);
  if (captureDir) await channels.screenshot({ path: path.join(captureDir, "cache-usage-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  assert.equal(await usage.evaluate(element => element.scrollWidth <= element.clientWidth), true);
  if (captureDir) await usage.screenshot({ path: path.join(captureDir, "cache-usage-mobile-dark.png") });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS cache request details distinguish hits, misses and unknowns, with weighted totals, actual-member grouping and narrow-screen containment");
}
