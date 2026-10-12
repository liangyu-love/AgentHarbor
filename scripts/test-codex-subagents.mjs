import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testCodexSubagents({ page, reset, captureDir }) {
  await reset("access");
  const fields = () => page.locator('[aria-labelledby="codex-subagents-title"]');
  const select = async (id, name) => {
    await page.locator(id).click();
    await page.getByRole("option", { name, exact: true }).click();
  };
  await fields().waitFor();
  await select("#codex-subagent-model", "Relay / responses-model");
  await select("#codex-subagent-effort", "xhigh");
  await fields().getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.subagentSaves?.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.subagentSaves[0]),
    { model: "mixed::responses-model", reasoningEffort: "xhigh" });
  await fields().getByText("待应用", { exact: true }).waitFor();
  await page.evaluate(() => window.review.mount("access"));
  await fields().getByRole("combobox").first().getByText("Relay / responses-model").waitFor();
  await select("#codex-subagent-model", "继承主模型");
  await select("#codex-subagent-effort", "继承 / 模型默认");
  await fields().getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.subagentSaves?.length === 2);
  assert.deepEqual(await page.evaluate(() => window.review.subagentSaves[1]), { model: null, reasoningEffort: null });
  await fields().getByRole("button", { name: "恢复原配置", exact: true }).click();
  await page.waitForFunction(() => window.review.subagentSaves?.length === 3);
  assert.equal(await page.evaluate(() => window.review.subagentSaves[2]), null);
  await page.waitForFunction(() => window.review.lastToast === "已恢复子代理原配置");
  await eventually(async () => assert.equal(await fields().getByRole("button", { name: "恢复原配置", exact: true }).isDisabled(), true));
  console.log("PASS Codex subagent save, reopen, inherit and restore original");

  for (const [mode, expected, absent] of [
    ["native", "responses-model", "Official Luna"], ["aggregate", "Luna Pool", "Relay / responses-model"],
  ]) {
    await reset("access");
    await page.evaluate(mode => {
      window.review.routerPatch = mode === "native"
        ? { accessMode: "native" } : { aggregateModelId: "pool" };
      window.review.mount("access");
    }, mode);
    await fields().waitFor();
    await page.locator("#codex-subagent-model").click();
    await page.getByRole("option", { name: expected, exact: true }).waitFor();
    await eventually(async () => assert.equal(await page.getByRole("option", { name: absent, exact: true }).count(), 0));
    await page.keyboard.press("Escape");
  }
  console.log("PASS child model choices respect Native and aggregation namespaces");

  await reset("access");
  await page.evaluate(() => {
    window.review.originalSubagents = { model: "removed::child", reasoningEffort: "xhigh" };
    window.review.mount("access");
  });
  await fields().getByRole("alert").waitFor();
  await select("#codex-subagent-model", "Official Luna");
  await page.evaluate(() => { window.review.failSubagentSave = true; });
  await fields().getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.lastToast);
  await eventually(async () => assert.equal(await page.locator("#codex-subagent-model").textContent(), "Official Luna"));
  await page.evaluate(() => { window.review.failSubagentSave = false; window.review.holdSubagentSave = true; });
  await fields().getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.releaseSubagentSave);
  await eventually(async () => assert.equal(await fields().getByRole("combobox").first().isDisabled(), true));
  await eventually(async () => assert.equal(await fields().getByRole("button", { name: "保存中…", exact: true }).isDisabled(), true));
  await page.evaluate(() => window.review.releaseSubagentSave());
  await fields().getByText("待应用", { exact: true }).waitFor();
  console.log("PASS invalid saved target, failed save retention and shared operation lock");

  await reset("access");
  await page.evaluate(() => { window.review.failSubagentLoad = true; window.review.mount("access"); });
  await page.getByRole("button", { name: "重试", exact: true }).waitFor();
  await page.evaluate(() => { window.review.failSubagentLoad = false; });
  await page.getByRole("button", { name: "重试", exact: true }).click();
  await fields().waitFor();
  await reset("access");
  await page.evaluate(() => {
    window.review.originalSubagents = { model: "gpt-6-luna", reasoningEffort: "high" };
    window.review.mount("access");
  });
  await fields().getByRole("combobox").first().getByText("Official Luna").waitFor();
  await eventually(async () => assert.equal(await fields().getByRole("button", { name: "保存", exact: true }).isEnabled(), true));
  await fields().getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.review.subagentSaves?.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.subagentSaves[0]),
    { model: "gpt-6-luna", reasoningEffort: "high" });
  await fields().getByText("待应用", { exact: true }).waitFor();
  console.log("PASS pre-existing user defaults can be saved without changing their values");
  if (captureDir) {
    await reset("access");
    await page.evaluate(() => {
      window.review.subagentModels = [{ id: "long-child", label: "Relay / vendor/deepseek-v4.1-flash-custom-long-channel-name-regression-model",
        reasoningEfforts: ["high", "xhigh"] }];
      window.review.originalSubagents = { model: "long-child", reasoningEffort: "xhigh" };
      window.review.mount("access");
    });
    for (const [name, width, dark] of [["desktop-light", 1200, false], ["desktop-dark", 1200, true],
      ["narrow-light", 420, false], ["narrow-dark", 420, true]]) {
      await page.setViewportSize({ width, height: 900 });
      await page.evaluate(dark => document.documentElement.classList.toggle("dark", dark), dark);
      await fields().scrollIntoViewIfNeeded();
      assert.equal(await fields().evaluate(el => el.scrollWidth <= el.clientWidth), true);
      await fields().screenshot({ path: path.join(captureDir, `codex-subagents-${name}.png`) });
    }
    await page.setViewportSize({ width: 1200, height: 700 });
  }
  console.log("PASS child-default load error and retry; long-name layout checks");
}
