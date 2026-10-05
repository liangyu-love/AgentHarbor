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
  const commandBalance = { remaining: 62, used: 8, rawQuota: 62, unit: "credits", quotaPerUnit: null,
    requestCount: null, checkedAt: 1800000000, conversionAvailable: true, entries: [
      entry(62, 70, "Command Code · GOAT"), entry(0, null, "purchased"), entry(0, null, "free"), entry(14, 14, "fiveHour"), entry(30, 40, "weekly"),
    ] };
  const details = {
    account: { name: "默认账户", userName: "liangyu-love" },
    plan: { id: "individual-goat", name: "GOAT", status: "active", currentPeriodStart: "2026-10-02T00:00:00Z", currentPeriodEnd: "2026-11-02T00:00:00Z" },
    usage: { totalCount: 2159, completedCount: 2159, failedCount: 0, successRate: 100, totalCost: 2.8453,
      totalCredits: 2.85, totalTokensIn: 124800000, totalTokensOut: 1100000, periodBasis: "billing-period" },
  };
  const loadCommand = balance => loadProvider({ baseUrl: "https://api.commandcode.ai/provider/v1", balanceSource: "commandCode", balance });
  await loadCommand({ ...commandBalance, commandCode: details });
  assert.ok((await face.innerText()).includes("$62"));
  assert.match(await face.innerText(), /Command Code · GOAT/);
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  const summary = dialog.locator("[data-command-code-balance]");
  await summary.getByText("liangyu-love", { exact: true }).waitFor();
  assert.match(await summary.innerText(), /默认账户/);
  assert.match(await summary.locator('[data-command-code-stat="requests"]').innerText(), /2,159/);
  assert.match(await summary.locator('[data-command-code-stat="requests"]').innerText(), /失败 0/);
  assert.match(await summary.locator('[data-command-code-stat="successRate"]').innerText(), /100%/);
  assert.match(await summary.locator('[data-command-code-stat="cost"]').innerText(), /\$2\.8453/);
  assert.match(await summary.locator('[data-command-code-stat="cost"]').innerText(), /\$2\.85/);
  assert.match(await summary.locator('[data-command-code-stat="tokens"]').innerText(), /125\.9M/);
  assert.match(await summary.locator('[data-command-code-stat="tokens"]').innerText(), /124\.8M 入 \/ 1\.1M 出/);
  assert.equal(await summary.locator('[data-command-code-window="fiveHour"] [role="progressbar"]').getAttribute("aria-valuenow"), "0");
  assert.equal(await summary.locator('[data-command-code-window="weekly"] [role="progressbar"]').getAttribute("aria-valuenow"), "25");
  assert.equal(await summary.getByText("余额不足或账户当前不可用", { exact: true }).count(), 0);
  assert.match(await summary.innerText(), /账期截止/);
  assert.match(await summary.locator('[data-command-code-credits]').innerText(), /\$62[\s\S]*\$0[\s\S]*\$0/);
  assert.equal((await summary.innerText()).includes("credits"), false);
  assert.match(await summary.locator('[data-command-code-window="weekly"]').innerText(), /\$10 \/ \$40/);
  if (captureDir) {
    for (const width of [1200, 390]) {
      await page.setViewportSize({ width, height: 900 });
      for (const dark of [false, true]) {
        await page.evaluate(value => document.documentElement.classList.toggle("dark", value), dark);
        await page.screenshot({ path: path.join(captureDir, `command-code-${width}-${dark ? "dark" : "light"}.png`), animations: "disabled" });
        const size = await dialog.evaluate(element => ({ scroll: element.scrollWidth, client: element.clientWidth }));
        assert.ok(size.scroll <= size.client + 1, `Command Code dialog overflow at ${width}px: ${JSON.stringify(size)}`);
      }
    }
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.evaluate(() => document.documentElement.classList.remove("dark"));
  }
  console.log("PASS Command Code official account statistics and quota use the requested dollar notation without changing numeric values");
  await page.waitForFunction(() => window.review.balanceRequests.length === 1);
  await page.evaluate(() => window.review.balanceRejectors[0]("newapi.errors.auth"));
  await dialog.getByText("查询未成功，保留上次查询结果。", { exact: true }).waitFor();
  assert.match(await summary.innerText(), /125\.9M/);
  assert.equal(await dialog.getByRole("button", { name: "刷新余额", exact: true }).isEnabled(), true);
  console.log("PASS failed Command Code refresh keeps the complete last successful report and remains retryable");

  await loadCommand(commandBalance);
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  await summary.waitFor();
  assert.equal(await summary.locator('[data-command-code-usage]').count(), 0);
  assert.match(await summary.innerText(), /\$62/);
  assert.match(await summary.innerText(), /Command Code · GOAT/);
  console.log("PASS old saved Command Code balances still show their credits and windows without inventing missing statistics");

  const partialUsage = Object.fromEntries(Object.keys(details.usage).map(key => [key, null]));
  await loadCommand({ ...commandBalance, commandCode: { ...details, usage: { ...partialUsage, failedCount: 0, totalCost: 0, totalTokensIn: 0 } } });
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  await summary.waitFor();
  assert.match(await summary.locator('[data-command-code-stat="requests"]').innerText(), /—[\s\S]*失败 0/);
  assert.match(await summary.locator('[data-command-code-stat="successRate"]').innerText(), /—/);
  assert.match(await summary.locator('[data-command-code-stat="cost"]').innerText(), /\$0/);
  assert.match(await summary.locator('[data-command-code-stat="tokens"]').innerText(), /—[\s\S]*0 入 \/ — 出/);
  assert.equal(await summary.getByText("本账期用量", { exact: true }).count(), 0);
  console.log("PASS missing official fields remain unknown, reported zero stays zero and an unknown period is not labeled as billing-period usage");

  await loadCommand({ ...commandBalance, entries: [{ ...entry(8, 10, "weekly"), valid: false }, entry(5, 10, "future-pool")] });
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  await summary.waitFor();
  assert.equal(await summary.locator('[data-command-code-credits]').count(), 0);
  assert.equal(await summary.getByText("窗口额度已用尽", { exact: true }).count(), 1);
  assert.equal(await summary.locator('[data-command-code-window="weekly"] [role="progressbar"]').getAttribute("aria-valuenow"), "20");
  console.log("PASS exhausted windows remain visible and unknown future pools do not become monthly credits");

  await loadCommand({ ...commandBalance, remaining: 7, used: 3, rawQuota: 7, entries: [entry(7, 10, "weekly")] });
  const quotaLabels = await page.evaluate(() => window.review.labels);
  assert.notEqual(quotaLabels.windowRemaining, "newapi.balanceWindowRemaining");
  await face.getByText(quotaLabels.windowRemaining, { exact: true }).waitFor();
  await face.getByText(quotaLabels.weeklyWindow, { exact: true }).waitFor();
  assert.ok((await face.innerText()).includes("$7"));
  if (captureDir) await face.screenshot({ path: path.join(captureDir, "command-code-window-only.png"), animations: "disabled" });
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  await summary.waitFor();
  assert.equal(await summary.locator('[data-command-code-credits]').count(), 0);
  assert.equal(await summary.locator('[data-command-code-window="weekly"] [role="progressbar"]').getAttribute("aria-valuenow"), "30");
  console.log("PASS window-only Command Code results are labeled as quota instead of account balance");

  await loadProvider({ baseUrl: "https://api.commandcode.ai/provider/v1", balanceSource: "custom", balanceScript: "return {remaining: 62, unit: 'credits'};", balance: commandBalance });
  await page.getByRole("button", { name: "查询余额 Quota fixture", exact: true }).click();
  assert.equal(await summary.count(), 0);
  assert.ok((await face.innerText()).includes("62 credits"));
  assert.match(await dialog.innerText(), /已购额度剩余/);
  assert.equal(await dialog.getByText("余额不足或账户当前不可用", { exact: true }).count(), 2);
  console.log("PASS custom balance scripts and other balance sources keep the generic balance interface");
}
