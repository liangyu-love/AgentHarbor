import assert from "node:assert/strict";
import { eventually } from "./eventually.mjs";

// Long lists show a page and bring in the next as they scroll; no button to press.
export async function testScrollPaging({ page, reset }) {
  await page.setViewportSize({ width: 1200, height: 700 });
  await reset("usage");
  await page.evaluate(() => {
    const now = Math.floor(Date.now() / 1000);
    window.review.usageEntries = Array.from({ length: 130 }, (_, index) => window.review.usageEntry({
      timestamp: now - 60 - index, client: index % 2 ? "codex" : "claudeCode", model: `paged-model-${index}` }));
    window.review.mount("usage");
  });
  const requests = page.locator("[data-usage-requests]");
  const rows = () => requests.locator("tbody > tr").count();
  await eventually(async () => assert.equal(await rows(), 50));
  assert.equal(await requests.getByRole("button", { name: /显示更多|Show more/ }).count(), 0, "No button to press");
  await page.waitForTimeout(150);
  assert.equal(await rows(), 50, "The next page waits until the table nears its end");
  for (const expected of [100, 130]) {
    await requests.locator("[data-usage-requests-end]").scrollIntoViewIfNeeded();
    await eventually(async () => assert.equal(await rows(), expected));
  }
  assert.equal(await requests.locator("[data-usage-requests-end]").count(), 0);
  await requests.getByLabel("客户端来源", { exact: true }).selectOption("codex");
  await eventually(async () => assert.equal(await rows(), 50, "A new filter starts from the first page"));
  console.log("PASS usage requests load page by page on scroll and restart on a new filter");
}
