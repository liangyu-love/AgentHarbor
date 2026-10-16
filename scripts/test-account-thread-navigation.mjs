import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testAccountThreadNavigation({ page, reset, captureDir }) {
  const capture = async name => { if (captureDir) await page.screenshot({ path: path.join(captureDir, name), animations: "disabled" }); };
  await page.setViewportSize({ width: 1440, height: 800 });
  await reset("account-resets");
  const badge = page.getByRole("button", { name: "2 次可用重置", exact: true });
  await badge.hover();
  const panel = page.getByRole("dialog", { name: "重置卡 2" });
  await panel.waitFor();
  await panel.getByText("重置卡 1", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await panel.getByText("23 天后到期", { exact: true }).count(), 1));
  assert.equal(await panel.getByText("30 天后到期", { exact: true }).count(), 1);
  await panel.hover();
  await page.waitForTimeout(250);
  assert.equal(await panel.isVisible(), true, "Hover panel must survive the gap from its trigger");
  await panel.locator("summary").click();
  await eventually(async () => assert.equal(await panel.locator("time").count(), 3, "History excludes grants older than 30 days"));
  await panel.getByText("已使用", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await panel.getByText("已到期", { exact: true }).count(), 0));
  await capture("official-reset-credits-light.png");
  await panel.getByRole("button", { name: "刷新", exact: true }).click();
  await page.waitForFunction(() => window.review.accountRefreshCalls > 0);
  await page.keyboard.press("Escape");
  await panel.waitFor({ state: "hidden" });
  await eventually(async () => assert.equal(await badge.getAttribute("aria-expanded"), "false", "Focus return must not reopen a dismissed panel"));
  await badge.focus();
  await page.keyboard.press("Enter");
  await panel.waitFor();
  await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "关闭");
  await page.keyboard.press("Escape");
  await panel.waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "0 次可用重置", exact: true }).click();
  const zero = page.getByRole("dialog", { name: "重置卡 0" });
  await zero.getByText("暂无可用重置卡").waitFor();
  await zero.locator("summary").click();
  await zero.getByText("已使用", { exact: true }).waitFor();
  await zero.getByRole("button", { name: "关闭", exact: true }).click();
  await zero.waitFor({ state: "hidden" });
  await page.getByRole("button", { name: "重置卡：尚未查询", exact: true }).click();
  await page.getByRole("dialog").getByText(/未提供不代表 0 张/).waitFor();
  await page.keyboard.press("Escape");
  await reset("account-resets-unavailable");
  await page.getByRole("button", { name: "2 次可用重置", exact: true }).click();
  await page.getByRole("dialog").getByText(/暂时无法读取卡片详情/).waitFor();
  await eventually(async () => assert.equal(await page.getByText("重置卡 1", { exact: true }).count(), 0));
  console.log("PASS official reset credits distinguish zero/unknown/failed details, show expiry and 30-day history, hover/keyboard/close/refresh");

  await reset("thread-tree");
  const tree = page.getByRole("navigation", { name: "项目线程树", exact: true });
  const results = page.locator("[data-thread-results]");
  await tree.getByRole("button", { name: "所有项目 100", exact: true }).waitFor();
  await eventually(async () => assert.equal(await tree.getByRole("button", { name: "展开或收起 Lighthouse", exact: true }).count(), 2));
  const project = tree.locator('button[title="C:/work/Lighthouse"]');
  const other = tree.locator('button[title="D:/work/Lighthouse"]');
  assert.equal(await tree.getByText("C:/work/Lighthouse", { exact: true }).count(), 1);
  await project.click();
  await eventually(async () => assert.equal(await results.getByText("Lighthouse", { exact: true }).count() > 0, true));
  assert.ok(await results.getByRole("checkbox").count() < 90, "Main list stays virtualized");
  await tree.getByRole("button", { name: "展开或收起 Lighthouse", exact: true }).first().click();
  await eventually(async () => assert.equal(await tree.locator('button[title^="Review thread"]').count(), 30, "Large tree branches are paged"));
  await tree.getByRole("button", { name: "显示更多（剩余 60 条）", exact: true }).click();
  await eventually(async () => assert.equal(await tree.locator('button[title^="Review thread"]').count(), 60));
  await tree.getByRole("button", { name: "Review thread 42", exact: true }).click();
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 1));
  await results.getByRole("checkbox", { name: "Review thread 42", exact: true }).click();
  await page.getByText("已选 1 个", { exact: true }).first().waitFor();
  await other.focus();
  await page.keyboard.press("Enter");
  await page.getByText("已选 1 个", { exact: true }).first().waitFor({ state: "hidden" });
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 5, "Same-name projects stay isolated by path"));
  await page.getByPlaceholder(/搜索.*标题/).fill("Review thread 91");
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 1));
  await tree.getByRole("button", { name: "所有项目 1/100", exact: true }).waitFor();
  await project.click();
  await results.getByText("未找到匹配的会话", { exact: true }).waitFor();
  await page.getByPlaceholder(/搜索.*标题/).fill("");
  await page.getByRole("radio", { name: "已归档 45", exact: true }).click();
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 45));
  assert.match(await project.textContent(), /45\/90/);
  await page.getByRole("radio", { name: "全部 90", exact: true }).click();
  await tree.getByRole("button", { name: "Review thread 42", exact: true }).click();
  await page.getByRole("radio", { name: "进行中 45", exact: true }).click();
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 45, "Archived focus is cleared by the active filter"));
  await other.click();
  await tree.getByRole("button", { name: "展开或收起 Lighthouse", exact: true }).nth(1).click();
  await tree.getByRole("button", { name: "Review thread 91", exact: true }).click();
  await page.evaluate(() => {
    window.review.threadItems = window.review.threadItems
      .filter(thread => thread.cwd !== "D:/work/Lighthouse" || thread.key === "t91")
      .map(thread => thread.key === "t91" ? { ...thread, cwd: "C:/work/Lighthouse" } : thread);
  });
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('nav button[title="D:/work/Lighthouse"]'));
  await eventually(async () => assert.equal(await tree.getByRole("button", { name: /所有项目/ }).getAttribute("aria-current"), "page", "Removed project falls back to all threads"));
  assert.ok(await results.getByRole("checkbox").count() > 1, "A moved thread must not leave an invisible single-thread focus");

  await reset("thread-tree");
  await page.evaluate(() => {
    window.review.threadItems = window.review.threadItems.slice(0, 7).map((thread, index) => ({
      ...thread,
      cwd: ["D:/workspace/Lighthouse", "\\\\?\\D:\\workspace\\Lighthouse", "d:/WORKSPACE/lighthouse/",
        "C:/Users/demo/Documents/Codex/2026-09-30/new-chat",
        "\\\\?\\C:\\Users\\demo\\Documents\\Codex\\2026-09-29\\renamed-chat",
        "C:/work/new-chat", "C:/work/Lighthouse"][index],
      folderName: ["Lighthouse", "Lighthouse", "lighthouse", "new-chat", "renamed-chat", "new-chat", "Lighthouse"][index],
      userMessages: index + 1, agentMessages: (index + 1) * 2,
    }));
  });
  await page.getByRole("button", { name: "刷新", exact: true }).click();
  await tree.getByRole("button", { name: "所有项目 7", exact: true }).waitFor();
  const metrics = page.locator("[data-thread-metrics]");
  const metric = name => metrics.locator(`[data-metric="${name}"]`);
  assert.equal(await metric("threads").textContent(), "7");
  assert.equal(await metric("projects").textContent(), "3", "Direct chats do not count as workspaces");
  assert.equal(await tree.getByRole("button", { name: "展开或收起 Lighthouse", exact: true }).count(), 2, "Windows aliases merge but different paths remain separate");
  await tree.locator('button[title="D:/workspace/Lighthouse"]').click();
  await eventually(async () => assert.equal(await metric("threads").textContent(), "3"));
  // The overview card reports token breakdown now: 4 input / 3 cached / 6
  // output per fixture thread. Input shows its uncached part, as the usage
  // page does, so the three parts add up to the total.
  assert.equal(await metric("input").textContent(), "3");
  assert.equal(await metric("cached").textContent(), "9");
  assert.equal(await metric("output").textContent(), "18");
  assert.equal(await metric("tokens-total").textContent(), "Token 总量：30");
  assert.equal(await metric("storage").textContent(), "300 B");
  assert.equal(await results.getByRole("checkbox").count(), 3, "Project filtering uses the normalized identity too");
  await page.getByPlaceholder(/搜索.*标题/).fill("no-such-thread");
  await results.getByText("未找到匹配的会话", { exact: true }).waitFor();
  await eventually(async () => assert.equal(await metric("threads").textContent(), "3", "Search does not silently change project totals"));
  await page.getByPlaceholder(/搜索.*标题/).fill("");
  await tree.getByRole("button", { name: "直接对话 2", exact: true }).click();
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 2));
  assert.equal(await metric("projects").textContent(), "0");
  assert.equal(await metric("input").textContent(), "2");
  assert.equal(await metric("cached").textContent(), "6");
  assert.equal(await metric("output").textContent(), "12");
  await tree.getByRole("button", { name: "展开或收起 直接对话", exact: true }).click();
  await tree.getByRole("button", { name: "Review thread 4", exact: true }).click();
  await eventually(async () => assert.equal(await results.getByRole("checkbox").count(), 1));
  assert.equal(await metric("threads").textContent(), "2", "Thread focus preserves the selected folder overview");
  await tree.getByRole("button", { name: "所有项目 7", exact: true }).click();
  await eventually(async () => assert.equal(await metric("input").textContent(), "7"));
  assert.equal(await metric("output").textContent(), "42");
  await capture("thread-project-overview-desktop.png");
  await page.setViewportSize({ width: 390, height: 700 });
  await capture("thread-project-overview-mobile.png");
  assert.equal(await page.locator("[data-page-scroll-container]").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await page.setViewportSize({ width: 1440, height: 800 });
  console.log("PASS Windows aliases merge, direct chats share a folder, and input/output/storage totals follow project selection");

  await reset("thread-tree");
  await tree.getByRole("button", { name: "展开或收起 Lighthouse", exact: true }).first().click();
  await page.getByRole("button", { name: "网格视图", exact: true }).click();
  await capture("thread-project-tree-light.png");
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await capture("thread-project-tree-dark.png");
  assert.equal(await results.evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await page.setViewportSize({ width: 960, height: 640 });
  await reset("thread-tree");
  assert.equal(await tree.getByRole("button", { name: "项目线程树 4", exact: true }).getAttribute("aria-expanded"), "false");
  await tree.getByRole("button", { name: "项目线程树 4", exact: true }).click();
  await capture("thread-project-tree-960.png");
  await page.setViewportSize({ width: 390, height: 700 });
  await capture("thread-project-tree-narrow.png");
  assert.equal(await page.locator("[data-page-scroll-container]").evaluate(element => element.scrollWidth <= element.clientWidth), true);
  await reset("account-resets");
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await page.getByRole("button", { name: "2 次可用重置", exact: true }).click();
  const bounds = await page.getByRole("dialog").boundingBox();
  assert.ok(bounds.x >= 0 && bounds.x + bounds.width <= 390, "Portal is not clipped by the account card or viewport");
  await capture("official-reset-credits-narrow-dark.png");
  await page.keyboard.press("Escape");
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS project tree uses full-path groups, paged children, search/status/refetch navigation, safe selection and responsive virtual grids");
}
