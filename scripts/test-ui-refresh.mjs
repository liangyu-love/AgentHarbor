import assert from "node:assert/strict";
import path from "node:path";

// Routing topology, relay cards and thread repair reflect what each client
// actually uses rather than a single "active" provider.
export async function testUiRefresh({ page, reset, captureDir }) {
  const aggregate = { id: "pool-main", publicModelId: "pool/main", displayName: "Main pool", enabled: true,
    members: [{ id: "member-a", providerId: "group-a", actualModelId: "alpha-model", enabled: true, priority: 0, weight: 1 }],
    strategy: "roundRobin", affinity: "session", retryPolicy: { maxAttempts: 2, failureThreshold: 3 }, compression: { mode: "inherit" } };
  const mountRouted = async (routerPatch, statePatch = {}) => {
    await reset("compact");
    await page.evaluate(({ routerPatch, statePatch, aggregate }) => {
      Object.assign(window.review, { routerPatch: { enabled: true, accessMode: "router", ...routerPatch },
        statePatch: { proxyRunning: true, ...statePatch }, aggregateModels: [aggregate] });
      window.review.mount("groups");
    }, { routerPatch, statePatch, aggregate });
    await page.locator('[data-route="hub-relays"]').waitFor();
  };
  const routeStates = () => page.evaluate(() => Object.fromEntries(["official", "relays", "aggregation"].map(destination =>
    [destination, document.querySelector(`[data-route="hub-${destination}"]`)?.getAttribute("data-route-state")])));

  await mountRouted({ aggregateModelId: null });
  await page.waitForFunction(() => document.querySelector('[data-route="hub-relays"]')?.getAttribute("data-route-state") === "routed");
  assert.deepEqual(await routeStates(), { official: "routed", relays: "routed", aggregation: "idle" },
    "Relay injection reaches the official account and the relays, never the unused aggregate");
  const codexNode = page.locator('[data-route-shortcut="codex"]');
  assert.match(await codexNode.innerText(), /官方 \+ 中转/);
  const faces = page.locator("[data-relay-card-face]");
  assert.equal(await faces.count(), 2);
  assert.deepEqual(await faces.evaluateAll(nodes => nodes.map(node => node.getAttribute("data-codex"))), ["injected", "injected"],
    "Every enabled relay is injected into Codex's menu");
  assert.equal(await page.locator("[data-relay-injected]").count(), 2);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "ui-refresh-relay-injection.png"), fullPage: true });

  await mountRouted({ aggregateModelId: aggregate.id });
  await page.waitForFunction(() => document.querySelector('[data-route="hub-aggregation"]')?.getAttribute("data-route-state") === "routed");
  assert.deepEqual(await routeStates(), { official: "idle", relays: "idle", aggregation: "routed" },
    "A selected aggregate is Codex's only destination");
  assert.equal(await page.locator("[data-relay-injected]").count(), 0, "Relays are not in Codex's menu while it uses an aggregate");

  await mountRouted({ accessMode: "native" }, { activeProviderId: "group-b" });
  await page.locator("[data-relay-direct]").waitFor();
  assert.equal(await page.locator("[data-relay-direct]").count(), 1);
  assert.equal(await faces.first().getAttribute("data-provider-id"), "group-b", "The deck opens on the member Codex uses");
  assert.equal(await page.locator("[data-relay-injected]").count(), 0);
  console.log("PASS topology and relay cards light only the destinations each client actually uses");

  await reset("thread-tree");
  await page.evaluate(() => {
    window.review.configuredProvider = "agentharbor_native";
    window.review.threadItems = window.review.threadItems.map((thread, index) =>
      index === 1 || index === 3 ? { ...thread, modelProvider: "cc_switch" } : thread);
  });
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  const notice = page.locator("[data-thread-misplaced-notice]");
  await notice.waitFor();
  assert.match(await notice.innerText(), /2 个会话存储在其他 provider 下/);
  await notice.getByRole("button", { name: "只看这些会话", exact: true }).click();
  const results = page.locator("[data-thread-results]");
  await page.waitForFunction(() => document.querySelectorAll("[data-thread-results] [role=checkbox]").length === 2);
  assert.equal(await results.locator("[data-thread-misplaced]").count(), 2);
  assert.equal(await results.getByRole("button", { name: "修复会话", exact: false }).count(), 2);
  await notice.getByRole("button", { name: "显示全部", exact: true }).click();
  await page.waitForFunction(() => document.querySelectorAll("[data-thread-results] [role=checkbox]").length > 2);
  const repairable = await results.locator("[data-thread-card]").evaluateAll(cards => cards.map(card => ({
    misplaced: !!card.querySelector("[data-thread-misplaced]"),
    repair: [...card.querySelectorAll("button")].some(button => /修复/.test(button.getAttribute("aria-label") ?? "")),
  })));
  assert.ok(repairable.length > 2 && repairable.every(card => card.misplaced === card.repair), JSON.stringify(repairable));
  console.log("PASS only threads stored under another provider are flagged and offer repair");
}
