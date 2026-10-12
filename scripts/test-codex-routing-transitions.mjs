import assert from "node:assert/strict";
import { eventually } from "./eventually.mjs";

// Threads keep Codex's built-in provider, so connection changes are a single
// backend call that only writes config. A running Codex is restarted by the
// user, either from the prompt or later from the pending indicator.
export async function testCodexRoutingTransitions({ page, reset }) {
  const mount = async (options = {}) => {
    await reset("compact");
    await page.evaluate(value => {
      Object.assign(window.review, { routerPatch: { enabled: true, accessMode: "router" }, ...value });
      window.review.mount("access");
    }, options);
    await page.waitForFunction(() => window.review.getState().router.enabled);
  };
  // Past the desktop app's 100-model menu, the card names what it leaves out.
  await mount({ menuOverflow: { total: 108, limit: 100, cut: ["R/a", "R/b", "R/c", "R/d", "R/e", "R/f", "R/g", "R/h"] } });
  const overflow = page.locator("[data-codex-menu-overflow]");
  await overflow.waitFor();
  const said = await overflow.innerText();
  assert.match(said, /108/);
  assert.match(said, /R\/a、R\/b、R\/c、R\/d、R\/e、R\/f/);
  assert.doesNotMatch(said, /R\/g/, "six names, then 'and more'");
  await mount();
  assert.equal(await page.locator("[data-codex-menu-overflow]").count(), 0, "within the menu nothing is said");
  console.log("PASS the Codex card names the models the desktop app's 100-model menu leaves out");

  await mount();
  const labels = await page.evaluate(() => ({
    access: window.review.t("newapi.codexAccess"),
    native: window.review.t("newapi.accessNative"),
    restartHint: window.review.t("newapi.codexRestartHint"),
    promptTitle: window.review.t("newapi.restartPromptTitle"),
    now: window.review.t("newapi.restartNow"),
    later: window.review.t("newapi.restartLater"),
    pending: window.review.t("newapi.topology.restartPending"),
    pendingHint: window.review.t("newapi.restartPendingHint"),
  }));
  const begin = async native => {
    if (native) {
      await page.locator("#codex-access-mode").click();
      await page.getByRole("option", { name: labels.native, exact: true }).click();
    } else await page.getByRole("switch", { name: labels.access, exact: true }).click();
  };
  const expected = native => native
    ? { operation: "accessMode", mode: "native" }
    : { operation: "enabled", enabled: false };
  const completed = native => page.waitForFunction(value => value
    ? window.review.getState().router.accessMode === "native"
    : !window.review.getState().router.enabled, native);
  const prompt = () => page.getByRole("alertdialog").filter({ hasText: labels.promptTitle });

  for (const native of [false, true]) {
    await mount();
    await begin(native);
    await completed(native);
    await page.waitForFunction(value => window.review.lastToastDescription === value, labels.restartHint);
    await eventually(async () => assert.equal(await page.getByRole("alertdialog").count(), 0));
    assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [expected(native)]);
    assert.deepEqual(await page.evaluate(() => window.review.restartCalls ?? []), []);
  }
  console.log("PASS with Codex stopped, disabling routing or selecting native access only writes the configuration");

  for (const native of [false, true]) {
    await mount({ codexRunning: true });
    await page.evaluate(() => { window.review.restartCalls = []; });
    await begin(native);
    await prompt().waitFor();
    assert.deepEqual(await page.evaluate(() => window.review.restartCalls), [], "the change never restarts Codex by itself");
    await prompt().getByRole("button", { name: labels.later, exact: true }).click();
    await prompt().waitFor({ state: "detached" });
    // The pending state stays visible with a quick restart.
    await page.getByText(labels.pendingHint, { exact: true }).waitFor();
    await page.getByRole("button", { name: labels.pending }).click();
    await page.getByRole("alertdialog").getByRole("button").last().click();
    await page.waitForFunction(() => window.review.restartCalls.length === 1);
    assert.deepEqual(await page.evaluate(() => window.review.restartCalls), ["codex"]);
  }
  console.log("PASS a running Codex gets a restart prompt; choosing later keeps a pending indicator with quick restart");

  await mount({ codexRunning: true });
  await page.evaluate(() => { window.review.restartCalls = []; });
  await begin(false);
  await prompt().getByRole("button", { name: labels.now, exact: true }).click();
  await page.waitForFunction(() => window.review.restartCalls.length === 1);
  console.log("PASS the restart prompt can restart Codex immediately");

  await mount({ routingMutationFailure: "newapi.errors.storage" });
  await begin(false);
  await page.waitForFunction(() => window.review.routingCalls.length === 1);
  await eventually(async () => assert.equal(await page.getByRole("alertdialog").count(), 0));
  await eventually(async () => assert.equal(await page.getByRole("switch", { name: labels.access, exact: true }).isChecked(), true));
  console.log("PASS configuration failures leave routing unchanged without restart prompts");
}
