import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testPiIntegration({ page, reset, captureDir }) {
  const tr = key => page.evaluate(key => window.review.t(key), key);
  const dialog = () => page.getByRole("dialog");
  const card = () => page.locator("#client-config-pi");
  const seed = async ({ allClients = false, folded = false, aggregate = false } = {}) => {
    await reset("compact");
    await page.evaluate(({ allClients, folded, aggregate }) => {
      const review = window.review;
      const base = review.getState().providers[0];
      review.providerFixtures = [base, { ...base, id: "second-pi", name: "Second relay" }];
      const client = (targetId, name) => ({ targetId, name, path: `C:/synthetic/${targetId}/models.json`,
        protocols: targetId === "claudeCode" ? ["anthropicMessages"] : ["responses", "chatCompletions", "anthropicMessages"],
        configured: false, conflict: false, conflictFields: [], error: null, providerId: null, model: null, request: null });
      review.clientStatusFixtures = allClients
        ? [client("claudeCode", "Claude Code"), client("opencode", "OpenCode"), client("pi", "Pi")]
        : [client("pi", "Pi")];
      review.aggregateModels = aggregate ? [{ id: "pi-pool", publicModelId: "public-pi", displayName: "Pi pool", enabled: true,
        strategy: "roundRobin", sessionAffinity: "preferSticky", retryPolicy: { maxAttempts: 2, failureThreshold: 3 },
        compression: { mode: "inherit" }, members: [{ id: "member", providerId: base.id, actualModelId: base.model, enabled: true, priority: 0, weight: 1 }],
      }] : [];
      localStorage.setItem("agentharbor.folded-sections.v1", JSON.stringify(Object.fromEntries(
        ["codex-access", "client-configs", "claude-desktop", "aggregation"].map(id => [id, folded ? "closed" : "open"]))));
      review.mount("access");
    }, { allClients, folded, aggregate });
    await page.getByRole("navigation", { name: await tr("newapi.configShortcuts") }).waitFor();
    if (!allClients) await page.locator('[data-client-tab="pi"]').click();
  };
  const openPi = async () => {
    await card().locator("button[data-config-action]").click();
    await dialog().waitFor();
  };
  const preview = async () => {
    await dialog().getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
    await dialog().locator("pre").waitFor();
    return page.evaluate(() => window.review.previewed);
  };

  await seed();
  await openPi();
  assert.equal(await dialog().getByRole("button", { name: await tr("clients.accessNative"), exact: true }).getAttribute("aria-pressed"), "true");
  await dialog().getByRole("button", { name: /Second relay/ }).click();
  const subsets = dialog().locator("details").filter({ has: page.locator("summary") });
  const first = subsets.first();
  await first.locator("summary").click();
  await first.getByRole("checkbox", { name: /anthropic-model/ }).uncheck();
  const request = await preview();
  assert.equal(request.targetId, "pi");
  assert.deepEqual(request.providerIds, ["mixed", "second-pi"]);
  assert.ok(request.providerModelIds.mixed.includes("responses-model"));
  assert.ok(request.providerModelIds.mixed.includes("chat-model"));
  assert.ok(!request.providerModelIds.mixed.includes("anthropic-model"));
  assert.ok(request.providerModelIds["second-pi"].includes("anthropic-model"));
  await dialog().getByRole("button", { name: await tr("clients.apply"), exact: true }).click();
  await dialog().waitFor({ state: "detached" });
  await card().getByRole("button", { name: await tr("clients.reapply"), exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("clients.reapply"), exact: true }).click();
  await page.waitForFunction(() => !!window.review.reappliedClient);
  await card().getByRole("button", { name: await tr("clients.restore"), exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("clients.restore"), exact: true }).click();
  await page.waitForFunction(() => !!window.review.restoredClient);
  await eventually(async () => assert.equal(await card().getByRole("button", { name: await tr("clients.configure"), exact: true }).count(), 1));
  console.log("PASS Pi reuses multi-relay protocol/model selections and managed apply, reapply and restore");

  await seed({ aggregate: true });
  await openPi();
  await dialog().getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  await dialog().getByRole("checkbox", { name: /Pi pool/ }).check();
  await dialog().locator("#client-aggregate-protocol").click();
  await page.getByRole("option", { name: "Messages", exact: true }).click();
  const aggregateRequest = await preview();
  assert.equal(aggregateRequest.targetId, "pi");
  assert.equal(aggregateRequest.accessMode, "router");
  assert.equal(aggregateRequest.aggregateIngressProtocol, "anthropicMessages");
  assert.ok(aggregateRequest.modelSelection.availableTargets.some(target => target.kind === "aggregate" && target.aggregateId === "pi-pool"));
  await dialog().getByRole("button", { name: await tr("common.cancel"), exact: true }).click();
  console.log("PASS Pi consumes aggregate public targets alongside direct relay model selections");

  await seed({ allClients: true, folded: true });
  const nav = page.getByRole("navigation", { name: await tr("newapi.configShortcuts") });
  assert.equal(await nav.locator("button[data-route-shortcut]").count(), 7);
  for (const mark of await nav.locator("img").all()) {
    assert.equal(await mark.evaluate(image => image.complete && image.naturalWidth > 0), true, "brand marks are actual loaded images");
  }
  for (const [label, target] of [["newapi.routeSourcePi", "client-config-pi"], ["newapi.routeSourceClaudeCode", "client-config-claudeCode"],
    ["newapi.routeSourceOpenCode", "client-config-opencode"], ["newapi.routeSourceCodex", "newapi-routing-title"],
    ["newapi.routeSourceCursor", "cursor-access-title"],
    ["newapi.routeSourceClaudeDesktop", "claude-desktop-title"], ["aggregation.title", "aggregation-title"]]) {
    await nav.getByRole("button", { name: await tr(label), exact: true }).click();
    await page.waitForFunction(target => {
      const section = document.getElementById(target);
      return !!section && section.contains(document.activeElement);
    }, target);
    assert.equal(await nav.getByRole("button", { name: await tr(label), exact: true }).getAttribute("aria-current"), "location");
  }
  assert.equal(await page.locator('[data-client-tab="aggregation"]').count(), 0);
  assert.equal(await page.locator('[data-client-tab="claudeDesktop"]').getAttribute("aria-selected"), "true",
    "each shortcut shows its client's panel in the console");
  await nav.getByRole("button", { name: "Pi", exact: true }).focus();
  await page.keyboard.press("Enter");
  await card().locator("button[data-config-action]").waitFor();
  await page.waitForFunction(() => document.activeElement?.closest("#client-config-pi") != null);
  console.log("PASS icon shortcuts expand saved folds, switch the console, cancel previous jumps and focus the corresponding configuration");

  await page.setViewportSize({ width: 390, height: 844 });
  await nav.scrollIntoViewIfNeeded();
  assert.equal(await nav.evaluate(node => node.scrollWidth <= node.clientWidth), true);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "pi-shortcuts-mobile-light.png"), fullPage: true, animations: "disabled" });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "pi-shortcuts-mobile-dark.png"), fullPage: true, animations: "disabled" });
  await page.setViewportSize({ width: 1200, height: 700 });
  await nav.getByRole("button", { name: "Pi", exact: true }).click();
  await page.waitForFunction(() => document.activeElement?.closest("#client-config-pi") != null);
  await openPi();
  assert.equal(await dialog().evaluate(node => node.scrollWidth <= node.clientWidth), true);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "pi-config-desktop-dark.png"), fullPage: true, animations: "disabled" });
  await dialog().getByRole("button", { name: await tr("common.cancel"), exact: true }).click();
  await page.evaluate(() => {
    document.documentElement.classList.remove("dark");
    localStorage.removeItem("agentharbor.folded-sections.v1");
  });
  console.log("PASS Pi configuration and shortcuts fit narrow screens, follow the theme and support keyboard navigation");
}
