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
  await panel.locator("[data-cursor-models]").getByText("Harbor Pool", { exact: false }).first().waitFor();
  await sources.locator('[data-model-source="aggregate"]').click();
  await eventually(async () => assert.deepEqual((await calls()).at(-1), ["sources", { relay: true, account: true, aggregate: false }]));
  await eventually(async () => assert.equal(await panel.locator("[data-cursor-models]").getByText("Harbor Pool").count(), 0));
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

  const node = page.locator('[data-route-shortcut="cursor"]');
  await eventually(async () => assert.match(await node.innerText(), new RegExp(await tr("newapi.topology.dest.relays"))));
  assert.equal(await page.locator('[data-client-tab="cursor"]').getAttribute("data-link"), "routed");
  if (captureDir) {
    for (const theme of ["light", "dark"]) {
      await page.evaluate(dark => document.documentElement.classList.toggle("dark", dark), theme === "dark");
      for (const [part, anchor] of [["top", "#cursor-access-title"], ["bottom", "[data-cursor-pipeline]"]]) {
        await page.locator(anchor).evaluate(node => node.scrollIntoView({ block: "start" }));
        await page.screenshot({ path: path.join(captureDir, `cursor-access-${part}-${theme}.png`), animations: "disabled" });
      }
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
}
