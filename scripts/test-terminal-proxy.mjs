import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testTerminalProxy({ page, reset, captureDir }) {
  const mount = async (options = {}) => {
    await reset("compact");
    await page.evaluate(value => { Object.assign(window.review, value); window.review.mount("terminal-proxy"); }, options);
    await page.locator('[data-terminal-proxy-state]:not([data-terminal-proxy-state="loading"])').waitFor();
  };
  const state = value => page.locator(`[data-terminal-proxy-state="${value}"]`);
  const dialog = () => page.getByRole("dialog");
  const url = () => dialog().getByLabel("代理地址", { exact: true });
  const open = async () => {
    await page.getByRole("button", { name: "编辑终端代理", exact: true }).click();
    await url().waitFor();
  };
  const save = () => dialog().getByRole("button", { name: "保存并启用", exact: true });
  const test = () => dialog().getByRole("button", { name: "测试连接", exact: true });
  const edit = () => page.getByRole("button", { name: "编辑终端代理", exact: true });
  const active = { enabled: true, applied: true, url: "http://127.0.0.1:7890" };
  const probes = [{ service: "Anthropic", reachable: true, statusCode: 404 }, { service: "OpenAI", reachable: true, statusCode: 401 }];

  await mount();
  await state("disabled").waitFor();
  assert.equal(await page.getByRole("button", { name: "打开 PowerShell" }).count(), 0);
  await open();
  assert.equal(await save().isEnabled(), false);
  await dialog().getByRole("button", { name: "使用软件代理地址" }).click();
  await eventually(async () => assert.equal(await url().inputValue(), "http://127.0.0.1:7890"));
  await dialog().getByLabel("直连地址", { exact: true }).fill("example.internal,localhost,127.0.0.1,::1");
  await test().click();
  await dialog().getByText("可连接 · HTTP 401", { exact: true }).waitFor();
  await save().click();
  await dialog().waitFor({ state: "detached" });
  await state("enabled").waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.terminalSetCalls), [{ enabled: true, url: "http://127.0.0.1:7890", noProxy: "example.internal,localhost,127.0.0.1,::1" }]);
  await page.getByRole("button", { name: "打开 PowerShell", exact: true }).click();
  await page.waitForFunction(() => window.review.terminalOpenCalls === 1);
  await page.getByRole("button", { name: "关闭并恢复", exact: true }).click();
  await state("disabled").waitFor();
  assert.equal(await page.evaluate(() => window.review.terminalSetCalls.at(-1).enabled), false);
  console.log("PASS terminal proxy loads, reuses an HTTP app proxy, tests two services, saves, launches PowerShell and restores independently");

  await mount({ terminalTestHold: true });
  await open();
  await url().fill("http://127.0.0.1:7890");
  await test().click();
  await page.waitForFunction(() => window.review.terminalTestResolvers.length === 1);
  await url().fill("http://127.0.0.1:7891");
  await test().click();
  await page.waitForFunction(() => window.review.terminalTestResolvers.length === 2);
  await page.evaluate(() => window.review.terminalTestResolvers[0]([{ service: "stale", reachable: false, statusCode: null }]));
  await eventually(async () => assert.equal(await dialog().getByText("stale", { exact: true }).count(), 0));
  assert.equal(await dialog().getByRole("button", { name: "测试中", exact: true }).isDisabled(), true);
  await page.evaluate(value => window.review.terminalTestResolvers[1](value), probes);
  await dialog().getByText("可连接 · HTTP 401", { exact: true }).waitFor();
  await url().fill("http://127.0.0.1:7892");
  await test().click();
  await page.waitForFunction(() => window.review.terminalTestResolvers.length === 3);
  await dialog().getByRole("button", { name: "取消", exact: true }).click();
  await open();
  await page.evaluate(value => window.review.terminalTestResolvers[2](value), probes);
  assert.equal(await dialog().locator("[data-terminal-proxy-probes]").count(), 0);
  console.log("PASS terminal proxy ignores stale tests after draft edits and close/reopen without ending a newer test");

  for (const [patch, expected] of [[{ ...active, conflicts: ["HTTP_PROXY"] }, "changed"], [{ ...active, applied: false, recoveryRequired: true }, "recoveryRequired"]]) {
    await mount({ terminalStatusPatch: patch });
    await state(expected).waitFor();
    assert.equal(await page.getByRole("button", { name: "打开 PowerShell", exact: true }).isDisabled(), true);
    await open();
    assert.equal(await save().count(), 0);
    // Restoring remains possible with an empty or invalid address.
    await url().fill("");
    await dialog().getByRole("button", { name: "关闭并恢复", exact: true }).click();
    await state("disabled").waitFor();
    assert.equal(await page.evaluate(() => window.review.terminalSetCalls.at(-1).enabled), false);
  }
  console.log("PASS external edits and interrupted operations expose restoration and block launch or reapply until recovery");

  await mount({ terminalSaveHold: true });
  await open();
  await url().fill("http://127.0.0.1:7890");
  await save().click();
  await page.waitForFunction(() => window.review.terminalSetCalls.length === 1);
  await page.keyboard.press("Escape");
  await eventually(async () => assert.equal(await dialog().count(), 1));
  assert.equal(await url().isDisabled(), true);
  assert.equal(await dialog().getByRole("button", { name: "取消", exact: true }).isDisabled(), true);
  await page.evaluate(() => window.review.terminalSaveResolver());
  await state("enabled").waitFor();
  assert.equal(await page.evaluate(() => window.review.terminalSetCalls.length), 1);
  console.log("PASS saving locks the dialog and draft until completion without duplicate writes");

  for (const operation of ["enable", "edit", "restore"]) {
    await mount(operation === "enable" ? {} : { terminalStatusPatch: active });
    const nextUrl = "http://127.0.0.1:7891";
    if (operation !== "restore") {
      await open();
      await url().fill(nextUrl);
    }
    await page.evaluate(() => {
      window.review.terminalLoadHold = true;
      void window.review.terminalClient.refetchQueries({ queryKey: ["terminal-proxy"] });
    });
    await page.waitForFunction(() => window.review.terminalLoadResolvers.length === 1);
    if (operation === "restore") await page.getByRole("button", { name: "关闭并恢复", exact: true }).click();
    else await save().click();
    await page.waitForFunction(enabled => window.review.terminalClient.getQueryData(["terminal-proxy"]).enabled === enabled,
      operation !== "restore");
    if (operation === "edit") await page.locator("[data-terminal-proxy]").getByText(nextUrl, { exact: true }).waitFor();
    await page.evaluate(async () => {
      window.review.terminalLoadHold = false;
      window.review.terminalLoadResolvers[0]();
      await new Promise(resolve => setTimeout(resolve, 0));
    });
    assert.deepEqual(await page.evaluate(() => {
      const cached = window.review.terminalClient.getQueryData(["terminal-proxy"]);
      return { enabled: cached.enabled, url: cached.url };
    }), { enabled: operation !== "restore", url: operation === "restore" ? active.url : nextUrl });
    await state(operation === "restore" ? "disabled" : "enabled").waitFor();
    if (operation === "restore") assert.equal(await page.getByRole("button", { name: "打开 PowerShell", exact: true }).count(), 0);
    else assert.equal(await page.getByRole("button", { name: "打开 PowerShell", exact: true }).isEnabled(), true);
  }
  console.log("PASS late status reads cannot overwrite enabling, URL edits or restoration after a successful save");

  await mount({ terminalSaveFailure: "settings.terminalProxy.errors.environment" });
  await open();
  await url().fill("http://127.0.0.1:7890");
  await save().click();
  await dialog().getByRole("alert").filter({ hasText: "无法读取或写入 Windows 用户环境变量" }).waitFor();
  assert.equal(await url().inputValue(), "http://127.0.0.1:7890");
  await page.evaluate(() => { window.review.terminalSaveFailure = null; });
  await save().click();
  await state("enabled").waitFor();
  await page.evaluate(() => { window.review.terminalOpenFailure = true; });
  await page.getByRole("button", { name: "打开 PowerShell", exact: true }).click();
  await page.waitForFunction(() => window.review.terminalOpenCalls === 1);
  await page.waitForFunction(() => window.review.lastToast === "出错了");
  console.log("PASS proxy save failures are translated, preserve the draft for retry and terminal launch failures are surfaced");

  await mount({ terminalLoadFailure: true });
  await state("loadFailed").waitFor();
  await page.evaluate(() => { window.review.terminalLoadFailure = false; });
  await page.getByRole("button", { name: "重试", exact: true }).click();
  await state("disabled").waitFor();
  await mount({ terminalStatusPatch: { supported: false } });
  await state("unsupported").waitFor();
  assert.equal(await edit().isDisabled(), true);
  console.log("PASS terminal proxy retries a failed status load and disables editing on unsupported platforms");

  await mount({ terminalSetResultPatch: { preservedVariables: ["HTTPS_PROXY"] }, terminalStatusPatch: active });
  await page.getByRole("button", { name: "关闭并恢复", exact: true }).click();
  await state("disabled").waitFor();
  await page.waitForFunction(() => window.review.lastToast === "终端代理已关闭");
  console.log("PASS restoration handles preserved external values without changing the app proxy");

  if (captureDir) {
    await mount({ terminalStatusPatch: active });
    await page.setViewportSize({ width: 1000, height: 760 });
    await open();
    await test().click();
    await dialog().getByText("可连接 · HTTP 401", { exact: true }).waitFor();
    for (const theme of ["light", "dark"]) {
      await page.evaluate(value => document.documentElement.classList.toggle("dark", value === "dark"), theme);
      await page.screenshot({ path: path.join(captureDir, `terminal-proxy-${theme}-desktop.png`), animations: "disabled" });
    }
    await page.setViewportSize({ width: 390, height: 844 });
    for (const theme of ["light", "dark"]) {
      await page.evaluate(value => document.documentElement.classList.toggle("dark", value === "dark"), theme);
      const bounds = await dialog().boundingBox();
      assert.ok(bounds && bounds.x >= 0 && bounds.x + bounds.width <= 390);
      await page.screenshot({ path: path.join(captureDir, `terminal-proxy-${theme}-narrow.png`), animations: "disabled" });
    }
    await page.setViewportSize({ width: 1200, height: 700 });
  }
}
