import assert from "node:assert/strict";

export async function testRoutingCache({ page, reset }) {
  await reset("routing-cache");
  const labels = await page.evaluate(() => ({
    access: window.review.t("newapi.codexAccess"),
    repair: window.review.t("threads.repairAll"),
    native: window.review.t("newapi.accessNative"),
    router: window.review.t("newapi.accessRouter"),
  }));
  const repair = () => page.getByRole("button", { name: labels.repair, exact: true }).first();
  const connections = () => page.getByRole("button", { name: "Review connections", exact: true }).click();
  const threads = () => page.getByRole("button", { name: "Review threads", exact: true }).click();
  const expectRouter = async active => {
    await page.waitForFunction(value => window.review.frontendClient.getQueryData(["threads", "codex", "routes"])?.routerActive === value, active);
    assert.equal(await repair().isEnabled(), active);
  };
  const changeMode = async (mode, label) => {
    await connections();
    await page.locator("#codex-access-mode").click();
    await page.getByRole("option", { name: label, exact: true }).click();
    await page.waitForFunction(value => window.review.getState().router.accessMode === value, mode);
    await threads();
  };

  await expectRouter(false);
  await connections();
  await page.getByRole("switch", { name: labels.access, exact: true }).click();
  await page.waitForFunction(() => window.review.getState().router.enabled);
  await threads();
  await expectRouter(true);

  await changeMode("native", labels.native);
  await expectRouter(false);
  await changeMode("router", labels.router);
  await expectRouter(true);

  await connections();
  await page.getByRole("switch", { name: labels.access, exact: true }).click();
  await page.waitForFunction(() => !window.review.getState().router.enabled);
  await threads();
  await expectRouter(false);
  console.log("PASS routing and access-mode changes refresh cached thread routing and batch-adoption controls without manual refresh");
}
