import assert from "node:assert/strict";
import path from "node:path";

export async function testSkillsActivation({ page, reset, captureDir }) {
  await reset("skills");
  const toggle = page.getByRole("switch", { name: "启用 Sample Skill", exact: true });
  const conflict = page.getByRole("switch", { name: "启用 Conflict Skill", exact: true });
  await toggle.waitFor();
  assert.equal(await toggle.getAttribute("aria-checked"), "true");
  assert.equal(await conflict.isDisabled(), true);
  await page.evaluate(() => { window.review.skillToggleHold = true; });
  await toggle.click();
  await page.waitForFunction(() => Boolean(window.review.skillToggleResolve));
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="启用 Sample Skill"]')?.disabled === true);
  assert.equal(await toggle.isDisabled(), true);
  assert.equal(await toggle.getAttribute("aria-checked"), "true", "retain real state while a write is pending");
  await page.evaluate(() => { window.review.skillToggleHold = false; window.review.skillToggleResolve(); });
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="启用 Sample Skill"]')?.getAttribute("aria-checked") === "false");
  assert.equal(await page.getByRole("heading", { name: "Sample Skill", exact: true }).count(), 1, "disabled skills remain listed");
  await page.evaluate(() => { window.review.skillToggleFailure = true; });
  await toggle.click();
  await page.waitForFunction(() => window.review.lastToast === "出错了");
  assert.equal(await toggle.getAttribute("aria-checked"), "false", "failed enable must not report success");
  await page.evaluate(() => { window.review.skillToggleFailure = false; });
  await toggle.click();
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="启用 Sample Skill"]')?.getAttribute("aria-checked") === "true");
  assert.deepEqual(await page.evaluate(() => window.review.skillToggleCalls.map(call => call.enabled)), [false, true, true]);
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    assert.equal(await toggle.isVisible(), true);
    await toggle.scrollIntoViewIfNeeded();
    const size = await page.evaluate(() => {
      const container = document.querySelector('[data-page-scroll-container]');
      return { scroll: container.scrollWidth, client: container.clientWidth };
    });
    assert.ok(size.scroll <= size.client + 1, `Skills overflow at ${width}px`);
    if (captureDir) await page.screenshot({ path: path.join(captureDir, `skills-${width}.png`) });
  }
  console.log("PASS Skills toggle retains disabled entries, waits for disk state, reports failures and blocks conflicts");
}
