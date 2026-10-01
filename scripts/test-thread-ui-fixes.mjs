import assert from "node:assert/strict";
import path from "node:path";

export async function testThreadUiFixes({ page, reset, captureDir }) {
  const capture = async name => {
    if (captureDir) await page.screenshot({ path: path.join(captureDir, name), animations: "disabled" });
  };
  await page.setViewportSize({ width: 1440, height: 820 });

  await reset("grid");
  await page.getByText("Review thread 0", { exact: true }).waitFor();
  const list = page.locator("[data-thread-list]");
  const groups = list.locator("[data-thread-group]");
  for (const width of [390, 510, 511, 512, 773, 774, 960]) {
    await list.evaluate((element, width) => { element.style.width = `${width}px`; }, width);
    const firstGrid = groups.first().locator("[data-thread-grid]");
    await firstGrid.scrollIntoViewIfNeeded();
    await page.waitForFunction(width => {
      const grid = document.querySelector("[data-thread-grid]");
      return grid?.clientWidth === width;
    }, width);
    const bounds = await groups.first().evaluate(element => {
      const grid = element.querySelector("[data-thread-grid]");
      const card = grid.lastElementChild.getBoundingClientRect();
      const next = element.nextElementSibling.getBoundingClientRect();
      return { columns: getComputedStyle(grid).gridTemplateColumns.split(" ").length,
        lastBottom: card.bottom, nextTop: next.top };
    });
    assert.equal(bounds.columns, Math.max(1, Math.floor((width + 12) / 262)), `Column boundary at ${width}px`);
    assert.ok(bounds.nextTop >= bounds.lastBottom, `No group overlap at ${width}px: ${JSON.stringify(bounds)}`);
  }
  await list.evaluate(element => { element.style.width = "510px"; });
  await page.locator("[data-page-scroll-container]").evaluate(element => { element.scrollTop = 750; });
  await capture("thread-grid-510-fixed.png");
  const last = groups.last();
  await last.getByText("Review thread 99", { exact: true }).scrollIntoViewIfNeeded();
  assert.equal(await last.getByText("Review thread 99", { exact: true }).isVisible(), true);
  assert.equal(await page.evaluate(() => window.scrollY), 0, "Grid scrolling stays inside PageStage");

  // Real long titles wrap above the estimate; the next project must still follow them.
  await reset("thread-tree");
  await page.getByRole("navigation", { name: "项目线程树", exact: true }).waitFor();
  await page.evaluate(() => {
    window.review.threadItems = window.review.threadItems.map(thread => ({
      ...thread, title: "合成的长线程标题。".repeat(40), cliVersion: "1.2.3-synthetic-long-version",
      modelProvider: "synthetic-provider-name", model: "synthetic-model-name",
    }));
  });
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  await page.getByRole("button", { name: "网格视图", exact: true }).click();
  const wrapped = page.locator("[data-thread-list]");
  await wrapped.evaluate(element => { element.style.width = "512px"; });
  const firstGroup = wrapped.locator("[data-thread-group]").first();
  await firstGroup.locator("[data-thread-grid]").scrollIntoViewIfNeeded();
  const wrapping = await firstGroup.evaluate(element => {
    const grid = element.querySelector("[data-thread-grid]");
    return { height: grid.firstElementChild.getBoundingClientRect().height,
      lastBottom: grid.lastElementChild.getBoundingClientRect().bottom,
      nextTop: element.nextElementSibling.getBoundingClientRect().top };
  });
  assert.ok(wrapping.height > 174, "Fixture exercises variable-height cards");
  assert.ok(wrapping.nextTop >= wrapping.lastBottom, "Wrapped cards cannot overlap the next project");
  console.log("PASS thread grids follow real column boundaries and wrapped heights without project overlap; nested scrolling reaches the last card");

  for (const dark of [false, true]) {
    await reset("account-resets");
    await page.evaluate(dark => {
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.setAttribute("data-has-custom-bg", "true");
      document.body.style.backgroundImage = "linear-gradient(45deg, #ff0066, #00ccff)";
    }, dark);
    await page.getByRole("button", { name: "2 次可用重置", exact: true }).click();
    let panel = page.getByRole("dialog");
    await panel.getByText("重置卡 1", { exact: true }).waitFor();
    assert.match(await panel.evaluate(element => getComputedStyle(element).backgroundColor), /, 0\.9\)$/, "Wallpaper popover retains its surface alpha");
    await capture(`reset-credits-wallpaper-${dark ? "dark" : "light"}-fixed.png`);
    await panel.getByRole("button", { name: "关闭", exact: true }).click();
    await panel.waitFor({ state: "hidden" });
    await reset("thread-details");
    await page.evaluate(dark => {
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.setAttribute("data-has-custom-bg", "true");
    }, dark);
    await page.getByRole("button", { name: /^查看详情：/ }).first().click();
    panel = page.getByRole("dialog");
    await panel.getByText("33,301,900 Token", { exact: true }).waitFor();
    assert.match(await panel.evaluate(element => getComputedStyle(element).backgroundColor), /, 0\.5\)$/, "Wallpaper sheet retains its surface alpha");
    await capture(`thread-details-wallpaper-${dark ? "dark" : "light"}-fixed.png`);
    await page.keyboard.press("Escape");
    await panel.waitFor({ state: "hidden" });
  }
  console.log("PASS wallpaper popover and thread sheet keep valid light/dark backgrounds and remain dismissible");

  await reset("thread-details");
  await page.evaluate(() => {
    const message = (role, text) => ({ role, text, timestamp: "2026-09-30T08:00:00Z", truncated: false });
    window.review.messageFixtures = {
      0: [message("user", "合成用户长消息。".repeat(300)), message("assistant", "合成助手 A 长消息。".repeat(300)),
        message("assistant", "合成助手 B 长消息。".repeat(300)), message("toolCall", "synthetic tool call"),
        message("toolResult", "synthetic tool result"), message("reasoning", "synthetic reasoning"),
        message("compaction", "synthetic compaction")],
      50: [message("assistant", "第二页长消息。".repeat(300))],
    };
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: async () => {} } });
  });
  await page.getByRole("button", { name: /^查看详情：/ }).first().click();
  const panel = page.getByRole("dialog");
  await panel.getByRole("tab", { name: "对话", exact: true }).click();
  await panel.getByRole("button", { name: "全部 (7)", exact: true }).waitFor();
  const articles = panel.locator("article");
  await articles.nth(0).getByRole("button", { name: "展开正文", exact: true }).click();
  await articles.nth(0).getByRole("button", { name: "复制消息", exact: true }).click();
  await articles.nth(0).locator(".lucide-check").waitFor();
  await panel.getByRole("button", { name: "助手 (2)", exact: true }).click();
  assert.equal(await articles.count(), 2);
  assert.equal(await articles.first().getByRole("button", { name: "展开正文", exact: true }).count(), 1, "Assistant does not inherit expanded user state");
  assert.equal(await articles.first().locator(".lucide-check").count(), 0, "Assistant does not inherit copied user state");
  await articles.first().getByRole("button", { name: "展开正文", exact: true }).click();
  await panel.getByRole("button", { name: "全部 (7)", exact: true }).click();
  assert.equal(await articles.nth(1).getByRole("button", { name: "收起正文", exact: true }).count(), 1, "A surviving message keeps its own state across filtering");
  assert.equal(await articles.nth(0).getByRole("button", { name: "展开正文", exact: true }).count(), 1);
  await panel.getByRole("button", { name: "工具 (2)", exact: true }).click();
  assert.equal(await articles.count(), 2);
  await panel.getByRole("button", { name: "推理 (2)", exact: true }).click();
  assert.equal(await articles.count(), 2);
  await panel.getByRole("button", { name: "全部 (7)", exact: true }).click();
  await panel.getByRole("button", { name: "下一页", exact: true }).click();
  await panel.getByRole("button", { name: "全部 (1)", exact: true }).waitFor();
  assert.equal(await articles.first().getByRole("button", { name: "展开正文", exact: true }).count(), 1, "Page state cannot migrate across offsets");
  await page.evaluate(() => {
    window.review.snapshotBytes = 54321;
    window.review.messageFixtures[0] = [{ role: "assistant", timestamp: "2026-09-30T08:00:00Z",
      text: "新快照长消息。".repeat(300), truncated: false }];
  });
  await articles.first().getByRole("button", { name: "展开正文", exact: true }).click();
  await panel.getByRole("button", { name: "重新读取记录", exact: true }).click();
  await page.waitForFunction(() => window.review.messageRequests.some(request => request.snapshot === 54321 && request.offset === 0));
  await articles.first().getByText(/^新快照长消息/).waitFor();
  assert.equal(await articles.first().getByRole("button", { name: "展开正文", exact: true }).count(), 1, "A new snapshot starts with fresh message state");
  await capture("thread-message-filter-state-fixed.png");
  await page.keyboard.press("Escape");
  await panel.waitFor({ state: "hidden" });
  console.log("PASS message filters keep original identity, isolate expanded/copied state, agree with counts, and reset on new pages/snapshots");
}
