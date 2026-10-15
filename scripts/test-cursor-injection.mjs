import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// Cursor injection: the console tab, the certificate gate before the IDE
// switch, the CLI switch and its command, the model kinds, Tab and commit
// model choices, and Cursor's place in the topology, dashboard and data map.
export async function testCursorInjection({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = () => page.evaluate(() => window.review.cursorCalls);
  const choose = async (trigger, name) => {
    await trigger.click();
    await page.getByRole("option", { name, exact: true }).click();
    await page.getByRole("option", { name, exact: true }).waitFor({ state: "hidden" });
  };

  await reset("access");
  await page.locator('[data-client-tab="cursor"]').click();
  const panel = page.locator("[data-cursor-access]");
  await panel.waitFor();
  assert.equal(await page.locator('[data-client-tab="cursor"]').getAttribute("aria-selected"), "true");
  await panel.getByText(await tr("cursor.certificate.untrusted"), { exact: true }).waitFor();
  const ide = panel.locator('[data-cursor-switch="ide"]');
  await eventually(async () => assert.equal(await ide.isDisabled(), true, "the IDE waits for a trusted certificate"));
  await panel.getByText(await tr("cursor.trustFirst"), { exact: true }).waitFor();

  await panel.locator('[data-cursor-certificate="trust"]').click();
  await panel.getByText(await tr("cursor.certificate.trusted"), { exact: true }).waitFor();
  await eventually(async () => assert.equal(await ide.isDisabled(), false));
  await ide.click();
  await eventually(async () => assert.equal(await ide.getAttribute("aria-checked"), "true"));
  await panel.getByText(await tr("cursor.status.routed"), { exact: true }).waitFor();
  await panel.getByText("127.0.0.1:47331", { exact: true }).waitFor();
  assert.deepEqual(await calls(), [["trust"], ["ide", true]]);
  console.log("PASS Cursor IDE injection waits for a trusted certificate, then connects through the local port");

  const cli = panel.locator('[data-cursor-switch="cli"]');
  await cli.click();
  await eventually(async () => assert.equal(await cli.getAttribute("aria-checked"), "true"));
  await panel.getByText('$env:CURSOR_API_ENDPOINT="http://127.0.0.1:47331"; cursor-agent', { exact: true }).waitFor();
  await panel.getByRole("button", { name: await tr("cursor.openTerminal") }).click();
  await eventually(async () => assert.deepEqual((await calls()).slice(-2), [["cli", true], ["terminal"]]));
  console.log("PASS cursor-agent gets its endpoint and a connected terminal");

  const sources = panel.locator("[data-cursor-sources]");
  const modelDetails = panel.locator("details[data-cursor-models]");
  assert.equal(await modelDetails.getAttribute("open"), null, "model details start collapsed");
  assert.equal(await panel.locator("[data-cursor-model-list]").isVisible(), false);
  await modelDetails.locator("summary").focus();
  await page.keyboard.press("Enter");
  await panel.locator("[data-cursor-models]").getByText("Harbor Pool", { exact: false }).first().waitFor();
  await sources.locator('[data-model-source="aggregate"]').click();
  await eventually(async () => assert.deepEqual((await calls()).at(-1), ["sources", { relay: true, account: true, aggregate: false }]));
  await eventually(async () => assert.equal(await panel.locator("[data-cursor-models]").getByText("Harbor Pool").count(), 0));
  await modelDetails.locator("summary").focus();
  await page.keyboard.press("Space");
  await eventually(async () => assert.equal(await modelDetails.getAttribute("open"), null));
  console.log("PASS model kinds combine freely and the offered list follows them");

  await choose(panel.locator("#cursor-tab-model"), "Model A · Relay A");
  await eventually(async () => assert.deepEqual((await calls()).at(-1), ["tab", "relay-a/model-A"]));
  await choose(panel.locator("#cursor-commit-model"), "Model A · Relay A");
  await eventually(async () => assert.deepEqual((await calls()).at(-1), ["commit", "relay-a/model-A", "english"]));
  await choose(panel.getByRole("combobox", { name: await tr("cursor.commitLocale") }), await tr("cursor.locale.chinese"));
  await eventually(async () => assert.deepEqual((await calls()).at(-1), ["commit", "relay-a/model-A", "chinese"]));
  await choose(panel.locator("#cursor-tab-model"), await tr("cursor.official"));
  await eventually(async () => assert.deepEqual((await calls()).at(-1), ["tab", null]));
  await panel.getByText(await tr("cursor.webSearchMissing"), { exact: true }).waitFor();
  console.log("PASS Tab and commit messages take a chosen model or stay with Cursor");

  // WebSearch is chosen once under the client console; Cursor links there
  // without leaving its own panel.
  await panel.locator("[data-cursor-web-search]").getByRole("button", { name: await tr("cursor.webSearchSet"), exact: true }).click();
  await eventually(async () => assert.equal(await page.evaluate(() => !!document.activeElement?.closest("[data-client-shared] [data-web-search-settings]")),
    true, "the shared search setting takes focus"));
  assert.equal(await page.locator('[data-client-tab="cursor"]').getAttribute("aria-selected"), "true");
  await panel.waitFor();
  console.log("PASS Cursor's WebSearch hint focuses the search setting shared under the client console");

  // The explanations, certificate details and request path stay folded.
  const notes = panel.locator("details[data-cursor-notes]");
  assert.equal(await notes.getAttribute("open"), null, "notes start collapsed");
  assert.equal(await panel.locator("[data-cursor-pipeline]").isVisible(), false);
  await notes.locator("summary").click();
  await panel.locator("[data-cursor-pipeline]").waitFor();
  await notes.getByText(await tr("cursor.certificate.hint"), { exact: true }).waitFor();
  await notes.locator("summary").click();
  await eventually(async () => assert.equal(await panel.locator("[data-cursor-pipeline]").isVisible(), false));
  console.log("PASS the Cursor panel folds its notes, certificate details and request path");

  const node = page.locator('[data-route-shortcut="cursor"]');
  await eventually(async () => assert.match(await node.innerText(), new RegExp(await tr("newapi.topology.dest.relays"))));
  assert.equal(await page.locator('[data-client-tab="cursor"]').getAttribute("data-link"), "routed");
  if (captureDir) {
    for (const theme of ["light", "dark"]) {
      await page.evaluate(dark => document.documentElement.classList.toggle("dark", dark), theme === "dark");
      await panel.screenshot({ path: path.join(captureDir, `cursor-access-${theme}.png`), animations: "disabled" });
      await notes.locator("summary").click();
      await notes.screenshot({ path: path.join(captureDir, `cursor-notes-${theme}.png`), animations: "disabled" });
      await notes.locator("summary").click();
    }
    await page.evaluate(() => document.documentElement.classList.remove("dark"));
  }

  await panel.locator('[data-cursor-certificate="remove"]').click();
  await panel.getByText(await tr("cursor.certificate.untrusted"), { exact: true }).waitFor();
  await eventually(async () => assert.equal(await ide.getAttribute("aria-checked"), "false", "removing the certificate lets go of the IDE"));
  console.log("PASS the topology shows Cursor routed, and removing the certificate turns the IDE off");

  await page.setViewportSize({ width: 390, height: 844 });
  await panel.scrollIntoViewIfNeeded();
  assert.equal(await panel.evaluate(node => node.scrollWidth <= node.clientWidth), true, "the panel fits a narrow screen");
  await page.setViewportSize({ width: 1200, height: 700 });

  // A page load starts the fixture afresh: seed Cursor with only the CLI on.
  await reset("overview");
  await page.evaluate(() => {
    window.review.keepCursor = true;
    window.review.cursor = { ide: false, cli: true, certificate: "untrusted", sources: { relay: true, account: true, aggregate: true },
      tabModel: null, commitModel: null, commitLocale: "english" };
    window.review.mount("overview");
    window.review.keepCursor = false;
  });
  await page.locator('[data-orbit-link="cursor"]').waitFor({ state: "attached" });
  await eventually(async () => assert.equal(await page.locator('[data-orbit-link="cursor"]').getAttribute("data-state"), "routed",
    "the CLI keeps Cursor connected after the IDE let go"));
  await page.locator(".overview-system-toggle").click();
  const files = page.getByRole("list", { name: await tr("workspace.keyFiles", { name: "Cursor" }) });
  await files.waitFor();
  assert.deepEqual(await files.locator("li").evaluateAll(items => items.map(item => item.getAttribute("data-file"))),
    ["settings", "cliConfig", "certificate", "conversations"]);
  console.log("PASS the dashboard orbit and data map include Cursor");

  // Many models and long provider names must not stretch the default panel.
  await reset("access");
  await page.locator('[data-client-tab="cursor"]').click();
  const defaultHeight = (await panel.locator("details[data-cursor-models]").locator("..").boundingBox()).height;
  await page.evaluate(() => {
    window.review.cursorModels = Array.from({ length: 48 }, (_, index) => ({
      id: `relay-${index % 2}/model-${index}`, cursorId: `ah-test-${index}`,
      label: `Model ${String(index + 1).padStart(2, "0")} with a long context and reasoning variant`,
      sourceName: index % 2 ? "A provider with a very long name for narrow screen layout checks" : "Relay A",
      kind: "relay",
    }));
    window.review.keepCursor = true;
    window.review.mount("access");
    window.review.keepCursor = false;
  });
  await page.locator('[data-client-tab="cursor"]').click();
  const manyDetails = panel.locator("details[data-cursor-models]");
  await manyDetails.getByText(await tr("cursor.modelCount", { count: 48 }), { exact: true }).waitFor();
  assert.equal(await manyDetails.getAttribute("open"), null);
  assert.ok((await manyDetails.locator("..").boundingBox()).height <= defaultHeight + 1,
    "48 models take no more default space than two models");
  assert.equal(await manyDetails.locator("[data-cursor-model]").count(), 48, "every model remains available");
  for (const [device, width, height] of [["desktop", 1200, 700], ["mobile", 390, 844]]) {
    await page.setViewportSize({ width, height });
    for (const theme of ["light", "dark"]) {
      await page.evaluate(dark => document.documentElement.classList.toggle("dark", dark), theme === "dark");
      assert.equal(await panel.evaluate(node => node.scrollWidth <= node.clientWidth), true);
      if (captureDir) await manyDetails.locator("..").screenshot({
        path: path.join(captureDir, `cursor-models-${device}-${theme}-collapsed.png`), animations: "disabled",
      });
      await manyDetails.locator("summary").click();
      const list = panel.locator("[data-cursor-model-list]");
      const bounds = await list.evaluate(node => ({ height: node.clientHeight, content: node.scrollHeight }));
      assert.ok(bounds.height <= 208 && bounds.content > bounds.height, "expanded models scroll within a bounded region");
      await list.locator('[data-cursor-model="relay-1/model-47"]').scrollIntoViewIfNeeded();
      await list.locator('[data-cursor-model="relay-1/model-47"]').waitFor({ state: "visible" });
      assert.equal(await panel.evaluate(node => node.scrollWidth <= node.clientWidth), true);
      await list.evaluate(node => { node.scrollTop = 0; });
      if (captureDir) await manyDetails.locator("..").screenshot({
        path: path.join(captureDir, `cursor-models-${device}-${theme}-expanded.png`), animations: "disabled",
      });
      await manyDetails.locator("summary").click();
    }
  }
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.evaluate(() => window.review.setLanguage("en"));
  assert.equal(await panel.evaluate(node => node.scrollWidth <= node.clientWidth), true, "English labels fit on mobile");
  await manyDetails.getByText("Model details", { exact: true }).waitFor();
  await page.evaluate(() => window.review.setLanguage("zh"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS 48 models stay compact by default, expand completely and scroll on desktop and mobile in both themes");

  await sources.locator('[data-model-source="relay"]').click();
  await panel.getByText(await tr("cursor.noModels"), { exact: true }).waitFor();
  assert.equal(await panel.locator("details[data-cursor-models]").count(), 0);
  console.log("PASS turning off all offered models leaves a clear empty-state action");
}
