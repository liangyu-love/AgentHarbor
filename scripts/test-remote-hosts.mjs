import assert from "node:assert/strict";
import path from "node:path";
import { showInjectTab } from "./inject-tabs.mjs";

const FINGERPRINT = "SHA256:fixtureHostKey0123456789abcdefghijklmnopqrstu";

// Remote hosts: adding a host (a password host refuses an empty password),
// trusting a new host key after seeing its fingerprint, configuring Codex and
// Claude Code on it direct only, restoring, and editing without retyping the
// stored secret.
export async function testRemoteHosts({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = name => page.evaluate(name => window.review.remoteCalls.filter(call => call[0] === name), name);

  await reset("access");
  await showInjectTab(page, "remote");
  const panel = page.locator("[data-remote-hosts]");
  const empty = panel.locator("[data-remote-empty]");
  await empty.waitFor();

  await empty.getByRole("button", { name: await tr("remote.add") }).click();
  const dialog = page.locator("[data-remote-host-dialog]");
  await dialog.waitFor();
  const password = dialog.getByLabel(await tr("remote.fields.password"), { exact: true });
  await dialog.getByLabel(await tr("remote.fields.host"), { exact: true }).fill("dev");
  await dialog.getByLabel(await tr("remote.fields.user"), { exact: true }).fill("ubuntu");
  await dialog.getByRole("button", { name: await tr("remote.authPassword") }).click();
  await dialog.getByRole("button", { name: await tr("remote.save") }).click();
  await dialog.getByRole("alert").getByText(await tr("remote.errors.passwordMissing")).waitFor();
  await password.fill("s3cret");
  await dialog.getByRole("button", { name: await tr("remote.save") }).click();
  await dialog.waitFor({ state: "hidden" });
  const added = (await calls("save")).at(-1)[1];
  assert.equal(added.password, "s3cret");
  assert.equal(added.name, "dev", "an empty name follows the host");
  await panel.locator('[data-remote-host="host-1"]').getByText("ubuntu@10.0.0.5").waitFor();
  console.log("PASS a remote host is added with its password, and a password host refuses an empty one");

  // A new host shows its key's fingerprint and connects only once it is trusted.
  const hostKey = panel.locator('[data-remote-host-key="info"]');
  await hostKey.getByText(FINGERPRINT).waitFor();
  assert.equal(await panel.locator("[data-remote-clients]").count(), 0, "nothing is read before the host is trusted");
  await hostKey.getByRole("button", { name: await tr("remote.hostKey.trust") }).click();
  const codex = panel.locator('[data-remote-client="codex"]');
  const claude = panel.locator('[data-remote-client="claudeCode"]');
  await codex.getByText(await tr("remote.clients.notConfigured")).waitFor();
  assert.deepEqual((await calls("test")).at(-1), ["test", "host-1", FINGERPRINT]);
  console.log("PASS a new host's key fingerprint is shown and trusted before its clients are read");

  // Codex: one relay's Responses models, previewed, then written.
  await codex.getByRole("button", { name: await tr("remote.clients.configure") }).click();
  const codexDialog = page.locator("[data-remote-codex-dialog]");
  await codexDialog.waitFor();
  await codexDialog.locator("#remote-codex-model").click();
  assert.deepEqual(await page.getByRole("option").allInnerTexts(), ["responses-model"], "only Responses models reach Codex");
  await page.getByRole("option", { name: "responses-model" }).click();
  await codexDialog.getByRole("button", { name: await tr("clients.preview") }).click();
  await codexDialog.locator("[data-remote-codex-preview] pre").getByText('model = "responses-model"', { exact: false }).waitFor();
  await codexDialog.getByRole("button", { name: await tr("clients.apply") }).click();
  await codexDialog.waitFor({ state: "hidden" });
  assert.deepEqual((await calls("applyCodex")).at(-1),
    ["applyCodex", "host-1", { providerId: "mixed", model: "responses-model" }, "rev-codex", true]);
  await codex.getByText(await tr("remote.clients.configured", { provider: "Provider mixed", model: "responses-model" })).waitFor();
  assert.equal(await panel.getByRole("button", { name: await tr("remote.delete") }).isDisabled(), true,
    "a host with configured clients cannot be deleted");
  console.log("PASS Codex on a remote host is configured direct with one relay's Responses models");

  // Claude Code reuses the client dialog, direct only.
  await claude.getByRole("button", { name: await tr("remote.clients.configure") }).click();
  const claudeDialog = page.locator("[data-client-config-dialog]");
  await claudeDialog.getByRole("heading", { name: await tr("remote.configureTitle", { name: "Claude Code", host: "dev" }) }).waitFor();
  assert.equal(await claudeDialog.locator(`[aria-label="${await tr("clients.accessMode")}"]`).count(), 0, "a remote Claude Code has no router mode");
  await claudeDialog.getByRole("button", { name: await tr("clients.preview") }).click();
  await claudeDialog.locator("[data-client-config-preview]").waitFor();
  const previewed = (await calls("previewClaude")).at(-1)[2];
  assert.equal(previewed.accessMode, "native");
  assert.equal(previewed.providerId, "mixed");
  assert.equal(previewed.model, "anthropic-model");
  await claudeDialog.getByRole("button", { name: await tr("clients.apply") }).click();
  await claudeDialog.waitFor({ state: "hidden" });
  await claude.getByText(await tr("remote.clients.configured", { provider: "Provider mixed", model: "anthropic-model" })).waitFor();
  if (captureDir) await panel.screenshot({ path: path.join(captureDir, "remote-hosts.png") });
  console.log("PASS Claude Code on a remote host is configured through the client dialog without router mode");

  await codex.getByRole("button", { name: await tr("remote.clients.restore") }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await tr("remote.clients.restore") }).click();
  await codex.getByText(await tr("remote.clients.notConfigured")).waitFor();
  assert.deepEqual((await calls("restoreCodex")).at(-1), ["restoreCodex", "host-1", false]);

  await panel.getByRole("button", { name: await tr("remote.edit") }).click();
  await dialog.waitFor();
  assert.equal(await password.getAttribute("placeholder"), await tr("remote.fields.storedSecret"));
  assert.equal(await password.inputValue(), "", "a stored password is never shown");
  await dialog.getByRole("button", { name: await tr("remote.save") }).click();
  await dialog.waitFor({ state: "hidden" });
  const edited = (await calls("save")).at(-1)[1];
  assert.equal(edited.id, "host-1");
  assert.equal(edited.password, "", "an untouched password field keeps the stored one");
  console.log("PASS a restore asks first, and editing a host keeps its stored password without showing it");
}
