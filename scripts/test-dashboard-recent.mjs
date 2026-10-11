import assert from "node:assert/strict";

// The dashboard's recent sessions merge Codex and Claude Code, filter by
// client, and open the threads page on the session's own source.
export async function testDashboardRecent({ page, reset }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  await reset("overview");
  const panel = page.locator('section[aria-labelledby="overview-recent-title"]');
  const filter = panel.getByRole("radiogroup", { name: await tr("workspace.recentSource") });
  await filter.waitFor();
  await panel.locator("[data-recent-client]").first().waitFor();
  const count = async source => Number((await filter.locator(`[data-recent-source="${source}"] span.tabular-nums`).innerText()).trim());
  const [all, codex, claude] = [await count("all"), await count("codex"), await count("claudeCode")];
  assert.ok(codex > 0 && claude === 1, `both sources are read: ${codex} Codex, ${claude} Claude Code`);
  assert.equal(all, codex + claude);
  assert.equal(await filter.locator('[data-recent-source="all"]').getAttribute("aria-checked"), "true");

  await filter.locator('[data-recent-source="claudeCode"]').click();
  const rows = panel.locator("[data-recent-client]");
  await panel.locator('[data-recent-client="claudeCode"]').waitFor();
  assert.deepEqual(await rows.evaluateAll(items => items.map(item => item.getAttribute("data-recent-client"))), ["claudeCode"]);
  await rows.first().getByRole("button").click();
  assert.equal(await page.evaluate(() => window.review.navigatedTo), "threads");
  assert.equal(await page.evaluate(() => localStorage.getItem("threads_source")), "claudeCode", "the threads page opens on Claude Code");

  await filter.locator('[data-recent-source="codex"]').click();
  assert.ok((await rows.evaluateAll(items => items.map(item => item.getAttribute("data-recent-client")))).every(client => client === "codex"));
  await page.evaluate(() => localStorage.removeItem("threads_source"));
  console.log("PASS the dashboard's recent sessions merge Codex and Claude Code and open the matching source");
}
