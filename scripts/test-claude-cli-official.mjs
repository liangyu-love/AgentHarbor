import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// Synthetic client statuses and IPC stubs only; no credentials or CLI files.
export async function testClaudeCliOfficial({ page, reset, captureDir }) {
  const tr = (key) => page.evaluate((value) => window.review.t(value), key);
  const clients = () => page.locator('section[id^="client-config-"]');
  const card = (name) => clients().filter({ has: page.getByRole("heading", { name, exact: true }) });
  const dialog = () => page.getByRole("alertdialog");
  const seed = async (options = {}) => {
    await reset("compact");
    await page.evaluate((options) => {
      const review = window.review;
      const provider = review.getState().providers[0];
      review.providerFixtures = options.noProviders ? [] : [provider];
      review.clientStatusFixtures = [
        { targetId: "claudeCode", name: "Claude Code", path: "C:/synthetic/.claude/settings.json",
          protocols: ["anthropicMessages"], configured: !!options.managed, conflict: !!options.conflict, stale: !!options.stale,
          conflictFields: options.conflict ? ["model"] : [], error: null,
          providerId: options.managed ? provider.id : null, model: options.managed ? provider.model : null,
          request: options.managed ? { targetId: "claudeCode", providerId: provider.id, model: provider.model,
            accessMode: "router", providerIds: [provider.id], authField: "authToken",
            claudeModels: { sonnet: provider.model, opus: "", fable: "", haiku: "", subagent: "" } } : null },
        { targetId: "opencode", name: "OpenCode", path: "C:/synthetic/opencode.json",
          protocols: ["responses", "chatCompletions", "anthropicMessages"], configured: false,
          conflict: false, error: null, providerId: null, model: null, request: null },
      ];
      Object.assign(review, options);
      review.mount("clients-existing");
    }, options);
    await card("Claude Code").getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).waitFor();
  };
  const open = async () => {
    await card("Claude Code").getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).click();
    await dialog().waitFor();
  };
  const confirm = async () => dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).click();
  const cancel = async () => dialog().getByRole("button", { name: await tr("common.cancel"), exact: true }).click();

  await seed({ noProviders: true });
  assert.equal(await card("Claude Code").getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).isEnabled(), true);
  assert.equal(await card("OpenCode").getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).count(), 0);
  await open();
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  assert.equal(await dialog().getByText("env.ANTHROPIC_AUTH_TOKEN", { exact: true }).count(), 1);
  assert.equal(await dialog().getByText("C:/synthetic/.claude/settings.json", { exact: true }).count(), 1);
  assert.equal(await dialog().locator("pre").count(), 0, "the preview exposes names, never configuration contents");
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), []);
  await cancel();
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), []);
  console.log("PASS Claude Code official restore is available without management or relays, excludes OpenCode and previews without writing");

  await seed({ managed: true, conflict: true, officialApplyHold: true });
  const profiles = '[{"id":"keep","name":"Saved relay preset","request":{"targetId":"claudeCode","providerId":"relay","model":"claude-test"}}]';
  await page.evaluate(value => localStorage.setItem("agentharbor.claude-code-profiles.v1", value), profiles);
  assert.equal(await card("Claude Code").getByRole("button", { name: await tr("clients.restore"), exact: true }).count(), 1,
    "restore original configuration remains a separate action");
  await open();
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  await dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).evaluate(button => { button.click(); button.click(); });
  await page.waitForFunction(() => window.review.officialSwitchCalls.length === 1);
  await eventually(async () => assert.equal(await dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).isDisabled(), true));
  await eventually(async () => assert.equal(await dialog().getByRole("button", { name: await tr("common.cancel"), exact: true }).isDisabled(), true));
  await page.keyboard.press("Escape");
  assert.equal(await dialog().count(), 1, "a writing operation cannot be dismissed");
  await page.evaluate(() => window.review.officialApplyResolve());
  await dialog().waitFor({ state: "detached" });
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), ["official-1"]);
  assert.equal(await card("Claude Code").getByText(await tr("clients.notConfigured"), { exact: true }).count(), 1);
  assert.equal(await card("Claude Code").getByRole("button", { name: await tr("clients.restore"), exact: true }).count(), 0);
  assert.equal(await card("OpenCode").getByText(await tr("clients.notConfigured"), { exact: true }).count(), 1);
  assert.equal(await page.evaluate(() => localStorage.getItem("agentharbor.claude-code-profiles.v1")), profiles);
  console.log("PASS confirmed official restore uses one revision, locks writes and refreshes client status without claiming a login");

  await seed({ officialPreviewHold: true });
  await open();
  await page.waitForFunction(() => window.review.officialPreviewCalls.length === 1);
  await eventually(async () => assert.equal(await dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).isDisabled(), true));
  await cancel();
  await dialog().waitFor({ state: "detached" });
  await open();
  await page.waitForFunction(() => window.review.officialPreviewCalls.length === 2);
  await page.evaluate(() => window.review.officialPreviewResolvers[0]());
  assert.equal(await dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).isDisabled(), true,
    "the cancelled preview must not unlock its replacement");
  assert.equal(await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).count(), 0);
  await page.evaluate(() => { window.review.officialPreviewHold = false; window.review.officialPreviewResolvers[1](); });
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  await confirm();
  await dialog().waitFor({ state: "detached" });
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), ["official-2"]);
  console.log("PASS cancelling a pending official preview releases the lock and ignores its late response");

  await seed({ officialStale: true });
  await open();
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  await confirm();
  await dialog().getByRole("alert").getByText(await tr("clients.errors.changed"), { exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), ["official-1"]);
  assert.equal(await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).count(), 0);
  await dialog().getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), ["official-1"],
    "a fresh preview never automatically applies a stale request");
  await confirm();
  await dialog().waitFor({ state: "detached" });
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), ["official-1", "official-2"]);
  console.log("PASS stale official revisions require a fresh preview and another explicit confirmation");

  await seed({ officialPreviewFailure: true });
  await open();
  await dialog().getByRole("alert").getByText(await tr("clients.errors.read"), { exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), []);
  await page.evaluate(() => { window.review.officialPreviewFailure = false; window.review.officialApplyFailure = true; });
  await dialog().getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  await confirm();
  await dialog().getByRole("alert").getByText(await tr("clients.errors.storage"), { exact: true }).waitFor();
  assert.equal(await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).count(), 0);
  await cancel();
  console.log("PASS official preview and write failures remain visible and discard invalidated previews");

  await seed({ officialNoChanges: true });
  await open();
  await dialog().getByText(await tr("clients.officialNoChanges"), { exact: true }).waitFor();
  assert.equal(await dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).isDisabled(), true);
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), []);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "claude-cli-official-unchanged.png"), fullPage: true });
  await cancel();

  await seed({ officialSessionOnly: true, officialWarnings: ["clients.warnings.officialManagedSettings"] });
  await open();
  await dialog().getByText("C:/synthetic/.claude/settings.json", { exact: true }).waitFor();
  assert.equal(await dialog().getByText(await tr("clients.officialRemovedFields"), { exact: true }).count(), 0);
  assert.equal(await dialog().getByRole("button", { name: await tr("clients.switchOfficial"), exact: true }).isEnabled(), true,
    "management cleanup may need a write even when no settings fields remain");
  assert.equal(await dialog().getByText(await tr("clients.warnings.officialManagedSettings"), { exact: true }).count(), 1);
  await confirm();
  await dialog().waitFor({ state: "detached" });
  assert.deepEqual(await page.evaluate(() => window.review.officialSwitchCalls), ["official-1"]);
  console.log("PASS unchanged official settings avoid writes while session-only cleanup and conditional warnings remain supported");

  await seed({ managed: true });
  await open();
  await dialog().getByText("env.ANTHROPIC_BASE_URL", { exact: true }).waitFor();
  if (captureDir) await dialog().screenshot({ path: path.join(captureDir, "claude-cli-official-desktop-light.png"), animations: "disabled" });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(await dialog().evaluate(element => element.scrollWidth <= element.clientWidth), true);
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), true);
  if (captureDir) await dialog().screenshot({ path: path.join(captureDir, "claude-cli-official-mobile-dark.png"), animations: "disabled" });
  await cancel();
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS official confirmation stays usable in desktop light and narrow dark themes");

  // A direct configuration whose relay card changed says so and offers to reapply it.
  await seed({ managed: true });
  assert.equal(await card("Claude Code").locator("[data-client-stale]").count(), 0);
  await seed({ managed: true, stale: true });
  const stale = card("Claude Code").locator("[data-client-stale]");
  await stale.getByText(await tr("clients.stale")).waitFor();
  await stale.getByRole("button", { name: await tr("clients.reapply"), exact: true }).click();
  await dialog().getByRole("button", { name: await tr("clients.reapply"), exact: true }).click();
  await page.waitForFunction(() => window.review.reappliedClient?.rebaseline === false);
  await stale.waitFor({ state: "detached" });
  console.log("PASS a direct configuration whose relay card changed says so and reapplies from the notice");
}
