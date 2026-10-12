import assert from "node:assert/strict";
import { eventually } from "./eventually.mjs";

// Synthetic autostart states only; nothing touches the registry.
export async function testAutostart({ page, reset }) {
  const t = key => page.evaluate(key => window.review.t(key), key);
  const mount = async (options = {}) => {
    await reset("compact");
    await page.evaluate(value => { Object.assign(window.review, value); window.review.mount("autostart"); }, options);
    await page.locator("[data-autostart]").waitFor();
  };
  const toggle = async () => page.getByRole("switch", { name: await t("settings.autostart.title") });

  await mount();
  assert.equal(await page.locator("[data-autostart]").getAttribute("data-autostart"), "off");
  assert.equal(await (await toggle()).getAttribute("aria-checked"), "false");
  await (await toggle()).click();
  await page.locator('[data-autostart="on"]').waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.autostartCalls), [true]);
  await (await toggle()).click();
  await page.locator('[data-autostart="off"]').waitFor();
  console.log("PASS start at sign-in turns on and off from settings");

  await mount({ autostartPatch: { enabled: true, blocked: true } });
  assert.equal(await (await toggle()).getAttribute("aria-checked"), "false", "a startup entry Task Manager turned off is off");
  await page.getByText(await t("settings.autostart.blocked")).waitFor();
  await (await toggle()).click();
  await page.locator('[data-autostart="on"]').waitFor();
  assert.equal(await page.getByText(await t("settings.autostart.blocked")).count(), 0);
  console.log("PASS a startup entry turned off in Task Manager says so and turns on again from here");

  await mount({ autostartFailure: true });
  await (await toggle()).click();
  const failed = await t("settings.autostart.failed");
  await page.waitForFunction(title => window.review.lastToast === title, failed);
  assert.equal(await page.locator("[data-autostart]").getAttribute("data-autostart"), "off");
  console.log("PASS a failed write is reported and leaves the switch as it was");

  await reset("compact");
  await page.evaluate(() => { window.review.autostartPatch = { supported: false }; window.review.mount("autostart"); });
  await page.waitForFunction(() => window.review.autostartCalls !== undefined);
  await page.waitForTimeout(200);
  await eventually(async () => assert.equal(await page.locator("[data-autostart]").count(), 0, "no row where the platform has no startup entry"));
  console.log("PASS start at sign-in is hidden where it is unsupported");
}
