import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// Synthetic memory lists only; nothing asks Windows for consent.
export async function testMemoryOptimizer({ page, reset, captureDir }) {
  const t = (key, options) => page.evaluate(([key, options]) => window.review.t(key, options), [key, options]);
  const card = page.locator("[data-memory-optimizer]");
  const area = id => page.locator(`[data-memory-area="${id}"] [role="checkbox"]`);
  const status = card.getByRole("status");
  const mount = async (options = {}, shown = true) => {
    await reset("compact");
    await page.evaluate(value => { Object.assign(window.review, value); window.review.mount("memory"); }, options);
    if (shown) await card.waitFor();
  };

  await mount();
  await eventually(async () => assert.match(await page.locator("[data-memory-meter]").innerText(), /^58\s*%/));
  assert.match(await page.locator("[data-memory-meter]").innerText(), /9\.3 GB/);
  for (const id of ["standbyList", "modifiedList"]) assert.equal(await area(id).getAttribute("aria-checked"), "false", `${id} is opt-in`);
  for (const id of ["workingSets", "lowPriorityStandbyList", "combinePages"]) assert.equal(await area(id).getAttribute("aria-checked"), "true");
  assert.equal(await card.getByText(await t("maintenance.memory.mayStall")).count(), 2);
  await card.getByText(await t("maintenance.memory.needsAdmin"), { exact: false }).waitFor();
  console.log("PASS memory optimization shows the page lists and keeps the stall-prone ones opt-in");

  await area("standbyList").click();
  await eventually(async () => assert.deepEqual(await page.evaluate(() => window.review.memoryCalls.areas.at(-1)),
    ["workingSets", "systemFileCache", "modifiedFileCache", "standbyList", "lowPriorityStandbyList", "registryCache", "combinePages"]));
  await card.getByRole("button", { name: await t("maintenance.memory.restoreDefaults") }).click();
  await eventually(async () => assert.equal(await area("standbyList").getAttribute("aria-checked"), "false"));
  console.log("PASS a chosen area is saved in run order and the defaults come back in one click");

  await page.evaluate(() => { window.review.memoryHold = true; });
  await card.getByRole("button", { name: await t("maintenance.memory.action") }).click();
  await card.getByRole("button", { name: await t("maintenance.memory.waitingConsent") }).waitFor();
  assert.equal(await card.getAttribute("data-memory-optimizer"), "running");
  assert.equal(await area("workingSets").isDisabled(), true, "areas stay put while it runs");
  await page.evaluate(() => { window.review.memoryHold = false; window.review.memoryResolve(); });
  const optimized = await t("maintenance.memory.optimized", { released: "2.0 GB", freed: "4.0 GB", available: "8.8 GB" });
  await eventually(async () => assert.equal(await status.innerText(), optimized));
  assert.deepEqual(await page.evaluate(() => window.review.memoryCalls.optimize),
    [["workingSets", "systemFileCache", "modifiedFileCache", "lowPriorityStandbyList", "registryCache", "combinePages"]]);
  await eventually(async () => assert.match(await page.locator("[data-memory-meter]").innerText(), /^45\s*%/));
  if (captureDir) {
    await card.scrollIntoViewIfNeeded();
    await card.screenshot({ path: path.join(captureDir, "memory-optimizer.png") });
  }
  console.log("PASS optimizing waits for consent, then reports what was released and refreshes the lists");

  await mount({ memoryFailure: "maintenance.memory.errors.cancelled" });
  await card.getByRole("button", { name: await t("maintenance.memory.action") }).click();
  await eventually(async () => assert.equal(await status.innerText(), await t("maintenance.memory.errors.cancelled")));
  assert.match(await status.getAttribute("class"), /bg-muted/, "declining consent is not shown as an error");
  await mount({ memoryFailure: undefined, memoryFailed: ["registryCache"] });
  await card.getByRole("button", { name: await t("maintenance.memory.action") }).click();
  await eventually(async () => assert.match(await status.innerText(),
    new RegExp(await t("maintenance.memory.partlyFailed", { areas: await t("maintenance.memory.areas.registryCache.label") }))));
  console.log("PASS declined consent and refused steps are told apart from success");

  await mount({ memoryFailed: undefined, memoryPatch: { elevated: true } });
  await card.getByText(await t("maintenance.memory.elevated"), { exact: false }).waitFor();
  await page.waitForFunction(() => Boolean(window.review.emitMemoryOptimized));
  await page.evaluate(() => {
    const gib = 1024 ** 3;
    const lists = (inUse, standby, free) => ({ total: 16 * gib, inUse, modified: 0, standby, free });
    window.review.emitMemoryOptimized({ before: lists(9 * gib, 5 * gib, 2 * gib), after: lists(9 * gib, 5 * gib, 2 * gib), failed: [] });
  });
  await eventually(async () => assert.equal(await status.innerText(), await t("maintenance.memory.alreadyLean", { available: "7.0 GB" })));
  console.log("PASS an elevated app optimizes without a prompt and a tray run's outcome shows on the page");

  await mount({ memoryPatch: { supported: false } }, false);
  await page.waitForFunction(() => window.review.memoryCalls !== undefined);
  await page.waitForTimeout(200);
  await eventually(async () => assert.equal(await card.count(), 0));
  console.log("PASS memory optimization is hidden where it is unsupported");
}
