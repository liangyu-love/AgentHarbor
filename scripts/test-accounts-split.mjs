import assert from "node:assert/strict";
import path from "node:path";

// Accounts and usage on their own pages, account-source tabs with sign-in,
// sharing with model inject, and account sources in the inject dialog.
export async function testAccountsSplit({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = () => page.evaluate(() => window.review.sourceCalls);

  await reset("accounts");
  await page.locator("article").filter({ hasText: "Alpha" }).first().waitFor();
  assert.equal(await page.locator(".usage-today-card").count(), 0, "the accounts page carries no token monitor");
  await reset("usage-page");
  await page.locator(".usage-today-card").first().waitFor();
  assert.equal(await page.getByRole("heading", { level: 2, name: await tr("usage.title") }).count(), 1);
  assert.equal(await page.locator(".usage-today-card").count(), 2);
  console.log("PASS usage has its own page and the accounts page keeps only accounts");

  await reset("accounts");
  const tabs = page.getByRole("radiogroup");
  await tabs.getByRole("radio", { name: /^Codex\s*3$/ }).waitFor();
  assert.equal(await tabs.getByRole("radio").count(), 4);
  assert.equal(await tabs.getByRole("radio", { name: /^Codex/ }).getAttribute("aria-checked"), "true");
  await tabs.getByRole("radio", { name: /^WorkBuddy\s*2$/ }).click();
  const workbuddy = page.locator('[data-account-source="workbuddy"]');
  await workbuddy.waitFor();
  assert.equal(await page.locator("article").filter({ hasText: "Alpha" }).count(), 0, "Codex cards leave with their tab");
  assert.equal(await workbuddy.locator("[data-source-account]").count(), 2);
  const active = workbuddy.locator('[data-source-account="wb-1"]');
  assert.match(await active.locator("[data-source-quota]").innerText(), /750\s*\/\s*1,000/);
  assert.equal(await active.getByRole("meter").getAttribute("aria-valuenow"), "25");
  await workbuddy.locator('[data-source-account="wb-2"]').getByText(await tr("accountSources.lapsed")).waitFor();
  assert.match(await workbuddy.locator("[data-source-note]").innerText(), /OpenCode/);
  assert.equal(await workbuddy.getByRole("switch").getAttribute("aria-checked"), "true");
  assert.equal(await workbuddy.locator("[data-source-channel]").getAttribute("data-source-channel"), "live",
    "shared with a signed-in current account: account → router → model inject all carry traffic");
  assert.equal(await workbuddy.locator('[data-source-channel] [data-signal="live"]').count(), 2);
  await workbuddy.getByText(await tr("accountSources.exposedModels", { count: 2 }), { exact: true }).waitFor();
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "accounts-workbuddy.png"), fullPage: true });

  await workbuddy.locator('[data-source-account="wb-2"]').getByRole("button", { name: await tr("accounts.setCurrent") }).click();
  await workbuddy.locator('[data-source-account="wb-2"]').getByText(await tr("accounts.current"), { exact: true }).first().waitFor();
  await active.getByRole("button", { name: await tr("accountSources.refreshQuota") }).click();
  const channel = workbuddy.locator("[data-source-channel]");
  // The current account is the expired one now: model inject still lists the
  // source, but nothing reaches the router from its account.
  assert.equal(await channel.getAttribute("data-source-channel"), "waiting");
  assert.equal(await channel.locator('[data-signal="live"]').count(), 1);
  await workbuddy.getByRole("switch").click();
  await workbuddy.getByText(await tr("accountSources.notExposed"), { exact: true }).waitFor();
  assert.deepEqual(await calls(), [["activate", "wb-2"], ["refreshQuota", "wb-1"], ["setExposed", "workbuddy", false]]);
  await page.waitForFunction(() => document.querySelector("[data-source-channel]")?.getAttribute("data-source-channel") === "idle");
  assert.equal(await channel.locator('[data-signal="live"]').count(), 0, "unshared, the lines fall back to dashed hairlines");
  console.log("PASS account-source tabs list each source's accounts, credits and expired sign-ins and toggle its share");

  await page.evaluate(() => { window.review.keepAccountsTab = true; window.review.mount("accounts"); });
  await page.locator('[data-account-source="workbuddy"]').waitFor();
  await page.evaluate(() => { window.review.keepAccountsTab = false; });
  console.log("PASS the accounts page reopens on the last source");

  // Moving right along the sources slides the panel in from the right; under
  // reduced motion it only fades and no packet travels the channel.
  await page.getByRole("radiogroup").getByRole("radio", { name: /^WorkBuddy AI/ }).click();
  const forward = page.locator('[data-accounts-panel="workbuddy-ai"]');
  assert.equal(await forward.getAttribute("data-direction"), "forward");
  assert.equal(await forward.evaluate(element => getComputedStyle(element).animationName), "accounts-panel-in");
  await page.getByRole("radiogroup").getByRole("radio", { name: /^WorkBuddy\s*\d+$/ }).click();
  assert.equal(await page.locator('[data-accounts-panel="workbuddy"]').getAttribute("data-direction"), "back");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.evaluate(() => { window.review.keepAccountsTab = true; window.review.mount("accounts"); });
  await page.locator('[data-source-channel="live"]').waitFor();
  assert.equal(await page.locator("[data-source-channel] animateMotion").count(), 0);
  await page.getByRole("radiogroup").getByRole("radio", { name: /^ChatGPT/ }).click();
  assert.equal(await page.locator('[data-accounts-panel="chatgpt-api"]').evaluate(element => getComputedStyle(element).animationName), "accounts-panel-fade");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => { window.review.keepAccountsTab = false; });
  console.log("PASS switching sources moves along the row of sources and reduced motion keeps only a fade");

  await page.getByRole("radiogroup").getByRole("radio", { name: /^ChatGPT\s*0$/ }).click();
  const chatgpt = page.locator('[data-account-source="chatgpt-api"]');
  await chatgpt.waitFor();
  assert.equal(await chatgpt.getByRole("switch").isDisabled(), true, "nothing to share before a model list exists");
  const signIn = await tr("accountSources.signIn", { name: "ChatGPT" });
  assert.equal(await page.getByRole("button", { name: signIn }).count(), 2, "hero and empty state both offer the sign-in");
  await chatgpt.getByRole("button", { name: signIn }).click();
  await chatgpt.getByRole("status").getByText(await tr("accountSources.signInHint")).waitFor();
  assert.equal(await page.getByRole("button", { name: signIn }).count(), 1, "the hero sign-in hides while one is pending");
  await chatgpt.getByRole("button", { name: await tr("accounts.cancelLogin") }).click();
  await page.waitForFunction(() => window.review.sourceCalls.some(call => call[0] === "cancelSignIn"));
  await page.evaluate(() => window.review.sourceSignIns[0].reject("accounts.loginCancelled"));
  await chatgpt.getByRole("status").waitFor({ state: "detached" });
  await page.getByRole("button", { name: signIn }).first().click();
  await page.evaluate(() => window.review.sourceSignIns[1].resolve());
  const added = chatgpt.locator('[data-source-account="new-1"]');
  await added.waitFor();
  assert.match(await added.locator("h4").innerText(), /^m•+e@example\.com$/, "an email name is masked");
  await added.getByText(/chatgpt\.com/).waitFor();
  assert.equal(await chatgpt.getByRole("switch").getAttribute("aria-checked"), "true");
  await added.getByRole("button", { name: await tr("accounts.remove") }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("common.delete") }).click();
  await added.waitFor({ state: "detached" });
  assert.deepEqual((await calls()).map(call => call[0]).slice(-1), ["remove"]);
  console.log("PASS a source sign-in can be cancelled, then adds a masked account, and removal asks first");

  // An account source in the inject dialog: badged, router-only, and Claude
  // Code is told WorkBuddy refuses its prompt.
  await reset("compact");
  await page.evaluate(() => {
    const review = window.review;
    const base = review.getState().providers[0];
    const model = id => ({ ...base.models[0], id, displayName: id, included: true, protocol: "chatCompletions", endpointTypes: ["chatCompletions"] });
    review.providerFixtures = [
      { ...base, id: "relay-a", name: "Relay A", enabled: true, hasApiKey: true, protocol: "chatCompletions", model: "relay-model", models: [model("relay-model")] },
      { ...base, id: "account-workbuddy", name: "WorkBuddy", enabled: true, hasApiKey: false, accountSource: "workbuddy",
        protocol: "chatCompletions", model: "glm-5.3", models: [model("glm-5.3")] },
    ];
    review.clientStatusFixtures = [{ targetId: "claudeCode", name: "Claude Code", path: "C:/synthetic/settings.json",
      protocols: ["anthropicMessages"], configured: false, conflict: false, error: null, providerId: null, model: null, request: null }];
    review.mount("clients");
  });
  await page.getByRole("button", { name: await tr("clients.configure"), exact: true }).first().click();
  const dialog = page.getByRole("dialog");
  const card = dialog.getByRole("button").filter({ hasText: "WorkBuddy" });
  await card.waitFor();
  await card.locator("[data-account-source-badge]").waitFor();
  assert.equal(await card.isDisabled(), true, "direct mode cannot reach an account source");
  assert.match(await card.innerText(), new RegExp(await tr("clients.excluded.routerOnly", { count: 1 })));
  await dialog.getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  assert.equal(await card.isDisabled(), false, "the router serves it");
  await card.locator("[data-account-source-refusal]").waitFor();
  if (captureDir) await dialog.screenshot({ path: path.join(captureDir, "inject-account-source.png") });
  console.log("PASS model inject badges account sources, keeps them router-only and warns Claude Code about WorkBuddy");
}
