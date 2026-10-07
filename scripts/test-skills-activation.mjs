import assert from "node:assert/strict";
import path from "node:path";

export async function testSkillsActivation({ page, reset, captureDir }) {
  await reset("skills");
  await page.emulateMedia({ reducedMotion: "reduce" });
  const toggle = page.getByRole("switch", { name: "启用 Sample Skill", exact: true });
  const conflict = page.getByRole("switch", { name: "启用 Conflict Skill", exact: true });
  await toggle.waitFor();
  assert.equal(await toggle.getAttribute("aria-checked"), "true");
  assert.equal(await conflict.count(), 0, "conflicts do not show a misleading active switch");
  assert.equal(await page.getByText("已跳过非 UTF-8 的 SKILL.md", { exact: true }).count(), 1);
  assert.equal(await page.getByRole("button", { name: "打开禁用副本", exact: true }).count(), 1);
  await page.evaluate(() => { window.review.skillToggleHold = true; });
  await toggle.click();
  await page.waitForFunction(() => Boolean(window.review.skillToggleResolve));
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="启用 Sample Skill"]')?.disabled === true);
  assert.equal(await toggle.isDisabled(), true);
  assert.equal(await page.getByRole("radio", { name: "Claude Code", exact: true }).isDisabled(), true, "cannot change source while a disk write is pending");
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
  await page.getByRole("button", { name: "保留禁用副本", exact: true }).click();
  assert.equal(await page.evaluate(() => window.review.skillConflictChoice), null, "selecting a copy requires confirmation");
  await page.getByRole("alertdialog").getByRole("button", { name: "确认", exact: true }).click();
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="启用 Conflict Skill"]')?.disabled === false);
  assert.equal(await conflict.getAttribute("aria-checked"), "false");
  assert.deepEqual(await page.evaluate(() => window.review.skillConflictChoice), { id: "conflict", keepEnabled: false, source: "codex" });
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
  await page.getByRole("radio", { name: "Codex", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  const claudeToggle = page.getByRole("switch", { name: "启用 Claude Sample Skill", exact: true });
  await claudeToggle.waitFor();
  assert.equal(await page.getByRole("radio", { name: "Claude Code", exact: true }).evaluate(node => node === document.activeElement), true, "keyboard focus survives source workspace remount");
  assert.equal(await page.getByRole("heading", { name: "Sample Skill", exact: true }).count(), 0, "same skill ID from Codex must not appear in Claude Code");
  assert.equal(await page.getByText("已跳过非 UTF-8 的 SKILL.md", { exact: true }).count(), 0, "source warnings remain isolated");
  const linked = page.locator(".rounded-2xl.border.bg-card").filter({ has: page.getByRole("heading", { name: "Linked Skill", exact: true }) });
  assert.equal(await linked.getByText("外部管理", { exact: true }).count(), 1);
  assert.equal(await linked.getByRole("switch").count(), 0);
  assert.equal(await linked.getByRole("button", { name: "删除", exact: true }).isDisabled(), true);
  await linked.getByRole("button", { name: "打开文件夹", exact: true }).click();
  assert.match(await page.evaluate(() => window.review.openedPath), /\.claude\/skills\/linked$/);
  await page.evaluate(() => { window.review.skillToggleHold = true; window.review.skillToggleResolve = null; });
  await claudeToggle.click();
  await page.waitForFunction(() => Boolean(window.review.skillToggleResolve));
  assert.equal(await page.getByRole("radio", { name: "Codex", exact: true }).isDisabled(), true);
  await page.evaluate(() => { window.review.skillToggleHold = false; window.review.skillToggleResolve(); });
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="启用 Claude Sample Skill"]')?.getAttribute("aria-checked") === "false");
  assert.deepEqual(await page.evaluate(() => window.review.skillToggleCalls.at(-1)), { id: "sample", enabled: false, source: "claudeCode" });
  await page.getByRole("radio", { name: /^备份 \(1\)$/ }).click();
  await page.getByRole("heading", { name: "Claude Backup", exact: true }).waitFor();
  await page.getByRole("button", { name: "恢复", exact: true }).click();
  await page.waitForFunction(() => window.review.skillRestoreCalls?.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.skillRestoreCalls[0]), { id: "claude-backup", source: "claudeCode" });
  await page.getByRole("radio", { name: "Codex", exact: true }).click();
  await toggle.waitFor();
  assert.equal(await toggle.getAttribute("aria-checked"), "true", "Claude disable must not change Codex skill state");
  assert.equal(await page.getByRole("heading", { name: "Claude Backup", exact: true }).count(), 0, "switching source resets backup tab");
  assert.deepEqual(await page.evaluate(() => window.review.skillToggleCalls.map(call => call.source)), ["codex", "codex", "codex", "claudeCode"]);
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.getByRole("radio", { name: "Claude Code", exact: true }).click();
    await claudeToggle.waitFor();
    const size = await page.evaluate(() => {
      const container = document.querySelector('[data-page-scroll-container]');
      return { scroll: container.scrollWidth, client: container.clientWidth };
    });
    assert.ok(size.scroll <= size.client + 1, `Claude Skills overflow at ${width}px`);
    if (captureDir) await page.screenshot({ path: path.join(captureDir, `claude-skills-${width}.png`) });
    await page.getByRole("radio", { name: "Codex", exact: true }).click();
    await toggle.waitFor();
  }
  console.log("PASS Skills source-scoped activation/backup, pending-write guard, readonly projections and narrow layouts");
  await page.emulateMedia({ reducedMotion: "no-preference" });
}
