import assert from "node:assert/strict";
import path from "node:path";

// The injection console and the aggregation routing board: one client panel
// at a time, merge suggestions from relays serving the same model, tiers and
// each member's live routing state.
export async function testAggregationBoard({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const manager = () => page.locator('section[aria-labelledby="aggregation-title"]');
  const seed = async ({ aggregates = [], live = [] } = {}) => {
    await reset("compact");
    await page.evaluate(({ aggregates, live }) => {
      const review = window.review;
      const base = review.getState().providers[0];
      const model = id => ({ ...base.models[0], id, displayName: id, included: true, protocol: "responses", endpointTypes: ["responses"], catalogRef: null });
      const relay = (id, name, models, extra = {}) => ({ ...base, id, name, baseUrl: `https://${id}.invalid/v1`, enabled: true, hasApiKey: true,
        protocol: "responses", model: models[0], models: models.map(model), ...extra });
      review.providerFixtures = [
        relay("one", "Relay One", ["claude-opus-5.5", "deepseek-v4.1-flash", "deepseek-v4.1-thinking", "kimi-k2-0905"]),
        relay("two", "Relay Two", ["claude-opus-5-5-20260801", "deepseek-v4-1-flash-260910", "qwen3-235b-a22b-2507"]),
        relay("three", "Relay Three", ["anthropic/claude-opus-5.5", "deepseek-v4.1", "qwen3-235b-a22b"]),
        relay("off", "Disabled relay", ["kimi-k2-0905"], { enabled: false }),
      ];
      review.aggregateModels = aggregates;
      review.aggregateLive = live;
      review.aggregateSaves = [];
      review.mount("access");
    }, { aggregates, live });
    await manager().waitFor();
  };

  // Merge suggestions group vendor spellings of one model, never variants.
  await seed();
  const suggestions = manager().locator("[data-merge-suggestion]");
  await suggestions.first().waitFor();
  assert.deepEqual(await suggestions.evaluateAll(nodes => nodes.map(node => node.getAttribute("data-merge-suggestion"))),
    ["claude-opus-5-5", "deepseek-v4-1-flash"],
    "dates, prefixes, dots and underscores merge; -thinking, -2507 and a disabled relay's model do not");
  assert.match(await suggestions.first().innerText(), /claude-opus-5\.5/, "the plainest spelling names the pool");
  await suggestions.first().click();
  const editor = manager().locator("form[data-aggregate-editor]");
  await editor.waitFor();
  assert.equal(await editor.locator("#aggregate-public-id").inputValue(), "claude-opus-5.5");
  assert.deepEqual(await editor.locator('[id^="member-model-"]').allInnerTexts(),
    ["claude-opus-5.5", "claude-opus-5-5-20260801", "anthropic/claude-opus-5.5"]);
  assert.equal(await manager().locator("[data-merge-suggestions]").count(), 0, "suggestions step aside while a draft is open");
  await editor.getByRole("button", { name: await tr("common.save"), exact: true }).click();
  await page.waitForFunction(() => window.review.aggregateSaves.length === 1);
  const saved = await page.evaluate(() => window.review.aggregateSaves[0]);
  assert.deepEqual(saved.members.map(({ providerId, actualModelId, priority }) => [providerId, actualModelId, priority]), [
    ["one", "claude-opus-5.5", 0], ["two", "claude-opus-5-5-20260801", 0], ["three", "anthropic/claude-opus-5.5", 0]]);
  assert.equal(saved.firstTokenTimeoutSecs, 0);
  await page.waitForFunction(() => document.querySelectorAll("[data-merge-suggestion]").length === 1);
  console.log("PASS merge suggestions find one model across vendor spellings and draft a pool from it");

  // Tiers and live state: who serves, who cools down, who is off or gone.
  const pool = { id: "pool", publicModelId: "pool/model", displayName: "Pool", enabled: true,
    strategy: "weightedRoundRobin", affinity: "session", retryPolicy: { maxAttempts: 2, failureThreshold: 3 },
    compression: { mode: "inherit" }, firstTokenTimeoutSecs: 45, members: [
      { id: "serving", providerId: "one", actualModelId: "claude-opus-5.5", enabled: true, priority: 0, weight: 3 },
      { id: "cooling", providerId: "two", actualModelId: "claude-opus-5-5-20260801", enabled: true, priority: 0, weight: 1 },
      { id: "gone", providerId: "three", actualModelId: "no-longer-listed", enabled: true, priority: 5, weight: 1 },
      { id: "off", providerId: "three", actualModelId: "anthropic/claude-opus-5.5", enabled: false, priority: 9, weight: 1 },
    ] };
  await seed({ aggregates: [pool], live: [{ aggregateId: "pool", preferredMemberId: "serving", preferredIdleSecs: 1, members: [
    { memberId: "serving", inFlight: 2, failures: 0, coolingMs: null, tripped: false },
    { memberId: "cooling", inFlight: 0, failures: 3, coolingMs: 41_200, tripped: true },
  ] }] });
  const row = manager().locator('[data-aggregate-row="pool"]');
  await row.locator('[data-member-state="hit"]').waitFor();
  assert.deepEqual(await row.locator("[data-route-member]").evaluateAll(nodes =>
    nodes.map(node => [node.getAttribute("data-route-member"), node.getAttribute("data-member-state"), node.closest("[data-tier]").getAttribute("data-tier")])),
  [["serving", "hit", "0"], ["cooling", "cooling", "0"], ["gone", "down", "1"], ["off", "off", "2"]],
  "members sit in their tiers with what each is doing now");
  assert.match(await row.locator('[data-member-state="cooling"]').innerText(), /42s/);
  assert.match(await row.locator('[data-member-state="hit"]').innerText(), /×3/);
  assert.match(await row.innerText(), new RegExp(await tr("aggregation.firstTokenShort", { seconds: 45 })));
  await manager().locator("[data-aggregation-live]").waitFor();
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "aggregation-board.png"), fullPage: true });

  // Editing normalizes priorities to tier positions and moves members by tier.
  await row.getByRole("button", { name: await tr("common.edit"), exact: true }).click();
  const lanes = editor.locator("[data-tier-lane]");
  assert.equal(await lanes.count(), 3);
  const members = editor.locator("[data-aggregate-member]");
  await members.nth(2).getByRole("button", { name: await tr("aggregation.moveUp", { index: 3 }), exact: true }).click();
  assert.equal(await lanes.count(), 2, "moving the only member of a tier up removes the empty tier");
  await members.nth(3).locator('[id^="member-weight-"]').fill("4");
  await members.nth(3).getByRole("switch").click();
  await editor.getByRole("button", { name: await tr("common.save"), exact: true }).click();
  await page.waitForFunction(() => window.review.aggregateSaves.length === 1);
  const edited = await page.evaluate(() => window.review.aggregateSaves[0]);
  assert.deepEqual(edited.members.map(({ id, priority, weight, enabled }) => [id, priority, weight, enabled]),
    [["serving", 0, 3, true], ["cooling", 0, 1, true], ["gone", 0, 1, true], ["off", 1, 4, true]],
    "saved priorities are tier positions 0, 1… and other fields are kept");
  assert.equal(edited.firstTokenTimeoutSecs, 45);
  console.log("PASS routing board shows tiers with live serving, cooling, off and unavailable members, and edits save tier positions");

  // The console shows one client at a time, remembers it and moves by keyboard.
  const tab = client => page.locator(`[data-client-tab="${client}"]`);
  const panel = client => page.locator(`#client-panel-${client}`);
  assert.equal(await tab("codex").getAttribute("aria-selected"), "true");
  assert.equal(await panel("claudeDesktop").isHidden(), true);
  await tab("codex").focus();
  await page.keyboard.press("ArrowDown");
  assert.equal(await tab("claudeCode").getAttribute("aria-selected"), "true");
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-client-tab")), "claudeCode");
  await page.keyboard.press("End");
  assert.equal(await tab("pi").getAttribute("aria-selected"), "true");
  assert.equal(await panel("pi").isVisible(), true);
  assert.equal(await panel("codex").isHidden(), true);
  assert.equal(await page.evaluate(() => localStorage.getItem("agentharbor.inject-client.v1")), "pi");
  await page.locator('[data-route-shortcut="claudeDesktop"]').click();
  await page.waitForFunction(() => document.querySelector('[data-client-tab="claudeDesktop"]')?.getAttribute("aria-selected") === "true");
  await page.waitForFunction(() => document.getElementById("claude-desktop-title")?.closest("[role=tabpanel]")?.hidden === false);
  for (const width of [1920, 1024, 390]) {
    await page.setViewportSize({ width, height: 900 });
    assert.equal(await page.locator("[data-client-console]").evaluate(node => node.scrollWidth <= node.clientWidth + 1), true,
      `the console fits ${width}px`);
    if (captureDir) {
      await page.locator("[data-client-console]").scrollIntoViewIfNeeded();
      await page.screenshot({ path: path.join(captureDir, `inject-console-${width}.png`) });
    }
  }
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS injection console shows one client at a time, follows topology shortcuts, keyboard and remembers the choice");

  // Route diagnostics: summary figures, a failures-only view and clearing that keeps running requests.
  await reset("compact");
  await page.evaluate(() => {
    const attempt = (id, status) => ({ attemptId: id, providerId: "relay", providerName: "Relay " + id, upstreamModel: "wire",
      matchedModel: "wire", protocol: "responses", sent: {}, status, failureStage: status >= 400 ? "headers" : null,
      error: status >= 400 ? "rate_limited" : null, durationMs: 100, catalogRevision: "rev", matchKind: "exact",
      pricingStatus: "unpriced", pricingSource: "", selectionReason: "aggregate_rotation", retryReason: null });
    const base = { timestamp: 1_790_740_800, requested: {}, stage: "complete", done: true, failureStage: null, error: null, firstTokenMs: 300 };
    window.review.routeDiagnostics = [
      { ...base, id: "retried", requestedModel: "pool/model", clientSource: "codex", status: 200, durationMs: 1500,
        attempts: [attempt("one", 429), { ...attempt("two", 200), retryReason: "aggregate_first_token_timeout" }] },
      { ...base, id: "failed", requestedModel: "broken/model", clientSource: "claudeCode", status: 502, error: "transport_error",
        failureStage: "connect", durationMs: 500, firstTokenMs: null, attempts: [attempt("three", 502)] },
      { ...base, id: "running", requestedModel: "live/model", clientSource: "pi", status: null, done: false, stage: "stream",
        durationMs: 200, firstTokenMs: 100, attempts: [attempt("four", 200)] },
    ];
    window.review.mount("access");
  });
  const routes = page.getByRole("region", { name: await tr("newapi.routeDiagnostics"), exact: true });
  await routes.getByRole("button", { name: await tr("newapi.routeDiagnostics"), exact: true }).click();
  const summaries = routes.locator("summary");
  await summaries.nth(2).waitFor();
  const stat = key => routes.locator(`[data-stat="${key}"] dd`).innerText();
  assert.deepEqual([await stat("requests"), await stat("failed"), await stat("average"), await stat("firstToken")],
    ["3", "1", "1.0 s", "200 ms"], "stats count every request, average finished ones and take the median first token");
  assert.match(await summaries.nth(0).innerText(), new RegExp(await tr("newapi.traceAttemptCount", { count: 2 })));
  assert.deepEqual(await routes.locator("details").evaluateAll(nodes => nodes.map(node => node.getAttribute("data-route-state"))),
    ["complete", "failed", "running"]);
  await summaries.nth(0).click();
  assert.match(await routes.innerText(), new RegExp(await tr("newapi.traceRetryReason.aggregate_first_token_timeout")));
  const failuresOnly = routes.getByRole("button", { name: await tr("newapi.traceFailedOnly"), exact: true });
  await failuresOnly.click();
  assert.equal(await failuresOnly.getAttribute("aria-pressed"), "true");
  await page.waitForFunction(() => document.querySelectorAll('[data-route-diagnostics] summary').length === 1);
  assert.match(await summaries.first().innerText(), /broken\/model/);
  await failuresOnly.click();
  await summaries.nth(2).waitFor();
  const clear = routes.locator("[data-trace-clear]");
  await clear.click();
  await page.waitForFunction(() => document.querySelectorAll('[data-route-diagnostics] summary').length === 1);
  assert.equal(await page.evaluate(() => window.review.diagnosticsClears), 1);
  assert.match(await summaries.first().innerText(), /live\/model/, "a request still running survives clearing");
  assert.equal(await clear.isDisabled(), true, "nothing finished is left to clear");
  if (captureDir) await routes.screenshot({ path: path.join(captureDir, "route-diagnostics.png") });
  console.log("PASS route diagnostics summarize requests, filter failures and clear finished records while keeping running ones");
}
