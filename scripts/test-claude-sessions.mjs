import assert from "node:assert/strict";
import path from "node:path";

// Runs only against the deterministic frontend fixture; never reads or alters
// the user's Claude/Codex sessions or launches either real client.
export async function testClaudeSessions({ page, reset, captureDir }) {
  const capture = async name => {
    if (captureDir) await page.screenshot({ path: path.join(captureDir, name), animations: "disabled" });
  };
  const source = name => page.getByRole("radio", { name, exact: true });
  const panel = () => page.getByRole("complementary", { name: "线程详情", exact: true });
  const label = key => page.evaluate(key => window.review.t(key), key);
  const calls = operation => page.evaluate(operation => window.review.conversationCalls.filter(call => call.operation === operation), operation);
  const installNativeMocks = () => page.evaluate(() => {
    window.review.copiedText = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => {
      window.review.copiedText.push(text);
    } } });
    window.__TAURI_INTERNALS__.invoke = async command => {
      if (command === "plugin:dialog|open") return "C:/synthetic/export";
      throw new Error("Unexpected native command: " + command);
    };
  });

  await page.setViewportSize({ width: 1200, height: 820 });
  await reset("thread-tree");
  await page.evaluate(() => {
    document.documentElement.classList.remove("dark");
    // Client source remains part of identity even with the same record key.
    window.review.claudeThreads[0].key = "t0";
  });
  await installNativeMocks();
  await source("Codex").waitFor();
  assert.equal(await source("Codex").getAttribute("aria-checked"), "true");
  assert.equal(await page.getByRole("radio", { name: "Markdown", exact: true }).count(), 0,
    "The header switches clients; export format belongs to the export controls");
  const codexTitle = page.getByRole("button", { name: "查看详情：Review thread 0", exact: true });
  await codexTitle.click();
  await panel().getByText("aimami_relay_2d66f8bc24::gpt-6-astra", { exact: true }).waitFor();
  await panel().getByRole("button", { name: "查看对话", exact: true }).click();
  await panel().getByText("合成的用户问题", { exact: true }).waitFor();
  assert.equal((await calls("details")).at(-1).target, "codex");
  assert.equal((await calls("messages")).at(-1).target, "codex");

  // Switching clients clears the old source's filter, selection and details.
  await page.getByRole("checkbox", { name: "Review thread 0", exact: true }).click();
  await page.getByPlaceholder("搜索标题、工程、会话 ID、提供商或模型…").fill("Review thread 0");
  await source("Codex").focus();
  await page.keyboard.press("ArrowRight");
  const claudeThread = await page.evaluate(() => window.review.claudeThreads[0]);
  const claudeTitle = page.getByRole("button", { name: `查看详情：${claudeThread.title}`, exact: true });
  await claudeTitle.waitFor();
  assert.equal(await source("Claude Code").evaluate(element => element === document.activeElement), true,
    "Keyboard client switching retains focus on the newly selected source");
  assert.equal(await panel().count(), 0, "A Codex panel cannot survive switching to Claude");
  assert.equal(await page.getByText("已选 1 个", { exact: true }).count(), 0);
  assert.equal(await page.getByPlaceholder("搜索标题、工程、会话 ID、提供商或模型…").inputValue(), "");
  assert.equal(await page.getByRole("button", { name: await label("threads.repairAll"), exact: true }).count(), 0);
  assert.equal(await page.getByRole("button", { name: await label("threads.deleteThread"), exact: true }).count(), 1);
  assert.equal(await page.getByRole("button", { name: await label("threads.recoveryAction"), exact: true }).count(), 0);
  assert.equal(await page.getByRole("radio", { name: /^已归档/ }).count(), 0);
  assert.equal(await page.locator('[data-metric="threads"]').evaluate(element => Boolean(element.closest('[role="button"]'))), false,
    "Claude count metrics cannot activate a hidden Codex status filter");
  assert.equal(await page.locator("[data-thread-card]").getByText("claudeCode", { exact: true }).count(), 0,
    "The client name is displayed once without exposing its raw source code");
  await claudeTitle.click();
  await panel().getByText("claude-fixture-model", { exact: true }).waitFor();
  assert.equal((await calls("details")).at(-1).target, "claudeCode",
    "A shared key cannot return Codex detail cache data for Claude");
  await panel().getByRole("button", { name: "查看对话", exact: true }).click();
  await panel().getByText("Claude fixture message", { exact: true }).waitFor();
  assert.equal(await panel().getByText("合成的用户问题", { exact: true }).count(), 0);
  assert.equal((await calls("messages")).at(-1).target, "claudeCode");
  assert.equal(await panel().getByRole("button", { name: await label("threads.deleteThread"), exact: true }).count(), 1);
  await panel().getByRole("button", { name: await label("threads.copyResumeCommand"), exact: true }).click();
  assert.deepEqual(await page.evaluate(() => window.review.copiedText), [`claude --resume ${claudeThread.id}`]);
  await panel().getByRole("button", { name: "关闭", exact: true }).click();

  // Both formats still export, scoped to the client of the clicked action.
  const claudeSelect = page.getByRole("checkbox", { name: claudeThread.title, exact: true });
  for (const format of ["markdown", "jsonl"]) {
    if (format === "jsonl") {
      await page.getByRole("button", { name: await label("threads.exportFormat"), exact: true }).click();
      await page.getByRole("menuitem", { name: await label("threads.formatJsonl"), exact: true }).click();
    }
    await claudeSelect.click();
    assert.equal(await page.getByRole("button", { name: /^删除所选/ }).count(), 1);
    await page.getByRole("button", { name: /^导出所选/ }).click();
    await page.waitForFunction(count => window.review.conversationCalls.filter(call => call.operation === "export").length === count,
      format === "markdown" ? 1 : 2);
    const exported = (await calls("export")).at(-1);
    assert.equal(exported.target, "claudeCode");
    assert.equal(exported.format, format);
    assert.deepEqual(exported.keys, [claudeThread.key]);
    await page.getByText("已选 1 个", { exact: true }).waitFor({ state: "hidden" });
  }
  await capture("claude-sessions-desktop-light.png");
  await page.setViewportSize({ width: 390, height: 820 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(await page.locator("[data-page-scroll-container]").evaluate(element => element.scrollWidth <= element.clientWidth), true,
    "Client controls and Claude cards stay inside the mobile viewport");
  await capture("claude-sessions-mobile-dark.png");
  await claudeTitle.scrollIntoViewIfNeeded();
  assert.equal(await page.locator("[data-thread-card]").first().evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await capture("claude-sessions-card-mobile-dark.png");
  await claudeTitle.click();
  await panel().getByRole("button", { name: "查看对话", exact: true }).click();
  await panel().getByText("Claude fixture message", { exact: true }).waitFor();
  assert.equal(await panel().evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await capture("claude-sessions-detail-mobile-dark.png");
  await panel().getByRole("button", { name: "关闭", exact: true }).click();
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 820 });
  await source("Codex").click();
  await codexTitle.waitFor();
  assert.equal(await source("Codex").getAttribute("aria-checked"), "true");
  assert.equal(await page.getByRole("button", { name: await label("threads.repairAll"), exact: true }).isVisible(), true);
  await capture("codex-sessions-desktop-light.png");

  // A delayed old-source response must never replace the active client's list.
  await reset("thread-tree");
  await codexTitle.waitFor();
  await page.evaluate(() => { window.review.holdConversationLists = ["claudeCode"]; });
  await source("Claude Code").click();
  await page.waitForFunction(() => window.review.conversationListResolvers.length === 1);
  await source("Codex").click();
  await codexTitle.waitFor();
  await page.evaluate(() => {
    window.review.holdConversationLists = [];
    window.review.conversationListResolvers.shift().resolve();
  });
  await page.waitForFunction(() => window.review.conversationCalls.some(call => call.operation === "listResolved" && call.target === "claudeCode"));
  assert.equal(await source("Codex").getAttribute("aria-checked"), "true");
  assert.equal(await claudeTitle.count(), 0, "Late Claude results cannot replace an active Codex workspace");
  await source("Claude Code").click();
  await claudeTitle.waitFor();
  assert.equal(await page.evaluate(() => localStorage.getItem("threads_source")), "claudeCode");
  // Native folder selection can finish after the user changes clients. Export
  // must keep the source captured by the original clicked card.
  await page.evaluate(() => {
    window.__TAURI_INTERNALS__.invoke = async command => {
      if (command !== "plugin:dialog|open") throw new Error("Unexpected native command: " + command);
      return new Promise(resolve => { window.review.resolveExportFolder = () => resolve("C:/synthetic/export"); });
    };
  });
  await page.getByRole("button", { name: await label("threads.exportThread"), exact: true }).click();
  await page.waitForFunction(() => typeof window.review.resolveExportFolder === "function");
  await source("Codex").click();
  await codexTitle.waitFor();
  await page.evaluate(() => window.review.resolveExportFolder());
  await page.waitForFunction(() => window.review.conversationCalls.some(call => call.operation === "export"));
  assert.equal((await calls("export")).at(-1).target, "claudeCode",
    "An old Claude export cannot silently change its target to the new Codex client");
  assert.equal(await source("Codex").getAttribute("aria-checked"), "true");
  await reset("thread-tree");
  await source("Claude Code").click();
  const deleting = await page.evaluate(() => ({ ...window.review.claudeThreads[0] }));
  await page.getByRole("checkbox", { name: deleting.title, exact: true }).click();
  await page.getByRole("button", { name: /^删除所选/ }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await label("common.cancel"), exact: true }).click();
  assert.equal((await calls("delete")).length, 0, "Cancel never deletes a session");
  await page.getByRole("button", { name: /^删除所选/ }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: await label("common.delete"), exact: true }).click();
  await page.waitForFunction(() => window.review.conversationCalls.some(call => call.operation === "delete"));
  assert.equal((await calls("delete")).at(-1).target, "claudeCode");
  assert.deepEqual((await calls("delete")).at(-1).keys, [deleting.key]);
  await page.getByRole("checkbox", { name: deleting.title, exact: true }).waitFor({ state: "hidden" });
  await source("Codex").click();
  await codexTitle.waitFor();
  console.log("PASS Claude/Codex scoped lists, detail/message caches, reset, resume, exports, late results and responsive UI");
}
