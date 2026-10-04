import assert from "node:assert/strict";
import path from "node:path";

export async function testProviderFeedback({ page, reset, captureDir }) {
  await reset("groups");
  await page.getByRole("button", { name: "添加中转模型", exact: true }).click();
  const dialog = page.getByRole("dialog");
  for (const [name, baseUrl] of [
    ["MiMo API", "https://api.xiaomimimo.com/v1"],
    ["MiMo Token Plan", "https://token-plan-cn.xiaomimimo.com/v1"],
    ["Command Code", "https://api.commandcode.ai/provider/v1"],
  ]) {
    await dialog.locator("#provider-preset").selectOption(name);
    assert.equal(await dialog.locator("#newapi-provider-baseUrl").inputValue(), baseUrl);
    assert.equal(await dialog.locator("#newapi-provider-name").inputValue(), name);
    assert.equal(await dialog.locator("#newapi-provider-apiKey").inputValue(), "");
  }
  console.log("PASS MiMo API, Token Plan and Command Code presets fill their independent official endpoints without inventing credentials");

  const loadProvider = async (patch, shared = false) => {
    await reset("compact");
    await page.evaluate(({ patch, shared }) => {
      const original = window.review.getState().providers[0];
      const balance = { remaining: 12.5, used: 2.5, rawQuota: 12.5, unit: "CNY",
        quotaPerUnit: null, requestCount: null, checkedAt: 1800000000, conversionAvailable: true };
      const item = { ...original, id: "group-a", name: "Quota fixture", baseUrl: "https://api.xiaomimimo.com/v1",
        balance, balanceSource: "custom", ...patch };
      window.review.providerFixtures = shared ? [item, { ...item, id: "group-b", name: "Shared fixture", balanceScript: "", balanceSharedFrom: item.id }] : [item];
      window.review.mount("groups");
    }, { patch, shared });
    await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).waitFor();
  };
  await loadProvider({});
  const face = page.locator("[data-relay-card-face]");
  assert.match(await face.innerText(), /¥12.5/);
  assert.equal(await face.getByRole("button", { name: "刷新余额", exact: true }).isDisabled(), true);
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  await dialog.getByText(/本应用尚未接入 MiMo 官方余额查询/).waitFor();
  assert.match(await dialog.locator("strong").innerText(), /12.5/);
  assert.equal(await dialog.getByRole("button", { name: "刷新余额", exact: true }).isDisabled(), true);
  assert.deepEqual(await page.evaluate(() => window.review.balanceRequests), []);
  await dialog.getByRole("checkbox", { name: "自定义余额脚本", exact: true }).check();
  await dialog.getByRole("button", { name: "保存并查询", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceRequests.length === 1);
  await page.evaluate(() => window.review.balanceResolvers[0]());
  await dialog.getByText("USD", { exact: true }).first().waitFor();
  assert.equal(await dialog.getByRole("button", { name: "刷新余额", exact: true }).isEnabled(), true);
  console.log("PASS MiMo keeps existing balances, avoids unsupported automatic queries and permits an explicit custom balance script");

  await loadProvider({ balanceScript: "return { remaining: 12.5, unit: 'CNY' };" }, true);
  await page.getByRole("button", { name: "下一个站点配置" }).click();
  await page.getByRole("button", { name: "查询余额 Shared fixture", exact: true }).click();
  await dialog.getByText("与「Quota fixture」共享余额。", { exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.review.balanceRequests), []);
  assert.equal(await dialog.getByRole("button", { name: "刷新余额", exact: true }).isEnabled(), true);
  await dialog.getByRole("button", { name: "刷新余额", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceRequests.length === 1);
  assert.equal(await page.evaluate(() => window.review.balanceRequests[0]), "group-b");
  await page.evaluate(() => window.review.balanceResolvers[0]());
  console.log("PASS shared MiMo balance honors its owner script without silently changing the existing automatic-query behavior");

  const entry = (remaining, total, label) => ({ remaining, total, used: total == null ? null : total - remaining, unit: "credits", valid: remaining > 0, label, resetsAt: "2030-01-01T00:00:00Z" });
  await loadProvider({ baseUrl: "https://api.commandcode.ai/provider/v1", balanceSource: "commandCode",
    balance: { remaining: 62, used: 8, rawQuota: 62, unit: "credits", quotaPerUnit: null,
      requestCount: null, checkedAt: 1800000000, conversionAvailable: true, entries: [
        entry(62, 70, "Command Code · GOAT"), entry(10, null, "purchased"), entry(2, null, "free"), entry(3, 5, "fiveHour"), entry(30, 40, "weekly"),
      ] } });
  assert.ok((await face.innerText()).includes("62 credits"));
  assert.match(await face.innerText(), /Command Code · GOAT/);
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  await page.waitForFunction(() => window.review.balanceRequests.length === 1);
  await page.evaluate(() => window.review.balanceRejectors[0]("newapi.errors.auth"));
  await dialog.getByText("查询未成功，保留上次查询结果。", { exact: true }).waitFor();
  for (const label of ["Command Code · GOAT", "5 小时", "本周剩余", "已购额度剩余", "免费额度剩余"]) {
    assert.match(await dialog.innerText(), new RegExp(label, "i"));
  }
  assert.match(await dialog.locator("strong").first().innerText(), /62/);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "command-code-plan.png"), animations: "disabled" });
  console.log("PASS Command Code shows plan and quota windows in the existing balance UI and keeps the last result when refresh fails");
  await loadProvider({ baseUrl: "https://api.commandcode.ai/provider/v1", balanceSource: "commandCode",
    balance: { remaining: 7, used: 3, rawQuota: 7, unit: "credits", quotaPerUnit: null,
      requestCount: null, checkedAt: 1800000000, conversionAvailable: true, entries: [entry(7, 10, "weekly")] } });
  const quotaLabels = await page.evaluate(() => window.review.labels);
  assert.notEqual(quotaLabels.windowRemaining, "newapi.balanceWindowRemaining");
  await face.getByText(quotaLabels.windowRemaining, { exact: true }).waitFor();
  await face.getByText(quotaLabels.weeklyWindow, { exact: true }).waitFor();
  assert.ok((await face.innerText()).includes("7 credits"));
  if (captureDir) await face.screenshot({ path: path.join(captureDir, "command-code-window-only.png"), animations: "disabled" });
  console.log("PASS window-only Command Code results are labeled as quota instead of account balance");

}
