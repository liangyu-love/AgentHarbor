import assert from "node:assert/strict";
import path from "node:path";

export async function testMagpieOptimizations({ page, reset, captureDir }) {
  await reset("access");
  const routes = page.getByRole("region", { name: "请求级路由诊断" });
  await routes.getByRole("button", { name: "请求级路由诊断", exact: true }).click();
  await routes.locator("summary").waitFor();
  await routes.locator("summary").click();
  await routes.getByText("客户端请求参数", { exact: true }).waitFor();
  assert.equal(await routes.getByText("reasoning.effort", { exact: true }).isVisible(), true);
  assert.equal(await routes.getByText("output_config.effort", { exact: true }).isVisible(), true);
  assert.match(await routes.innerText(), /claude-opus-5-5-kiro/);
  assert.match(await routes.innerText(), /claude-opus-5-5 → Synthetic channel/);
  assert.match(await routes.innerText(), /230 ms/);
  const toggle = routes.getByRole("switch", { name: "同模型渠道故障切换" });
  assert.equal(await toggle.getAttribute("aria-checked"), "false");
  await toggle.click();
  await page.waitForFunction(() => document.querySelector('[role="switch"][aria-label="同模型渠道故障切换"]')?.getAttribute("aria-checked") === "true");
  const catalog = page.getByRole("region", { name: "模型能力目录与匹配预览" });
  await catalog.getByRole("button", { name: "模型能力目录与匹配预览", exact: true }).click();
  await catalog.locator("summary").waitFor();
  await catalog.locator("summary").click();
  assert.equal(await catalog.getByText("My custom alias", { exact: false }).isVisible(), true);
  assert.match(await catalog.innerText(), /继续使用上一份有效目录/);
  assert.match(await catalog.innerText(), /1,000,000/);
  assert.match(await catalog.innerText(), /内置模型规则/);
  await catalog.getByRole("textbox").fill("absent-model");
  await catalog.getByText("没有匹配的已配置模型。", { exact: true }).waitFor();
  await catalog.getByRole("textbox").fill("kiro");
  await catalog.locator("summary").waitFor();
  if (captureDir) { await routes.scrollIntoViewIfNeeded(); await page.screenshot({ path: path.join(captureDir, "routing-diagnostics.png"), fullPage: true }); }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await routes.scrollIntoViewIfNeeded();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true, "diagnostics should not create horizontal page overflow");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "routing-diagnostics-mobile-dark.png"), fullPage: true });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS routing trace exposes requested/sent values, failover is opt-in, and capability sources survive stale cache");
}
