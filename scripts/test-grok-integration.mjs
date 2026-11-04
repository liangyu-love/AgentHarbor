import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testGrokIntegration({ page, reset, captureDir }) {
  const tr = key => page.evaluate(key => window.review.t(key), key);
  const dialog = () => page.getByRole("dialog");
  const card = () => page.locator("#client-config-grok");
  await reset("compact");
  await page.evaluate(() => {
    const review = window.review;
    review.providerFixtures = [review.getState().providers[0]];
    review.clientStatusFixtures = [{ targetId: "grok", name: "Grok Build", path: "C:/synthetic/.grok/config.toml",
      protocols: ["responses", "chatCompletions", "anthropicMessages"], configured: false, conflict: false, conflictFields: [],
      error: null, providerId: null, model: null, request: null }];
    review.aggregateModels = [];
    localStorage.setItem("agentharbor.folded-sections.v1", JSON.stringify(Object.fromEntries(
      ["codex-access", "client-configs", "claude-desktop", "aggregation"].map(id => [id, "open"]))));
    review.mount("access");
  });
  await page.getByRole("navigation", { name: await tr("newapi.configShortcuts") }).waitFor();
  await page.locator('[data-client-tab="grok"]').click();
  assert.equal(await page.locator('[data-client-tab="grok"] svg path').count(), 2, "the rail shows Grok's mark");
  await card().getByText(await tr("clients.grokHint")).waitFor();

  await card().locator("button[data-config-action]").click();
  await dialog().waitFor();
  // Grok's default model is written, so the model is named as a startup model.
  await dialog().getByText(await tr("clients.startupModel"), { exact: true }).waitFor();
  await dialog().getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  await dialog().getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
  await dialog().locator("pre").waitFor();
  const request = await page.evaluate(() => window.review.previewed);
  if (captureDir) await dialog().screenshot({ path: path.join(captureDir, "grok-dialog.png") });
  assert.equal(request.targetId, "grok");
  assert.equal(request.accessMode, "router");
  await dialog().getByRole("button", { name: await tr("clients.apply"), exact: true }).click();
  await dialog().waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.review.appliedClient.request.targetId), "grok");
  await page.waitForFunction(value => window.review.lastToastDescription === value, await tr("clients.grokReload"));
  console.log("PASS Grok Build applies its models with a startup model, then says how Grok picks them up");

  // The routing map draws Grok with its mark, routed to the relays.
  await eventually(async () => assert.equal(await card().locator("button[data-config-action]").innerText(), await tr("clients.change")));
  assert.equal(await page.locator('[data-route-shortcut="grok"] svg path').count(), 2, "the topology shows Grok's mark");
  await eventually(async () => assert.equal(await page.locator('[data-route="grok-hub"]').getAttribute("data-route-state"), "routed"));
  console.log("PASS the routing map shows Grok's mark and its routed line into the router");

  // The dashboard: orbit ring text and the data map.
  await page.evaluate(() => window.review.mount("overview"));
  const orbit = page.locator("[data-dash-orbit]");
  await orbit.waitFor();
  await eventually(async () => assert.equal(await page.locator('[data-orbit-link="grok"]').getAttribute("data-state"), "routed"));
  assert.match(await orbit.locator("textPath").textContent(), /GROK/);
  await page.locator(".overview-system-toggle").click();
  const files = page.getByRole("list", { name: await page.evaluate(() => window.review.t("workspace.keyFiles", { name: "Grok Build" })) });
  await files.waitFor();
  assert.deepEqual(await files.locator("li").evaluateAll(items => items.map(item => item.getAttribute("data-file"))),
    ["config", "auth", "sessions"]);
  console.log("PASS the dashboard orbit and the data map include Grok Build");
}
