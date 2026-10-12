import assert from "node:assert/strict";
import path from "node:path";

// The general gateway panel: keep-running switch, addresses, named keys
// (reveal, copy, regenerate, delete), the model list by short name, the
// connection export, and the short-name field of the relay editor.
export async function testGateway({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = () => page.evaluate(() => window.review.gatewayCalls);
  const choose = async (selector, name) => {
    await selector.click();
    await page.getByRole("option", { name, exact: true }).click();
    await page.getByRole("option", { name, exact: true }).waitFor({ state: "hidden" });
  };

  await reset("access");
  await page.evaluate(() => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => { window.review.copied = text; } } });
  });
  const gateway = page.locator("[data-gateway]");
  await gateway.waitFor();
  assert.equal(await gateway.locator("[data-gateway-status]").innerText(), await tr("gateway.stopped"));
  const endpoints = await gateway.locator("[data-gateway-endpoint] code").allInnerTexts();
  assert.deepEqual(endpoints, ["http://127.0.0.1:43127/gateway/v1", "http://127.0.0.1:43127/gateway"]);
  await gateway.getByText(await tr("gateway.noKeys"), { exact: true }).waitFor();

  const keyName = gateway.getByRole("textbox", { name: await tr("gateway.keyName") });
  await keyName.fill("Hermes");
  await gateway.getByRole("button", { name: await tr("gateway.createKey") }).click();
  const hermes = gateway.locator('[data-gateway-key="key-1"]');
  await hermes.waitFor();
  assert.equal(await hermes.locator("[data-gateway-secret]").innerText(), "sk-ah-…0001");
  assert.equal(await keyName.inputValue(), "", "the name field clears after creating");
  await keyName.fill("Hermes");
  await gateway.getByRole("button", { name: await tr("gateway.createKey") }).click();
  await gateway.getByRole("alert").getByText(await tr("gateway.errors.keyNameTaken")).waitFor();
  await keyName.fill("");

  await hermes.getByRole("button", { name: await tr("gateway.showKey") }).click();
  await hermes.locator("[data-gateway-secret]").getByText("sk-ah-fixture-hermes-0001").waitFor();
  await hermes.getByRole("button", { name: await tr("gateway.hideKey") }).click();
  await hermes.locator("[data-gateway-secret]").getByText("sk-ah-…0001").waitFor();
  await hermes.getByRole("button", { name: await tr("gateway.copyKey") }).click();
  await page.waitForFunction(() => window.review.copied === "sk-ah-fixture-hermes-0001");
  console.log("PASS the gateway shows its addresses, creates named keys, refuses a duplicate name and reveals a key only on request");

  const schematic = gateway.locator("[data-gateway-schematic]");
  assert.equal(await schematic.getAttribute("data-gateway-schematic"), "idle");
  await schematic.locator('[data-schematic-key="key-1"]').waitFor();
  assert.equal(await schematic.locator('[data-signal="live"]').count(), 0, "a stopped gateway shows dashed lines only");
  await gateway.getByRole("switch", { name: await tr("gateway.keepRunning") }).click();
  await gateway.locator("[data-gateway-status]").getByText(await tr("gateway.running")).waitFor();
  assert.equal(await schematic.getAttribute("data-gateway-schematic"), "live");
  // One key line plus a line to each kind of model that has any.
  assert.equal(await schematic.locator('[data-signal="live"]').count(), 4);
  assert.equal(await schematic.locator('[data-schematic-kind="account"]').getAttribute("data-live"), "true");

  await gateway.getByRole("button", { name: await tr("gateway.models", { count: 4 }) }).click();
  assert.deepEqual(await gateway.locator("[data-gateway-group]").evaluateAll(groups => groups.map(group => group.getAttribute("data-gateway-group"))),
    ["relay", "account", "aggregate"]);
  assert.deepEqual(await gateway.locator("[data-gateway-model]").evaluateAll(rows => rows.map(row => row.getAttribute("data-gateway-model"))),
    ["relay-a/fast-model", "relay-a/vendor/deep-model", "workbuddy/glm-5.3", "pool/harbor-pool"]);
  await gateway.getByRole("textbox", { name: await tr("gateway.filterModels") }).fill("glm");
  assert.deepEqual(await gateway.locator("[data-gateway-model]").evaluateAll(rows => rows.map(row => row.getAttribute("data-gateway-model"))),
    ["workbuddy/glm-5.3"]);
  await gateway.locator('[data-gateway-model="workbuddy/glm-5.3"]').getByRole("button", { name: await tr("gateway.copyModel") }).click();
  await page.waitForFunction(() => window.review.copied === "workbuddy/glm-5.3");
  await gateway.getByRole("textbox", { name: await tr("gateway.filterModels") }).fill("");
  console.log("PASS the gateway keeps running on request and lists relay, account and aggregate models by their gateway names");

  const prefix = gateway.getByRole("textbox", { name: await tr("gateway.aggregatePrefix") });
  assert.equal(await prefix.inputValue(), "pool");
  const savePrefix = gateway.locator("[data-gateway-prefix]").getByRole("button", { name: await tr("common.save") });
  assert.equal(await savePrefix.isDisabled(), true, "nothing to save until the prefix changes");
  await prefix.fill("Relay-A");
  assert.equal(await prefix.inputValue(), "relay-a", "the prefix is typed in lowercase");
  await savePrefix.click();
  await gateway.getByRole("alert").getByText(await tr("gateway.errors.prefixTaken")).waitFor();
  await prefix.fill("mix");
  await savePrefix.click();
  await gateway.locator('[data-gateway-model="mix/harbor-pool"]').waitFor();
  assert.deepEqual((await calls()).filter(call => call[0] === "prefix").map(call => call[1]), ["relay-a", "mix"]);
  await prefix.fill("pool");
  await savePrefix.click();
  await gateway.locator('[data-gateway-model="pool/harbor-pool"]').waitFor();
  console.log("PASS aggregates carry an editable gateway prefix that no relay short name may take");

  await gateway.getByRole("button", { name: await tr("gateway.connection") }).click();
  await gateway.getByRole("button", { name: await tr("gateway.preview"), exact: true }).click();
  const preview = gateway.locator("[data-gateway-preview]");
  await preview.waitFor();
  assert.match(await preview.innerText(), /<gateway key>/);
  assert.match(await preview.innerText(), /127\.0\.0\.1:43127\/gateway\/v1/);
  await gateway.getByRole("button", { name: await tr("gateway.copyConnection") }).click();
  await page.waitForFunction(() => window.review.copied?.includes("sk-ah-fixture-hermes-0001"));
  await choose(gateway.locator("#gateway-export-profile"), "Anthropic SDK");
  await gateway.locator("#gateway-export-protocol").click();
  assert.equal(await page.getByRole("option", { name: "Responses", exact: true }).getAttribute("aria-disabled"), "true");
  assert.equal(await page.getByRole("option", { name: "Chat", exact: true }).getAttribute("aria-disabled"), "true");
  await page.keyboard.press("Escape");
  await gateway.getByRole("button", { name: await tr("gateway.preview"), exact: true }).click();
  assert.match(await preview.innerText(), /127\.0\.0\.1:43127\/gateway"/, "the Anthropic SDK takes the bare gateway root");
  await choose(gateway.locator("#gateway-export-profile"), "OpenCode");
  await choose(gateway.locator("#gateway-export-protocol"), "Chat");
  assert.equal(await preview.count(), 0, "changing a setting withdraws the previous preview");
  await choose(gateway.locator("#gateway-export-model"), "pool/harbor-pool");
  await gateway.getByRole("button", { name: await tr("gateway.preview"), exact: true }).click();
  const exports = (await calls()).filter(call => call[0] === "export").map(call => call[1]);
  assert.deepEqual(exports.map(request => request.revealCredential), [false, true, false, false]);
  assert.deepEqual({ ...exports.at(-1) }, { ingressProtocol: "chatCompletions", selectedModelId: "pool/harbor-pool", consumerProfile: "opencode",
    keyId: "key-1", revealCredential: false });
  await page.evaluate(async () => {
    window.review.gatewayModels = [];
    await window.review.frontendClient.invalidateQueries({ queryKey: ["gateway"] });
  });
  await gateway.getByRole("button", { name: await tr("gateway.preview"), exact: true }).and(page.locator(":disabled")).waitFor();
  assert.equal(await gateway.getByRole("button", { name: await tr("gateway.copyConnection") }).isDisabled(), true,
    "an unavailable explicit choice is never replaced by another model");
  await page.evaluate(async () => {
    delete window.review.gatewayModels;
    await window.review.frontendClient.invalidateQueries({ queryKey: ["gateway"] });
  });
  if (captureDir) await gateway.screenshot({ path: path.join(captureDir, "gateway-panel.png") });
  console.log("PASS connection settings mask the key in previews, copy reveals it, and an unavailable model blocks export");

  await hermes.getByRole("button", { name: await tr("gateway.regenerate") }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("gateway.regenerate") }).click();
  await hermes.locator("[data-gateway-secret]").getByText("sk-ah-…0002").waitFor();
  await hermes.getByRole("button", { name: await tr("common.delete") }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("common.delete") }).click();
  await hermes.waitFor({ state: "detached" });
  assert.deepEqual((await calls()).map(call => call[0]).filter(name => ["regenerate", "delete"].includes(name)), ["regenerate", "delete"]);
  console.log("PASS regenerating or deleting a gateway key asks first");

  await reset("provider");
  await page.evaluate(() => {
    window.review.dialogProviderFixture = { ...window.review.getState().providers[0], gatewaySlug: "relay-alpha" };
    window.review.mount("provider");
  });
  const editor = page.getByRole("dialog");
  const slug = editor.locator("#newapi-provider-gatewaySlug");
  assert.equal(await slug.inputValue(), "relay-alpha");
  await slug.fill("alpha");
  await editor.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.saved?.gatewaySlug === "alpha");
  console.log("PASS the relay editor saves the gateway short name");
}
