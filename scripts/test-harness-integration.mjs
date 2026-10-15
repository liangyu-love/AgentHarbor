import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testHarnessIntegration({ page, reset, captureDir }) {
  const tr = key => page.evaluate(key => window.review.t(key), key);
  const dialog = () => page.getByRole("dialog");
  const card = () => page.locator("#client-config-harness");
  const seed = async ({ profiles = ["desktop", "web", "dsh-tui"] } = {}) => {
    await reset("compact");
    await page.evaluate(({ profiles }) => {
      const review = window.review;
      review.providerFixtures = [review.getState().providers[0]];
      review.clientStatusFixtures = [{ targetId: "harness", name: "DeepSeek Harness",
        path: "C:/synthetic/.dsh/profiles/desktop/cordis.patch.yml", protocols: ["responses", "chatCompletions", "anthropicMessages"],
        configured: false, conflict: false, conflictFields: [], error: null, providerId: null, model: null, request: null,
        harness: { home: "C:/synthetic/.dsh", credentials: "C:/synthetic/.dsh/.credentials.yaml",
          profiles: profiles.map(name => ({ name, path: `C:/synthetic/.dsh/profiles/${name}/cordis.patch.yml` })) } }];
      review.aggregateModels = [];
      localStorage.setItem("agentharbor.folded-sections.v1", JSON.stringify(Object.fromEntries(
        ["codex-access", "client-configs", "claude-desktop", "aggregation"].map(id => [id, "open"]))));
      review.mount("access");
    }, { profiles });
    await page.getByRole("navigation", { name: await tr("newapi.configShortcuts") }).waitFor();
    await page.locator('[data-client-tab="harness"]').click();
  };
  const configure = () => card().locator("button[data-config-action]");
  const preview = async () => {
    await dialog().getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
    await dialog().locator("pre").waitFor();
    return page.evaluate(() => window.review.previewed);
  };

  // Nothing initialized yet: say where Harness keeps its profiles, write nothing.
  await seed({ profiles: [] });
  await card().locator("[data-harness-no-profile]").waitFor();
  assert.equal(await configure().isDisabled(), true);
  console.log("PASS Harness without an initialized profile explains where profiles live and offers no apply");

  await seed();
  assert.equal(await page.locator('[data-client-tab="harness"] svg path').count(), 1, "the rail shows Harness's own mark");
  await configure().click();
  await dialog().waitFor();
  const profile = dialog().locator("#harness-profile");
  assert.match(await profile.innerText(), /desktop/);
  const defaultModel = dialog().locator("[data-harness-default-model] input");
  assert.equal(await defaultModel.isChecked(), true);
  await dialog().getByRole("button", { name: await tr("clients.accessRouter"), exact: true }).click();
  let request = await preview();
  if (captureDir) await dialog().screenshot({ path: path.join(captureDir, "harness-dialog.png") });
  assert.equal(request.targetId, "harness");
  assert.equal(request.accessMode, "router");
  assert.equal(request.harnessProfile, "desktop");
  assert.equal(request.harnessDefaultModel, true);

  await profile.click();
  await page.getByRole("option", { name: /^web/ }).click();
  await defaultModel.uncheck();
  request = await preview();
  assert.equal(request.harnessProfile, "web");
  assert.equal(request.harnessDefaultModel, false);
  await dialog().getByRole("button", { name: await tr("clients.apply"), exact: true }).click();
  await dialog().waitFor({ state: "detached" });
  const applied = await page.evaluate(() => window.review.appliedClient.request);
  assert.equal(applied.harnessProfile, "web");
  assert.equal(applied.harnessDefaultModel, false);
  await page.waitForFunction(value => window.review.lastToastDescription === value, await tr("clients.harnessReload"));
  console.log("PASS Harness applies to the chosen profile with the default-model switch, then says how Harness reloads");

  // Configured: the profile is fixed until a restore.
  await eventually(async () => assert.equal(await configure().innerText(), await tr("clients.change")));
  if (captureDir) await page.locator("[data-client-console]").screenshot({ path: path.join(captureDir, "harness-console.png") });
  await configure().click();
  await dialog().waitFor();
  assert.equal(await dialog().locator("#harness-profile").isDisabled(), true);
  assert.match(await dialog().locator("#harness-profile").innerText(), /web/);
  await dialog().getByText(await tr("clients.harnessProfileLocked")).waitFor();
  assert.equal(await dialog().locator("[data-harness-default-model] input").isChecked(), false);
  await dialog().getByRole("button", { name: await tr("common.cancel"), exact: true }).click();
  console.log("PASS a configured Harness keeps its profile until it is restored");

  // The card names the profile next to access, source and model.
  const facts = card().locator("[data-client-facts] > div");
  assert.equal(await facts.count(), 4);
  assert.equal(await facts.nth(1).locator("dd").innerText(), "web");
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 800 });
    assert.equal(await card().evaluate(node => node.scrollWidth <= node.clientWidth + 1), true, `the Harness panel fits ${width}px`);
  }
  await page.setViewportSize({ width: 1200, height: 700 });
  if (captureDir) await page.locator("[data-client-console]").screenshot({ path: path.join(captureDir, "harness-console.png") });

  // The routing map draws Harness with its mark, routed to the relays.
  const shortcut = page.locator('[data-route-shortcut="harness"]');
  assert.equal(await shortcut.locator("svg path").count(), 1, "the topology shows Harness's mark");
  await eventually(async () => assert.equal(await page.locator('[data-route="harness-hub"]').getAttribute("data-route-state"), "routed"));
  if (captureDir) await page.locator("[data-topology-layout]").screenshot({ path: path.join(captureDir, "harness-topology.png") });
  console.log("PASS the routing map shows Harness's mark and its routed line into the router");

  // The dashboard: orbit, its ring text and the data map.
  await page.evaluate(() => window.review.mount("overview"));
  const orbit = page.locator("[data-dash-orbit]");
  await orbit.waitFor();
  await eventually(async () => assert.equal(await page.locator('[data-orbit-link="harness"]').getAttribute("data-state"), "routed"));
  assert.match(await orbit.locator("textPath").textContent(), /HARNESS/);
  assert.equal(await orbit.getByTitle(/^Harness ·/).locator("svg path").count(), 1, "the orbit shows Harness's mark");
  if (captureDir) {
    // Past the entrance animation.
    await page.waitForTimeout(1500);
    await orbit.screenshot({ path: path.join(captureDir, "harness-orbit.png") });
  }
  await page.locator(".overview-system-toggle").click();
  const files = page.getByRole("list", { name: await page.evaluate(() => window.review.t("workspace.keyFiles", { name: "DeepSeek Harness" })) });
  await files.waitFor();
  assert.deepEqual(await files.locator("li").evaluateAll(items => items.map(item => item.getAttribute("data-file"))),
    ["profiles", "credentials", "homePatch", "sessions"]);
  const location = page.locator('[data-location="harness"]');
  await location.getByText(await page.evaluate(() => window.review.t("workspace.dataHarnessProfile", { profile: "web" }))).waitFor();
  if (captureDir) await location.screenshot({ path: path.join(captureDir, "harness-data-map.png") });
  console.log("PASS the dashboard orbit, its ring text and the data map include Harness and its profile");
}
