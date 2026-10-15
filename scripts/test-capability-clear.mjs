import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

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
  const choose = (clear, field) => clear.getByLabel("已手动设置", { exact: true }).selectOption(field);
  const preview = clear => clear.getByRole("button", { name: "预览恢复结果", exact: true }).click();
  const apply = clear => clear.getByRole("button", { name: "确认恢复自动值", exact: true });

  let clear = await open();
  assert.equal(await clear.locator("optgroup").count(), 2);
  assert.deepEqual(await clear.locator("option").evaluateAll(options => options.map(option => option.value)),
    ["contextWindow", "price", "compression", "providerContextWindow", "providerCompression"], "only stored manual settings can be restored");
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
  await page.waitForFunction(() => !document.querySelector('[data-capability-clear] option[value="price"]'));
  assert.deepEqual(await page.evaluate(() => {
    const reviewed = window.review.clearApplyCalls[0];
    return { field: reviewed.field, provider: reviewed.before.providerId, model: reviewed.before.upstreamModel };
  }), { field: "price", provider: "relay", model: "claude-opus-5-5-kiro" });
  console.log("PASS clearing uses the reviewed field/model identity, discards late previews, and exposes cache-only prices");

  clear = await open();
  await page.evaluate(() => { window.review.clearNoOverride = true; });
  await preview(clear);
  await clear.getByText("此项已没有手动设置，无需恢复。", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await apply(clear).isDisabled(), true));
  await page.evaluate(() => { window.review.clearNoOverride = false; window.review.clearPreviewFailure = true; });
  await preview(clear);
  await clear.getByRole("alert").waitFor();
  await eventually(async () => assert.equal(await clear.locator("[data-clear-result]").count(), 0, "failed previews must drop the previous apply target"));
  await page.evaluate(() => { window.review.clearPreviewFailure = false; });
  await preview(clear);
  await apply(clear).waitFor();
  await page.evaluate(() => { window.review.clearApplyFailure = true; });
  await apply(clear).click();
  await clear.getByRole("alert").waitFor();
  await eventually(async () => assert.equal(await apply(clear).isEnabled(), true, "failed writes remain retryable with the reviewed result"));
  await page.evaluate(() => { window.review.clearApplyFailure = false; window.review.clearApplyHold = true; window.review.clearApplyStale = true; });
  await apply(clear).evaluate(button => { button.click(); button.click(); });
  await page.waitForFunction(() => window.review.clearApplyCalls.length === 2);
  await eventually(async () => assert.equal(await clear.getByLabel("已手动设置", { exact: true }).isDisabled(), true));
  await page.evaluate(() => window.review.clearApplyResolve());
  await clear.getByText("配置或目录已变化，预览已更新；请重新检查后应用。", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.clearApplyCalls.length), 2, "stale previews must not apply automatically");
  await page.evaluate(() => { window.review.clearApplyHold = false; });
  await apply(clear).click();
  await clear.getByText("已恢复自动值。", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.review.clearApplyCalls.at(-1).configRevision), "config-clear-2");
  console.log("PASS absent overrides cannot write, errors retry, duplicate submits are blocked, and stale previews require review");

  clear = await open();
  await preview(clear);
  await clear.locator('[data-clear-row="clientOutput"]').waitFor();
  await eventually(async () => assert.match(await clear.locator('[data-clear-row="compression"]').innerText(), /270,000[\s\S]*90,000/));
  assert.match(await clear.locator('[data-clear-row="codexCompression"]').innerText(), /270,000[\s\S]*90,000/);
  assert.match(await clear.locator('[data-clear-row="clientOutput"]').innerText(), /200,000[\s\S]*100,000/);
  assert.equal(await clear.locator('[data-clear-row="outputLimit"]').count(), 0, "unchanged raw metadata is hidden while its changed client clamp stays visible");
  await clear.getByRole("button", { name: "完整结果", exact: true }).click();
  await eventually(async () => assert.equal(await clear.locator('[data-clear-row="outputLimit"]').isVisible(), true));
  await page.evaluate(() => { window.review.clearSharedEffect = true; });
  await preview(clear);
  await clear.getByText("受影响模型（3）", { exact: true }).waitFor();
  await page.evaluate(() => { window.review.clearSharedEffect = false; });
  await choose(clear, "providerCompression");
  await preview(clear);
  await clear.locator('[data-clear-row="sharedCompression"]').waitFor();
  await eventually(async () => assert.match(await clear.locator('[data-clear-row="sharedCompression"]').innerText(), /90,000[\s\S]*80,000/));
  assert.match(await clear.locator('[data-clear-row="codexCompression"]').innerText(), /90,000[\s\S]*80,000/);
  await clear.getByText("受影响模型（3）", { exact: true }).click();
  await eventually(async () => assert.equal(await clear.locator("li").count(), 3));
  await clear.getByText("small", { exact: true }).waitFor();
  await eventually(async () => assert.match(await clear.innerText(), /3 \/ 3 个模型结果变化/));
  await clear.evaluate(element => element.scrollIntoView({ block: "start" }));
  if (captureDir) await clear.screenshot({ path: path.join(captureDir, "capability-clear-1200-light.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await clear.evaluate(element => element.scrollIntoView({ block: "start" }));
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
  assert.equal(await clear.getByRole("button", { name: "预览恢复结果", exact: true }).isEnabled(), true);
  await page.evaluate(() => { window.review.clearPreviewHold = false; });
  await preview(clear);
  await apply(clear).waitFor();
  await page.evaluate(() => { window.review.clearApplyHold = true; });
  await apply(clear).click();
  await page.waitForFunction(() => window.review.clearApplyCalls?.length === 1);
  await page.evaluate(() => window.review.frontendClient.setQueryData(["model-capabilities"], current => ({
    ...current, models: current.models.map(model => ({ ...model, context: { ...model.context, value: 32000 } })),
  })));
  assert.equal(await clear.getByLabel("已手动设置", { exact: true }).isDisabled(), true, "a model refresh cannot unlock an ongoing write");
  await page.evaluate(() => window.review.frontendClient.setQueryData(["model-capabilities"], current => ({
    ...current, models: current.models.map(model => ({ ...model, manualOverrides: [] })),
  })));
  await clear.getByText("恢复中…", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await clear.locator("select").count(), 0, "removing every field during a write leaves progress visible without another action"));
  await page.evaluate(() => window.review.clearApplyResolve());
  await clear.getByText("已恢复自动值。", { exact: true }).waitFor();
  console.log("PASS model refresh invalidates old previews while preserving the lock and outcome of an ongoing write");

  clear = await open();
  await page.evaluate(() => window.review.frontendClient.setQueryData(["model-capabilities"], current => ({
    ...current, models: current.models.map(model => ({ ...model, manualOverrides: ["compression"] })),
  })));
  await page.waitForFunction(() => document.querySelector('[data-capability-clear] select')?.value === "compression");
  await eventually(async () => assert.equal(await clear.locator("option").count(), 1));
  assert.equal(await clear.locator("optgroup").count(), 1);
  await preview(clear);
  await clear.getByText("将移除手动设置；当前自动结果与现值相同。", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await apply(clear).isEnabled(), true, "a stored setting with an equal automatic value is still removable"));
  assert.equal(await page.evaluate(() => window.review.clearPreviewCalls.at(-1).field), "compression");

  clear = await open();
  await page.evaluate(() => window.review.frontendClient.setQueryData(["model-capabilities"], current => ({
    ...current, models: current.models.map(model => ({ ...model, manualOverrides: [] })),
  })));
  await clear.waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.review.clearPreviewCalls?.length ?? 0), 0);
  console.log("PASS fully automatic models expose no restore operation or unnecessary preview request");

  // The catalog lives on the relay page; each relay card opens it scoped to that relay.
  await reset("compact");
  await page.evaluate(() => {
    const base = window.review.getState().providers[0];
    window.review.providerFixtures = [
      { ...base, id: "relay", name: "Synthetic channel", baseUrl: "https://synthetic.invalid/v1" },
      { ...base, id: "other", name: "Other relay", baseUrl: "https://other.invalid/v1" },
    ];
    window.review.mount("access");
  });
  const catalog = page.locator("[data-model-capabilities]");
  assert.equal(await catalog.count(), 1, "only the relay page shows the catalog");
  assert.equal(await page.locator("[data-route-diagnostics] [data-model-capabilities]").count(), 0);
  await page.locator('[data-relay-capabilities="other"]').click();
  await catalog.locator('[data-capability-focus="other"]').waitFor();
  await catalog.getByText("仅看 Other relay", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await catalog.locator("[data-capability-row]").count(), 0, "other relays' models are filtered out"));
  await page.locator('[data-relay-capabilities="relay"]').click();
  await catalog.locator('[data-capability-focus="relay"]').waitFor();
  await eventually(async () => assert.equal(await catalog.locator("[data-capability-row]").count(), 1));
  await eventually(async () => assert.equal(await catalog.evaluate(node => {
    const box = node.getBoundingClientRect();
    return box.top < window.innerHeight && box.bottom > 0;
  }), true, "the catalog scrolls into view"));
  await catalog.getByRole("button", { name: "显示全部中转", exact: true }).click();
  await eventually(async () => assert.equal(await catalog.locator("[data-capability-focus]").count(), 0));
  await eventually(async () => assert.equal(await catalog.locator("[data-capability-row]").count(), 1));
  await page.evaluate(() => { delete window.review.providerFixtures; });
  console.log("PASS relay cards open the capability catalog scoped to their relay and the scope can be cleared");
}
