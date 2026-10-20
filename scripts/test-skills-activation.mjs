import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// The Skills page: one library copy per skill, a switch per agent, bringing in
// what agents already have, adding a folder, uninstalling and the backups.
export async function testSkillsActivation({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = (operation) => page.evaluate((operation) => window.review.skillCalls.filter((call) => call.operation === operation), operation);
  const row = (name) => page.locator(`[data-skill="${name}"]`);
  const cell = (name, app) => row(name).locator(`[data-skill-cell="${app}"]`);

  await reset("skills");
  await page.emulateMedia({ reducedMotion: "reduce" });

  // ---- the matrix reads each agent's folder
  await row("sample").waitFor();
  assert.equal(await cell("sample", "codex").getAttribute("aria-checked"), "true");
  assert.equal(await cell("sample", "cursor").getAttribute("data-state"), "off");
  assert.equal(await cell("sample", "pi").isDisabled(), true, "an agent that is not installed is locked");
  assert.equal(await cell("review", "cursor").getAttribute("data-state"), "occupied");
  console.log("PASS the Skills matrix shows each agent's folder: linked, free, taken or missing");

  // ---- a switch links one agent and the page waits for it
  await page.evaluate(() => { window.review.skillHold = true; });
  await cell("sample", "cursor").click();
  await page.waitForFunction(() => !!window.review.skillResolve);
  await eventually(async () => assert.equal(await cell("sample", "codex").isDisabled(), true, "every switch waits for the write"));
  await page.evaluate(() => { window.review.skillHold = false; window.review.skillResolve(); });
  await eventually(async () => assert.equal(await cell("sample", "cursor").getAttribute("aria-checked"), "true"));
  assert.deepEqual((await calls("set")).at(-1), { operation: "set", names: ["sample"], app: "cursor", on: true });

  // A folder holding another skill by the name is reported, not replaced.
  await cell("review", "cursor").click();
  await page.waitForFunction((title) => window.review.lastToast === title, await tr("skills.failures", { count: 1 }));
  assert.equal(await cell("review", "cursor").getAttribute("data-state"), "occupied");

  // A column turns every listed skill off for one agent.
  await page.locator('[data-skills-column="codex"]').click();
  await page.getByRole("menuitem", { name: await tr("skills.column.disableAll") }).click();
  await eventually(async () => assert.equal(await cell("review", "codex").getAttribute("aria-checked"), "false"));
  assert.equal(await cell("sample", "codex").getAttribute("aria-checked"), "false");
  console.log("PASS switches link one agent at a time, and a taken name is reported instead of replaced");

  // ---- skills the agents already have are brought in
  const banner = page.locator("[data-skills-unmanaged-banner]");
  await banner.getByText(await tr("skills.unmanaged.banner", { count: 2 }), { exact: true }).waitFor();
  await banner.getByRole("button", { name: await tr("skills.unmanaged.review") }).click();
  const unmanaged = page.locator("[data-skills-unmanaged]");
  const linked = unmanaged.locator('[data-unmanaged-skill="linked"]');
  await linked.getByText(await tr("skills.unmanaged.external"), { exact: true }).waitFor();
  assert.equal(await linked.getByRole("button").count(), 0, "another tool's link is not brought in");
  await unmanaged.locator('[data-unmanaged-skill="old"]').getByText(await tr("skills.unmanaged.legacy"), { exact: true }).waitFor();
  await unmanaged.locator('[data-unmanaged-skill="notes"]').getByRole("button", { name: await tr("skills.unmanaged.import") }).click();
  await unmanaged.locator('[data-unmanaged-skill="notes"]').waitFor({ state: "detached" });
  assert.deepEqual((await calls("import")).at(-1).items, [{ name: "notes", app: "claudeCode", legacyDisabled: false }]);
  await unmanaged.getByRole("button", { name: await tr("skills.unmanaged.importAll") }).click();
  await unmanaged.locator('[data-unmanaged-skill="old"]').waitFor({ state: "detached" });
  await page.getByRole("radio", { name: new RegExp(`^${await tr("skills.tabs.library")}`) }).click();
  await row("notes").waitFor();
  assert.equal(await cell("notes", "claudeCode").getAttribute("aria-checked"), "true", "a skill brought in stays on where it was");
  assert.equal(await cell("old", "codex").getAttribute("aria-checked"), "false", "a skill turned off before stays off");
  console.log("PASS skills on this machine are brought in, staying on where they were, and other tools' links are left alone");

  // ---- adding a folder puts it in the chosen agents
  await page.evaluate(() => { window.review.pickedFolder = "C:/downloads/pdf-tools"; });
  await page.getByRole("button", { name: await tr("skills.add") }).first().click();
  const dialog = page.locator("[data-skills-add]");
  assert.equal(await dialog.getByRole("checkbox", { name: "Codex" }).isChecked(), true);
  assert.equal(await dialog.getByRole("checkbox", { name: "Pi" }).isDisabled(), true);
  await dialog.getByRole("checkbox", { name: "Claude Code" }).click();
  await dialog.getByRole("button", { name: await tr("skills.addDialog.confirm"), exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  await row("pdf-tools").waitFor();
  assert.deepEqual((await calls("add")).at(-1), { operation: "add", path: "C:/downloads/pdf-tools", apps: ["codex"] });
  assert.equal(await cell("pdf-tools", "codex").getAttribute("aria-checked"), "true");
  assert.equal(await page.evaluate(() => localStorage.getItem("agentharbor.skill-targets")), '["codex"]');
  console.log("PASS an added folder is linked into the chosen agents and the choice is remembered");

  // ---- uninstalling asks first, keeps a backup, and the backup restores it
  await row("review").getByRole("button", { name: await tr("skills.more") }).click();
  await page.getByRole("menuitem", { name: await tr("skills.row.uninstall") }).click();
  const alert = page.getByRole("alertdialog");
  await alert.getByText(await tr("skills.confirmUninstall", { name: "review" }), { exact: true }).waitFor();
  await alert.getByRole("button", { name: await tr("skills.uninstall"), exact: true }).click();
  await row("review").waitFor({ state: "detached" });
  await page.getByRole("radio", { name: new RegExp(`^${await tr("skills.tabs.backups")}`) }).click();
  const backups = page.locator("[data-skills-backups]");
  await backups.locator('[data-skill-backup="backup-review"]').getByRole("button", { name: await tr("skills.backups.restore") }).click();
  await page.waitForFunction((title) => window.review.lastToast === title, await tr("skills.restored"));
  await backups.locator('[data-skill-backup="backup-gone"]').getByRole("button", { name: await tr("skills.backups.delete") }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("skills.backups.delete"), exact: true }).click();
  await backups.locator('[data-skill-backup="backup-gone"]').waitFor({ state: "detached" });
  await page.getByRole("radio", { name: new RegExp(`^${await tr("skills.tabs.library")}`) }).click();
  await row("review").waitFor();
  assert.equal(await cell("review", "codex").getAttribute("aria-checked"), "false", "a restored skill comes back off");
  console.log("PASS uninstalling asks first and keeps a backup that restores the skill");

  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const size = await page.evaluate(() => {
      const container = document.querySelector("[data-page-scroll-container]");
      return { scroll: container.scrollWidth, client: container.clientWidth };
    });
    assert.ok(size.scroll <= size.client + 1, `Skills overflow at ${width}px`);
    if (captureDir) await page.screenshot({ path: path.join(captureDir, `skills-${width}.png`) });
  }
  await page.setViewportSize({ width: 1200, height: 820 });
  console.log("PASS the Skills page fits desktop and phone widths");
}
