import assert from "node:assert/strict";
import path from "node:path";

export async function testThreadDetails({ page, reset, captureDir }) {
  const open = async mode => {
    await reset(mode);
    const trigger = page.getByRole("button", { name: /^查看详情：/ }).first();
    await trigger.click();
    const panel = page.getByRole("dialog");
    await panel.waitFor();
    return { trigger, panel };
  };
  await page.setViewportSize({ width: 1440, height: 820 });
  let { trigger, panel } = await open("thread-details");
  await panel.getByText("33,301,900 Token", { exact: true }).waitFor();
  await page.evaluate(() => {
    window.review.copiedText = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async text => {
      if (window.review.clipboardError) throw new Error("Synthetic clipboard denial");
      window.review.copiedText.push(text);
    } } });
  });
  await panel.getByRole("button", { name: /复制.*ID/ }).click();
  assert.deepEqual(await page.evaluate(() => window.review.copiedText), ["t0"]);
  assert.equal(await page.evaluate(() => window.review.messageOffsets.length), 0, "Opening details does not eagerly read transcript bodies");
  assert.equal(await page.getByText("已选 1 个", { exact: true }).count(), 0, "Opening details must not select the card");
  await panel.getByText("aimami_relay_2d66f8bc24::gpt-6-astra", { exact: true }).waitFor();
  await panel.getByText("查看每日记录", { exact: true }).click();
  assert.equal(await panel.locator("tbody tr").count(), 2);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "thread-details-usage-light.png"), animations: "disabled" });
  await panel.getByRole("tab", { name: "用量", exact: true }).focus();
  await page.keyboard.press("ArrowRight");
  await panel.getByText("exec_command", { exact: true }).waitFor();
  await panel.getByText("12 次调用", { exact: true }).waitFor();
  await page.keyboard.press("ArrowRight");
  await panel.getByText("JSONL 路径", { exact: true }).waitFor();
  await page.keyboard.press("End");
  await panel.getByText("合成的用户问题", { exact: true }).waitFor();
  await panel.getByRole("button", { name: "复制消息", exact: true }).first().click();
  assert.deepEqual(await page.evaluate(() => window.review.copiedText), ["t0", "合成的用户问题"]);
  await page.evaluate(() => { window.review.clipboardError = true; });
  await panel.getByRole("button", { name: "复制消息", exact: true }).first().click();
  await page.waitForFunction(() => window.review.lastToast === "复制失败");
  assert.equal(await panel.locator("script").count(), 0, "JSONL text cannot create DOM scripts");
  await panel.getByRole("button", { name: "展开正文", exact: true }).click();
  await panel.getByText(/仅预览前 20,000 字符/).waitFor();
  await panel.getByRole("button", { name: "收起正文", exact: true }).click();
  await panel.getByRole("button", { name: "下一页", exact: true }).click();
  await panel.getByText("第二页的合成消息", { exact: true }).waitFor();
  assert.equal(await panel.getByRole("button", { name: "下一页", exact: true }).isDisabled(), true);
  assert.deepEqual(await page.evaluate(() => window.review.messageOffsets), [0, 50]);
  assert.equal(await panel.getByRole("tabpanel").evaluate(element => element.scrollTop), 0, "Pagination resets scroll position");
  await panel.getByRole("button", { name: "上一页", exact: true }).click();
  await panel.getByText("合成的用户问题", { exact: true }).waitFor();
  await panel.getByText("记录诊断（点击查看）", { exact: true }).first().click();
  await panel.getByText(/2 条记录包含不展示的加密载荷/).first().waitFor();
  // Same-size edits still invalidate the snapshot token. Retry must get a new
  // detail snapshot and return to page one instead of repeating the old token.
  await page.evaluate(() => { window.review.snapshotId = "fixture-2"; });
  await panel.getByRole("button", { name: "下一页", exact: true }).click();
  await panel.getByRole("alert").getByText("Synthetic stale snapshot; refresh", { exact: true }).waitFor();
  const detailCalls = await page.evaluate(() => window.review.detailCalls);
  await panel.getByRole("button", { name: "重试", exact: true }).click();
  await panel.getByText("合成的用户问题", { exact: true }).waitFor();
  assert.equal(await panel.getByRole("alert").count(), 0);
  assert.ok(await page.evaluate(() => window.review.detailCalls) > detailCalls);
  assert.deepEqual(await page.evaluate(() => window.review.messageRequests.at(-1)),
    { snapshot: 12345, offset: 0, snapshotId: "fixture-2" });
  await page.keyboard.press("Escape");
  await panel.waitFor({ state: "hidden" });
  assert.equal(await trigger.evaluate(element => element === document.activeElement), true, "Escape restores invoking title focus");

  ({ panel } = await open("thread-details"));
  await panel.getByRole("button", { name: "删除会话", exact: true }).click();
  await panel.waitFor({ state: "hidden" });
  await page.getByRole("alertdialog").getByRole("button", { name: "取消", exact: true }).click();
  await page.getByRole("alertdialog").waitFor({ state: "hidden" });

  ({ panel } = await open("thread-details-unknown"));
  await panel.getByText(/不代表用量为 0/).waitFor();
  await page.keyboard.press("Escape");
  ({ panel } = await open("thread-details-error"));
  await panel.getByRole("alert").getByText("Synthetic rollout identity mismatch").waitFor();
  await panel.getByRole("tab", { name: "元数据", exact: true }).click();
  await panel.getByText("JSONL 路径", { exact: true }).waitFor();
  await page.keyboard.press("Escape");
  ({ panel } = await open("thread-details-missing"));
  await panel.getByText(/原始 JSONL 记录不存在/).waitFor();
  assert.equal(await panel.getByRole("button", { name: "查看对话", exact: true }).isDisabled(), true);
  assert.equal(await page.evaluate(() => window.review.detailCalls), 0);
  await page.keyboard.press("Escape");

  await page.setViewportSize({ width: 390, height: 700 });
  ({ panel } = await open("thread-details"));
  await panel.getByText("33,301,900 Token", { exact: true }).waitFor();
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(await panel.evaluate(element => element.scrollWidth <= element.clientWidth), true, "No mobile horizontal overflow");
  const footer = await panel.locator("footer").boundingBox();
  assert.ok(footer.y + footer.height <= 701, "Footer remains visible in the small viewport");
  const closeButton = await panel.getByRole("button", { name: "关闭", exact: true }).boundingBox();
  assert.ok(closeButton.width >= 40 && closeButton.height >= 40, "Close has a usable touch target");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "thread-details-usage-mobile-dark.png"), animations: "disabled" });
  await panel.getByRole("button", { name: "查看对话", exact: true }).click();
  await panel.getByText("合成的用户问题", { exact: true }).waitFor();
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "thread-details-chat-mobile-dark.png"), animations: "disabled" });
  await panel.getByRole("button", { name: "关闭", exact: true }).click();
  await panel.waitFor({ state: "hidden" });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  console.log("PASS read-only thread details: usage/models/dates/tools/metadata, lazy paged chat, safe text, diagnostics, unknown/missing/errors, keyboard/focus, mobile close/footer");
}
