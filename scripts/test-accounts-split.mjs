import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// Accounts and usage on their own pages, account-source tabs with sign-in,
// sharing with model inject, and account sources in the inject dialog.
export async function testAccountsSplit({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = () => page.evaluate(() => window.review.sourceCalls);
  // A panel's share switch, named by its label (check-in has switches of its own).
  const share = async (panel) => panel.getByRole("switch", { name: await tr("accountSources.expose"), exact: true });

  await reset("accounts");
  await page.locator("article").filter({ hasText: "Alpha" }).first().waitFor();
  await eventually(async () => assert.equal(await page.locator("[data-usage-kpi]").count(), 0, "the accounts page carries no token monitor"));
  await reset("usage-page");
  await page.locator("[data-usage-kpi]").first().waitFor();
  await eventually(async () => assert.equal(await page.getByRole("heading", { level: 2, name: await tr("usage.title") }).count(), 1));
  assert.equal(await page.locator("[data-usage-kpi]").count(), 4);
  console.log("PASS usage has its own page and the accounts page keeps only accounts");

  await reset("accounts");
  const tabs = page.getByRole("radiogroup");
  const berth = async (name, count) => tabs.getByRole("radio", { name: await tr("accountSources.berth", { name, count }), exact: true });
  await (await berth("Codex", 3)).waitFor();
  await eventually(async () => assert.equal(await tabs.getByRole("radio").count(), 20));
  assert.deepEqual(await page.locator("[data-pier]").evaluateAll(piers => piers.map(pier => pier.getAttribute("data-pier"))),
    ["openai", "tencent", "github", "google", "aws", "xai", "cursor", "alibaba", "bytedance", "minimax", "zhipu", "opencode", "commandcode"],
    "channels sit on their vendors' piers");
  assert.equal(await tabs.getByRole("radio", { name: /^Codex/ }).getAttribute("aria-checked"), "true");
  const hullLeft = () => page.locator(".source-dock-hull").evaluate(element => parseFloat(element.style.left));
  const codexLeft = await hullLeft();
  await (await berth("WorkBuddy", 2)).click();
  await page.waitForFunction(left => parseFloat(document.querySelector(".source-dock-hull").style.left) > left, codexLeft);
  const wbBerth = page.locator('[data-berth="workbuddy"]');
  assert.deepEqual(await wbBerth.locator("[data-light]").evaluateAll(lights => lights.map(light => light.getAttribute("data-light"))),
    ["active", "lapsed"], "one light per account: the current one and the expired one");
  assert.equal(await wbBerth.locator("[data-berth-shared]").count(), 1, "a shared channel carries the link mark");
  assert.equal(await page.locator('[data-berth="copilot"]').getByText(await tr("accountSources.berthEmpty")).count(), 1);
  assert.equal(await page.locator("[data-dock-mooring]").count(), 1, "a mooring line drops from the chosen berth to its panel");
  assert.equal(await page.locator('[data-berth="workbuddy"] .source-berth-glyph').evaluate(element => element.classList.contains("is-docking")), true);
  const workbuddy = page.locator('[data-account-source="workbuddy"]');
  await workbuddy.waitFor();
  await eventually(async () => assert.equal(await page.locator("article").filter({ hasText: "Alpha" }).count(), 0, "Codex cards leave with their tab"));
  assert.equal(await workbuddy.locator("[data-source-account]").count(), 2);
  const active = workbuddy.locator('[data-source-account="wb-1"]');
  assert.match(await active.locator("[data-source-quota]").innerText(), /750\s*\/\s*1,000/);
  assert.equal(await active.getByRole("meter").getAttribute("aria-valuenow"), "25");
  await workbuddy.locator('[data-source-account="wb-2"]').getByText(await tr("accountSources.lapsed")).waitFor();
  await eventually(async () => assert.match(await workbuddy.locator("[data-source-note]").innerText(), /Claude Code/));
  assert.equal(await (await share(workbuddy)).getAttribute("aria-checked"), "true");
  assert.equal(await workbuddy.locator("[data-source-channel]").getAttribute("data-source-channel"), "live",
    "shared with a signed-in current account: account → router → model inject all carry traffic");
  assert.equal(await workbuddy.locator('[data-source-channel] [data-signal="live"]').count(), 2);
  await workbuddy.getByText(await tr("accountSources.exposedModels", { count: 2 }), { exact: true }).waitFor();
  const modelList = workbuddy.locator("[data-source-models]");
  assert.equal(await modelList.locator("details, summary").count(), 0, "no native disclosure triangle");
  const modelFold = modelList.getByRole("button", { name: await tr("accountSources.modelListShared", { shared: 2, count: 2 }) });
  assert.equal(await modelFold.getAttribute("aria-expanded"), "false");
  await modelList.locator("li").first().waitFor({ state: "detached" });
  await modelFold.click();
  await eventually(async () => assert.deepEqual(await modelList.locator("li").allInnerTexts(), ["GLM-5.3", "Kimi-K3"]));
  await modelFold.click();
  await modelList.locator("li").first().waitFor({ state: "detached" });
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "accounts-workbuddy.png"), fullPage: true });

  await workbuddy.locator('[data-source-account="wb-2"]').getByRole("button", { name: await tr("accounts.setCurrent") }).click();
  await workbuddy.locator('[data-source-account="wb-2"]').getByText(await tr("accounts.current"), { exact: true }).first().waitFor();
  await active.getByRole("button", { name: await tr("accountSources.refreshQuota") }).click();
  const channel = workbuddy.locator("[data-source-channel]");
  // The current account is the expired one now: model inject still lists the
  // source, but nothing reaches the router from its account.
  assert.equal(await channel.getAttribute("data-source-channel"), "waiting");
  assert.equal(await channel.locator('[data-signal="live"]').count(), 1);
  await (await share(workbuddy)).click();
  await workbuddy.getByText(await tr("accountSources.notExposed"), { exact: true }).waitFor();
  assert.deepEqual(await calls(), [["activate", "wb-2"], ["refreshQuota", "wb-1"], ["setExposed", "workbuddy", false]]);
  await page.waitForFunction(() => document.querySelector("[data-source-channel]")?.getAttribute("data-source-channel") === "idle");
  await eventually(async () => assert.equal(await channel.locator('[data-signal="live"]').count(), 0, "unshared, the lines fall back to dashed hairlines"));
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
  await page.getByRole("radiogroup").getByRole("radio", { name: await tr("accountSources.berth", { name: "WorkBuddy", count: 2 }), exact: true }).click();
  await eventually(async () => assert.equal(await page.locator('[data-accounts-panel="workbuddy"]').getAttribute("data-direction"), "back"));
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.evaluate(() => { window.review.keepAccountsTab = true; window.review.mount("accounts"); });
  await page.locator('[data-source-channel="live"]').waitFor();
  await eventually(async () => assert.equal(await page.locator("[data-source-channel] animateMotion").count(), 0));
  await page.getByRole("radiogroup").getByRole("radio", { name: /^ChatGPT/ }).click();
  assert.equal(await page.locator('[data-accounts-panel="chatgpt-api"]').evaluate(element => getComputedStyle(element).animationName), "accounts-panel-fade");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => { window.review.keepAccountsTab = false; });
  console.log("PASS switching sources moves along the row of sources and reduced motion keeps only a fade");

  await page.getByRole("radiogroup").getByRole("radio", { name: await tr("accountSources.berth", { name: "ChatGPT", count: 0 }), exact: true }).click();
  const chatgpt = page.locator('[data-account-source="chatgpt-api"]');
  await chatgpt.waitFor();
  await eventually(async () => assert.equal(await (await share(chatgpt)).isDisabled(), true, "nothing to share before a model list exists"));
  const signIn = await tr("accountSources.signIn", { name: "ChatGPT" });
  assert.equal(await page.getByRole("button", { name: signIn }).count(), 2, "hero and empty state both offer the sign-in");
  await chatgpt.getByRole("button", { name: signIn }).click();
  await chatgpt.getByRole("status").getByText(await tr("accountSources.signInHint")).waitFor();
  await eventually(async () => assert.equal(await page.getByRole("button", { name: signIn }).count(), 1, "the hero sign-in hides while one is pending"));
  await chatgpt.getByRole("button", { name: await tr("accounts.cancelLogin") }).click();
  await page.waitForFunction(() => window.review.sourceCalls.some(call => call[0] === "cancelSignIn"));
  await page.evaluate(() => window.review.sourceSignIns[0].reject("accounts.loginCancelled"));
  await chatgpt.getByRole("status").waitFor({ state: "detached" });
  await page.getByRole("button", { name: signIn }).first().click();
  await page.evaluate(() => window.review.sourceSignIns[1].resolve());
  const added = chatgpt.locator('[data-source-account="new-1"]');
  await added.waitFor();
  await eventually(async () => assert.match(await added.locator("h4").innerText(), /^m•+e@example\.com$/, "an email name is masked"));
  await added.getByText(/chatgpt\.com/).waitFor();
  await eventually(async () => assert.equal(await (await share(chatgpt)).getAttribute("aria-checked"), "true"));
  await added.getByRole("button", { name: await tr("accounts.remove") }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("common.delete") }).click();
  await added.waitFor({ state: "detached" });
  assert.deepEqual((await calls()).map(call => call[0]).slice(-1), ["remove"]);
  console.log("PASS a source sign-in can be cancelled, then adds a masked account, and removal asks first");

  // Arrow keys walk the berths; Copilot's device sign-in shows the code to type at GitHub.
  await page.locator('[data-berth="chatgpt-api"]').focus();
  await page.keyboard.press("End");
  await page.locator('[data-account-source="commandcode"]').waitFor();
  await page.locator('[data-pier="commandcode"] [data-berth="commandcode"]').waitFor();
  for (let step = 0; step < 15; step++) await page.keyboard.press("ArrowLeft");
  await page.locator('[data-account-source="copilot"]').waitFor();
  await eventually(async () => assert.equal(await page.locator('[data-berth="copilot"]').getAttribute("aria-checked"), "true"));
  await page.evaluate(() => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { window.review.copied = text; } } });
  });
  const copilot = page.locator('[data-account-source="copilot"]');
  await copilot.getByRole("button", { name: await tr("accountSources.signIn", { name: "GitHub Copilot" }) }).click();
  const code = copilot.locator("[data-device-code] code");
  await code.waitFor();
  assert.equal((await code.innerText()).replace(/\s/g, ""), "ABCD-1234");
  await copilot.getByRole("status").getByText(await tr("accountSources.deviceCodeHint")).waitFor();
  await copilot.getByRole("button", { name: await tr("accountSources.copyCode") }).click();
  await page.waitForFunction(() => window.review.copied === "ABCD-1234");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "accounts-copilot-device-code.png"), fullPage: true });
  await page.evaluate(() => window.review.sourceSignIns.at(-1).resolve());
  await copilot.locator('[data-source-account="new-1"]').waitFor();
  await eventually(async () => assert.equal(await copilot.locator("[data-device-code]").count(), 0, "the code leaves once the account is in"));
  await page.locator('[data-berth="copilot"] [data-light="active"]').waitFor();
  console.log("PASS the channel dock walks by keyboard and Copilot's device sign-in shows its code until the account is in");

  // Antigravity asks the user to accept its risk first; Gemini CLI takes a Google Cloud project.
  await page.locator('[data-berth="antigravity"]').click();
  const antigravity = page.locator('[data-account-source="antigravity"]');
  await antigravity.locator("[data-source-risk]").waitFor();
  const agSignIn = antigravity.getByRole("button", { name: await tr("accountSources.signIn", { name: "Antigravity" }) });
  const signInsBefore = (await calls()).filter(call => call[0] === "signIn").length;
  await agSignIn.click();
  const risk = page.getByRole("alertdialog");
  await risk.getByText(await tr("accountSources.risk.antigravity")).waitFor();
  await risk.getByRole("button", { name: await tr("common.cancel") }).click();
  await risk.waitFor({ state: "detached" });
  assert.equal((await calls()).filter(call => call[0] === "signIn").length, signInsBefore, "declining the risk signs in to nothing");
  await agSignIn.click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("accountSources.riskAccept") }).click();
  await page.waitForFunction(count => window.review.sourceCalls.filter(call => call[0] === "signIn").length === count + 1, signInsBefore);
  assert.deepEqual((await calls()).filter(call => call[0] === "signIn").at(-1), ["signIn", "antigravity"]);
  await antigravity.getByRole("button", { name: await tr("accounts.cancelLogin") }).click();
  await page.evaluate(() => window.review.sourceSignIns.at(-1).reject("accounts.loginCancelled"));

  await page.locator('[data-berth="gemini-cli"]').click();
  const gemini = page.locator('[data-account-source="gemini-cli"]');
  const project = gemini.getByRole("textbox", { name: await tr("accountSources.project") });
  await project.fill("corp-ai-123");
  await gemini.locator("[data-source-project]").getByRole("button", { name: await tr("common.save") }).click();
  await gemini.getByText(await tr("accountSources.usedPercent", { value: 20 })).waitFor();
  assert.deepEqual((await calls()).filter(call => call[0] === "setProject").at(-1), ["setProject", "gc-1", "corp-ai-123"]);
  console.log("PASS Antigravity asks to accept its risk before signing in, and a Gemini CLI account takes its Google Cloud project");

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
      { ...base, id: "account-antigravity", name: "Antigravity", enabled: true, hasApiKey: false, accountSource: "antigravity",
        protocol: "chatCompletions", model: "gemini-3.5-flash", models: [model("gemini-3.5-flash")] },
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
  await eventually(async () => assert.equal(await card.isDisabled(), true, "direct mode cannot reach an account source"));
  assert.match(await card.innerText(), new RegExp(await tr("clients.excluded.routerOnly", { count: 1 })));
  await dialog.getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  await eventually(async () => assert.equal(await card.isDisabled(), false, "the router serves it"));
  await dialog.getByRole("button").filter({ hasText: "Antigravity" }).locator("[data-account-source-refusal]").waitFor();
  assert.equal(await card.locator("[data-account-source-refusal]").count(), 0, "WorkBuddy's flagged phrases are reworded, so Claude Code isn't warned off it");
  if (captureDir) await dialog.screenshot({ path: path.join(captureDir, "inject-account-source.png") });
  console.log("PASS model inject badges account sources, keeps them router-only and warns Claude Code about Antigravity only");

  // A source's models can be left out of the share; leaving all out stops it.
  await reset("accounts");
  await page.locator('[data-berth="workbuddy"]').click();
  const filtered = page.locator('[data-account-source="workbuddy"]');
  const models = filtered.locator("[data-source-models]");
  await models.getByRole("button", { name: await tr("accountSources.modelListShared", { shared: 2, count: 2 }) }).click();
  const kimi = models.locator('[data-source-model="kimi-k3-1"]');
  const glm = models.locator('[data-source-model="glm-5.3"]');
  assert.equal(await kimi.getAttribute("aria-pressed"), "true");
  assert.equal(await models.locator("[data-models-none]").isDisabled(), false, "a shared source can leave every model out");
  await kimi.click();
  await page.waitForFunction(() => window.review.sourceCalls.some(call => call[0] === "setHiddenModels"));
  assert.deepEqual((await calls()).filter(call => call[0] === "setHiddenModels").at(-1), ["setHiddenModels", "workbuddy", ["kimi-k3-1"]]);
  await models.locator('[data-source-model="kimi-k3-1"][aria-pressed="false"]').waitFor();
  await models.getByText(await tr("accountSources.modelListShared", { shared: 1, count: 2 })).waitFor();
  assert.equal(await glm.isDisabled(), false, "the last shared model can be left out too");
  await models.locator("[data-models-all]").click();
  await models.locator('[data-source-model="kimi-k3-1"][aria-pressed="true"]').waitFor();
  assert.deepEqual((await calls()).filter(call => call[0] === "setHiddenModels").at(-1), ["setHiddenModels", "workbuddy", []]);
  const shared = await share(filtered);
  assert.equal(await shared.getAttribute("aria-checked"), "true");
  await models.locator("[data-models-none]").click();
  await models.getByText(await tr("accountSources.modelListShared", { shared: 0, count: 2 })).waitFor();
  assert.deepEqual((await calls()).filter(call => call[0] === "setHiddenModels").at(-1), ["setHiddenModels", "workbuddy", ["glm-5.3", "kimi-k3-1"]]);
  await eventually(async () => assert.equal(await shared.getAttribute("aria-checked"), "false", "leaving every model out stops the share"));
  assert.equal(await shared.isDisabled(), true, "nothing to share until a model is chosen");
  await filtered.getByText(await tr("accountSources.noModelsChosen")).waitFor();
  console.log("PASS a source's models can be left out of the share and brought back; leaving all out stops sharing");

  // Daily check-in: the source's switch and press, each account's status and its own switch.
  await reset("accounts");
  await page.locator('[data-berth="workbuddy"]').click();
  const checkin = page.locator('[data-account-source="workbuddy"]');
  const controls = checkin.locator("[data-checkin-controls]");
  const auto = controls.getByRole("switch", { name: await tr("accountSources.checkin.auto"), exact: true });
  assert.equal(await auto.getAttribute("aria-checked"), "true");
  const claimed = checkin.locator('[data-source-account="wb-1"] [data-account-checkin="claimed"]');
  await eventually(async () => assert.match(await claimed.innerText(), new RegExp(await tr("accountSources.checkin.outcome.claimed"))));
  assert.match(await claimed.innerText(), /\+5/);
  assert.equal(await checkin.locator('[data-source-account="wb-2"] [data-account-checkin]').count(), 0, "an expired sign-in shows no check-in");
  const now = controls.getByRole("button", { name: await tr("accountSources.checkin.now") });
  await now.click();
  await page.waitForFunction(() => window.review.sourceCalls.some(call => call[0] === "checkIn"));
  // The toast says what came of it: the signed-out account takes no part.
  const ran = [await tr("accountSources.checkin.ran"), await tr("accountSources.checkin.tally.claimed", { count: 1 })];
  await eventually(async () => assert.deepEqual(
    await page.evaluate(() => [window.review.lastToast, window.review.lastToastDescription]), ran));
  await claimed.getByRole("switch").click();
  const leftOut = checkin.locator('[data-source-account="wb-1"] [data-account-checkin="off"]');
  await leftOut.waitFor();
  assert.equal(await leftOut.getByRole("switch").getAttribute("aria-checked"), "false");
  await eventually(async () => assert.equal(await now.isDisabled(), true, "with every account left out or signed out there is nothing to press"));
  assert.equal(await now.locator("xpath=..").getAttribute("title"), await tr("accountSources.checkin.noneUsable"), "a disabled press says why");
  await auto.click();
  await eventually(async () => assert.equal(await auto.getAttribute("aria-checked"), "false"));
  assert.deepEqual((await calls()).filter(call => call[0].startsWith("setCheckin") || call[0] === "checkIn"),
    [["checkIn", "workbuddy"], ["setCheckinOff", "wb-1", true], ["setCheckin", "workbuddy", false]]);
  if (captureDir) {
    await page.waitForTimeout(600);
    await page.screenshot({ path: path.join(captureDir, "accounts-checkin.png"), fullPage: true });
  }
  await page.locator('[data-berth="chatgpt-api"]').click();
  await page.locator('[data-account-source="chatgpt-api"]').waitFor();
  assert.equal(await page.locator("[data-checkin-controls]").count(), 0, "a source with no check-in shows none");
  console.log("PASS a source's daily check-in is switched and pressed, and each account shows how it went and takes part or not");

  // A source with several ways in offers a menu; a key asks for itself first.
  await page.locator('[data-berth="commandcode"]').click();
  const commandcode = page.locator('[data-account-source="commandcode"]');
  await commandcode.waitFor();
  await commandcode.locator('[data-sign-in-methods="commandcode"]').click();
  const menu = page.getByRole("menu");
  assert.equal(await menu.getByRole("menuitem").count(), 3);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "accounts-sign-in-methods.png") });
  await menu.getByRole("menuitem", { name: await tr("accountSources.method.commandcode.key") }).click();
  const keyDialog = page.getByRole("dialog");
  const field = keyDialog.getByRole("textbox").or(keyDialog.locator('input[type="password"]'));
  await field.first().fill("  user_secret  ");
  // A secret is typed behind the shared show/hide eye.
  assert.equal(await keyDialog.locator("input").getAttribute("type"), "password");
  await keyDialog.getByRole("button", { name: "show", exact: true }).click();
  assert.equal(await keyDialog.locator("input").getAttribute("type"), "text");
  await keyDialog.getByRole("button", { name: "hide", exact: true }).click();
  await keyDialog.getByRole("button", { name: await tr("accountSources.input.submit") }).click();
  await page.waitForFunction(() => window.review.sourceCalls.some(call => call[0] === "signIn" && call[1] === "commandcode"));
  assert.deepEqual((await calls()).filter(call => call[0] === "signIn").at(-1), ["signIn", "commandcode", "key", "user_secret"]);
  await page.evaluate(() => window.review.sourceSignIns.at(-1).reject("accounts.loginCancelled"));
  console.log("PASS a source's sign-in methods come as a menu, and a key is typed before signing in");
}
