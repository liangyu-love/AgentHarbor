import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// Synthetic client statuses and terminal proxy states only; nothing is written.
export async function testDirectProxy({ page, reset, captureDir }) {
  const t = (key, values) => page.evaluate(([key, values]) => window.review.t(key, values), [key, values]);
  const dialog = () => page.getByRole("dialog");
  const note = () => dialog().locator("[data-direct-proxy]");
  const open = async (targetId, options = {}) => {
    await reset("compact");
    await page.evaluate(({ options }) => {
      const review = window.review;
      const base = review.getState().providers[0];
      review.providerFixtures = [{ ...base, networkMode: options.cardDirect ? "direct" : "proxy" }];
      const client = (targetId, name) => ({ targetId, name, path: `C:/synthetic/${targetId}/settings.json`,
        protocols: targetId === "claudeCode" ? ["anthropicMessages"] : ["responses", "chatCompletions", "anthropicMessages"],
        configured: false, conflict: false, conflictFields: [], error: null, providerId: null, model: null, request: null });
      review.clientStatusFixtures = [client("claudeCode", "Claude Code"), client("opencode", "OpenCode"), client("pi", "Pi")];
      review.terminalStatusPatch = options.terminal;
      localStorage.setItem("agentharbor.folded-sections.v1", JSON.stringify(Object.fromEntries(
        ["codex-access", "client-configs", "claude-desktop", "aggregation"].map(id => [id, "open"]))));
      review.mount("access");
    }, { options });
    await page.locator(`[data-client-tab="${targetId}"]`).click();
    await page.locator(`#client-config-${targetId} button[data-config-action]`).click();
    await dialog().waitFor();
    // The terminal proxy state arrives after the dialog opens.
    await note().locator("p").nth(1).waitFor();
  };
  const state = () => note().getAttribute("data-direct-proxy");
  const text = () => note().locator("p").nth(1).innerText();

  await open("claudeCode");
  assert.equal(await dialog().getByRole("button", { name: await t("clients.accessNative"), exact: true }).getAttribute("aria-pressed"), "true");
  assert.equal(await state(), "claudeOff");
  assert.equal(await note().locator("p").first().innerText(), await t("clients.directProxy.title"));
  assert.equal(await text(), await t("clients.directProxy.claudeOff"));
  if (captureDir) await dialog().screenshot({ path: path.join(captureDir, "direct-proxy-claude.png"), animations: "disabled" });
  await page.evaluate(() => {
    window.review.terminalProxyOpened = 0;
    window.addEventListener("agentharbor:open-terminal-proxy", () => { window.review.terminalProxyOpened++; });
  });
  await note().getByRole("button", { name: await t("clients.directProxy.open"), exact: true }).click();
  assert.equal(await page.evaluate(() => window.review.terminalProxyOpened), 1);
  await dialog().getByRole("button", { name: await t("clients.accessRouter"), exact: true }).click();
  await eventually(async () => assert.equal(await note().count(), 0, "the router goes through the app proxy itself"));
  console.log("PASS Claude Code direct mode explains the proxy it gets and links to the terminal proxy");

  await open("claudeCode", { terminal: { enabled: true, applied: true, url: "http://127.0.0.1:7890/" } });
  assert.equal(await state(), "terminalOn");
  assert.equal(await text(), await t("clients.directProxy.terminalOn", { name: "Claude Code", url: "http://127.0.0.1:7890/" }));
  await open("claudeCode", { cardDirect: true });
  assert.equal(await state(), "claudeCardDirect");
  await open("claudeCode", { terminal: { supported: false } });
  assert.equal(await state(), "claudeUnsupported");
  assert.equal(await note().getByRole("button").count(), 0, "no terminal proxy to set up off Windows");
  console.log("PASS the note follows the terminal proxy, the card's network setting and the platform");

  await open("opencode");
  assert.equal(await state(), "otherOff");
  assert.equal(await text(), await t("clients.directProxy.otherOff", { name: "OpenCode" }));
  await open("pi", { terminal: { supported: false } });
  assert.equal(await state(), "otherUnsupported");
  assert.equal(await text(), await t("clients.directProxy.otherUnsupported", { name: "Pi" }));
  console.log("PASS OpenCode and Pi direct mode point at the terminal proxy or the router");

  // The link lands on the terminal proxy row, scrolled into view and highlighted.
  await reset("compact");
  await page.evaluate(() => { window.review.terminalFocusRequested = true; window.review.mount("terminal-proxy"); });
  await page.locator("[data-terminal-proxy-highlighted]").waitFor();
  assert.equal(await page.evaluate(() => window.review.terminalFocusHandled), 1);
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("aria-label")), await t("settings.terminalProxy.edit"));
  assert.equal(await page.locator("[data-terminal-proxy-direct]").innerText(), await t("settings.terminalProxy.directClients"));
  await page.locator("[data-terminal-proxy-highlighted]").waitFor({ state: "detached", timeout: 4000 });
  console.log("PASS the terminal proxy row takes focus once, says it serves direct clients and fades its highlight");
}
