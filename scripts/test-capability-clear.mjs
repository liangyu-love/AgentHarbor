import assert from "node:assert/strict";
import path from "node:path";

export async function testCapabilityClear({ page, reset, captureDir }) {
  const open = async () => {
    await reset("access");
    const catalog = page.getByRole("region", { name: "模型能力目录与匹配预览", exact: true });
    await catalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
    const summary = catalog.locator("summary").filter({ hasText: "Synthetic channel" });
    await summary.waitFor();
    await summary.click();
    return catalog.locator("[data-capability-clear]");
  };
  const choose = (clear, field) => clear.getByLabel("覆盖项", { exact: true }).selectOption(field);
  const preview = clear => clear.getByRole("button", { name: "预览", exact: true }).click();
  const apply = clear => clear.getByRole("button", { name: "确认清除", exact: true });

  let clear = await open();
  assert.equal(await clear.locator("optgroup").count(), 2);
  await page.evaluate(() => { window.review.clearPreviewHold = true; });
  await preview(clear);
  await page.waitForFunction(() => window.review.clearPreviewResolvers?.length === 1);
  await choose(clear, "price");
  await page.evaluate(() => { window.review.clearPreviewHold = false; });
  await preview(clear);
  await clear.locator('[data-clear-row="price"]').waitFor();
  await page.evaluate(() => window.review.clearPreviewResolvers[0]());
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await clear.locator('[data-clear-row="contextWindow"]').count(), 0, "late context preview must not replace the price preview");
  assert.match(await clear.locator('[data-clear-row="price"]').innerText(), /缓存读取 \$0\.4[\s\S]*缓存读取 \$0\.2/);
  await apply(clear).click();
  await page.waitForFunction(() => window.review.clearApplyCalls?.length === 1);
  assert.deepEqual(await page.evaluate(() => {
    const reviewed = window.review.clearApplyCalls[0];
    return { field: reviewed.field, provider: reviewed.before.providerId, model: reviewed.before.upstreamModel };
  }), { field: "price", provider: "relay", model: "claude-opus-5-5-kiro" });
  console.log("PASS clearing uses the reviewed field/model identity, discards late previews, and exposes cache-only prices");

  clear = await open();
  await page.evaluate(() => { window.review.clearNoOverride = true; });
  await preview(clear);
  await clear.getByText("此项没有可清除的覆盖。", { exact: true }).waitFor();
  assert.equal(await apply(clear).isDisabled(), true);
  await page.evaluate(() => { window.review.clearNoOverride = false; window.review.clearPreviewFailure = true; });
  await preview(clear);
  await clear.getByRole("alert").waitFor();
  assert.equal(await clear.locator("[data-clear-result]").count(), 0, "failed previews must drop the previous apply target");
  await page.evaluate(() => { window.review.clearPreviewFailure = false; });
  await preview(clear);
  await apply(clear).waitFor();
  await page.evaluate(() => { window.review.clearApplyFailure = true; });
  await apply(clear).click();
  await clear.getByRole("alert").waitFor();
  assert.equal(await apply(clear).isEnabled(), true, "failed writes remain retryable with the reviewed result");
  await page.evaluate(() => { window.review.clearApplyFailure = false; window.review.clearApplyHold = true; window.review.clearApplyStale = true; });
  await apply(clear).evaluate(button => { button.click(); button.click(); });
  await page.waitForFunction(() => window.review.clearApplyCalls.length === 2);
  assert.equal(await clear.getByLabel("覆盖项", { exact: true }).isDisabled(), true);
  await page.evaluate(() => window.review.clearApplyResolve());
  await clear.getByText("配置或目录已变化，预览已更新；请重新检查后应用。", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.clearApplyCalls.length), 2, "stale previews must not apply automatically");
  await page.evaluate(() => { window.review.clearApplyHold = false; });
  await apply(clear).click();
  await clear.getByText("覆盖已清除。", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.clearApplyCalls.at(-1).configRevision), "config-clear-2");
  console.log("PASS absent overrides cannot write, errors retry, duplicate submits are blocked, and stale previews require review");

  clear = await open();
  await preview(clear);
  await clear.locator('[data-clear-row="clientOutput"]').waitFor();
  assert.match(await clear.locator('[data-clear-row="clientOutput"]').innerText(), /200,000[\s\S]*100,000/);
  assert.equal(await clear.locator('[data-clear-row="outputLimit"]').count(), 0, "unchanged raw metadata is hidden while its changed client clamp stays visible");
  await clear.getByRole("button", { name: "完整结果", exact: true }).click();
  assert.equal(await clear.locator('[data-clear-row="outputLimit"]').isVisible(), true);
  await page.evaluate(() => { window.review.clearSharedEffect = true; });
  await preview(clear);
  await clear.getByText("受影响模型（3）", { exact: true }).waitFor();
  await page.evaluate(() => { window.review.clearSharedEffect = false; });
  await choose(clear, "providerCompression");
  await preview(clear);
  await clear.locator('[data-clear-row="sharedCompression"]').waitFor();
  assert.match(await clear.locator('[data-clear-row="sharedCompression"]').innerText(), /90,000[\s\S]*80,000/);
  await clear.getByText("受影响模型（3）", { exact: true }).click();
  assert.equal(await clear.locator("li").count(), 3);
  await clear.getByText("small", { exact: true }).waitFor();
  assert.match(await clear.innerText(), /3 \/ 3 个模型结果变化/);
  if (captureDir) await clear.screenshot({ path: path.join(captureDir, "capability-clear-1200-light.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  assert.equal(await clear.evaluate(element => element.scrollWidth <= element.clientWidth), true);
  if (captureDir) await clear.screenshot({ path: path.join(captureDir, "capability-clear-390-dark.png") });
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS projected output clamps, native shared compression, distinct model IDs and narrow themed comparison layouts");

  clear = await open();
  await page.evaluate(() => { window.review.clearPreviewHold = true; });
  await preview(clear);
  await page.waitForFunction(() => window.review.clearPreviewResolvers?.length === 1);
  await page.evaluate(() => window.review.frontendClient.setQueryData(["model-capabilities"], current => ({
    ...current, models: current.models.map(model => ({ ...model, context: { ...model.context, value: 64000 } })),
  })));
  await page.evaluate(() => window.review.clearPreviewResolvers[0]());
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  assert.equal(await clear.locator("[data-clear-result]").count(), 0, "a refreshed model invalidates the pending preview");
  assert.equal(await clear.getByRole("button", { name: "预览", exact: true }).isEnabled(), true);
  await page.evaluate(() => { window.review.clearPreviewHold = false; });
  await preview(clear);
  await apply(clear).waitFor();
  await page.evaluate(() => { window.review.clearApplyHold = true; });
  await apply(clear).click();
  await page.waitForFunction(() => window.review.clearApplyCalls?.length === 1);
  await page.evaluate(() => window.review.frontendClient.setQueryData(["model-capabilities"], current => ({
    ...current, models: current.models.map(model => ({ ...model, context: { ...model.context, value: 32000 } })),
  })));
  assert.equal(await clear.getByLabel("覆盖项", { exact: true }).isDisabled(), true, "a model refresh cannot unlock an ongoing write");
  await page.evaluate(() => window.review.clearApplyResolve());
  await clear.getByText("覆盖已清除。", { exact: true }).waitFor();
  console.log("PASS model refresh invalidates old previews while preserving the lock and outcome of an ongoing write");
}
