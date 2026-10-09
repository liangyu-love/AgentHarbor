import assert from "node:assert/strict";

export async function testCodexRoutingTransitions({ page, reset }) {
  const mount = async (options = {}) => {
    await reset("compact");
    await page.evaluate(value => {
      Object.assign(window.review, { routerPatch: { enabled: true, accessMode: "router" }, ...value });
      window.review.mount("access");
      window.review.routedThreads = true;
    }, options);
    await page.waitForFunction(() => window.review.getState().router.enabled);
  };
  await mount();
  const labels = await page.evaluate(() => ({
    access: window.review.t("newapi.codexAccess"),
    native: window.review.t("newapi.accessNative"),
    cancel: window.review.t("common.cancel"),
    restore: window.review.t("newapi.disableRoutingRestore"),
    nativeRestore: window.review.t("newapi.nativeRoutingRestore"),
    force: window.review.t("newapi.forceDisableRouting"),
    nativeForce: window.review.t("newapi.forceNativeRouting"),
    blocked: window.review.t("newapi.routingRestoreBlockedTitle"),
    reopenFailed: window.review.t("threads.routerAdoptionOpenFailed"),
  }));
  const alert = () => page.getByRole("alertdialog");
  const begin = async native => {
    if (native) {
      await page.locator("#codex-access-mode").click();
      await page.getByRole("option", { name: labels.native, exact: true }).click();
    } else await page.getByRole("switch", { name: labels.access, exact: true }).click();
    await alert().getByRole("button", { name: native ? labels.nativeRestore : labels.restore, exact: true }).click();
  };
  const expected = (native, force = false) => native
    ? { operation: "accessMode", mode: "native", force }
    : { operation: "enabled", enabled: false, force };
  const completed = native => page.waitForFunction(value => value
    ? window.review.getState().router.accessMode === "native"
    : !window.review.getState().router.enabled, native);

  for (const native of [false, true]) {
    await mount();
    await begin(native);
    await completed(native);
    assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [expected(native)]);
    assert.deepEqual(await page.evaluate(() => window.review.batchCalls), []);
  }
  console.log("PASS disabling routing and selecting native access delegate thread migration to one backend command");

  for (const native of [false, true]) {
    await mount({ routingMutationFailure: "CODEX_ROUTING_RESTORE_FAILED: synthetic thread is locked" });
    await begin(native);
    await alert().getByRole("heading", { name: labels.blocked, exact: true }).waitFor();
    assert.deepEqual(await page.evaluate(() => ({ enabled: window.review.getState().router.enabled,
      mode: window.review.getState().router.accessMode })), { enabled: true, mode: "router" });
    await alert().getByRole("button", { name: labels.cancel, exact: true }).click();
    await alert().waitFor({ state: "detached" });
    assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [expected(native)]);
    assert.equal(await page.getByRole("switch", { name: labels.access, exact: true }).isChecked(), true);
    await begin(native);
    await alert().getByRole("heading", { name: labels.blocked, exact: true }).waitFor();
    await alert().getByRole("button", { name: native ? labels.nativeForce : labels.force, exact: true }).click();
    await completed(native);
    assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [expected(native), expected(native), expected(native, true)]);
    assert.deepEqual(await page.evaluate(() => window.review.batchCalls), []);
  }
  console.log("PASS failed migration keeps routing, cancelling the force choice preserves it and confirming retries with force=true");

  for (const native of [false, true]) {
    await mount({ routeOverviewFailure: "synthetic unreadable thread index",
      routingMutationFailure: "CODEX_ROUTING_RESTORE_FAILED: synthetic unreadable thread index" });
    if (native) {
      await page.locator("#codex-access-mode").click();
      await page.getByRole("option", { name: labels.native, exact: true }).click();
    } else await page.getByRole("switch", { name: labels.access, exact: true }).click();
    await alert().getByRole("heading", { name: labels.blocked, exact: true }).waitFor();
    assert.deepEqual(await page.evaluate(() => ({ enabled: window.review.getState().router.enabled,
      mode: window.review.getState().router.accessMode })), { enabled: true, mode: "router" });
    assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [expected(native)]);
    await alert().getByRole("button", { name: native ? labels.nativeForce : labels.force, exact: true }).click();
    await completed(native);
    assert.deepEqual(await page.evaluate(() => window.review.routingCalls), [expected(native), expected(native, true)]);
    assert.equal(await page.evaluate(() => window.review.routedThreads), true,
      "forcing a connection change must leave the unmigrated thread records for recovery");
  }
  console.log("PASS unreadable thread overviews defer to the backend, retain routing on failure and allow explicit forced disable or native access");

  await mount({ routingMutationFailure: "newapi.errors.storage" });
  await begin(false);
  await alert().waitFor({ state: "detached" });
  await page.waitForFunction(() => window.review.routingCalls.length === 1);
  assert.equal(await alert().count(), 0);
  assert.equal(await page.getByRole("switch", { name: labels.access, exact: true }).isChecked(), true);
  console.log("PASS unrelated configuration failures do not offer forced route removal");

  for (const native of [false, true]) {
    await mount({ routingMigrationResult: { changed: 1, unchanged: 0, failed: [], reopenError: "synthetic launch failure" } });
    await begin(native);
    await completed(native);
    await page.waitForFunction(title => window.review.lastToast === title, labels.reopenFailed);
  }
  console.log("PASS migrated threads retain a visible warning when Codex cannot be reopened");
}
