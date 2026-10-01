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
  // The applied state distinguishes what clients already use from a pending sync.
  assert.match(await catalog.innerText(), /已生效/);
  assert.match(await catalog.innerText(), /阈值 900,000/);
  // Manual price overrides are saved per model and estimated cost only.
  await catalog.getByRole("button", { name: "编辑价格", exact: true }).click();
  await catalog.getByLabel("输入", { exact: true }).fill("1.5");
  await catalog.getByLabel("输出", { exact: true }).fill("7.5");
  await catalog.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => (window.review.priceSaves ?? []).length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.priceSaves[0]), {
    providerId: "relay",
    modelId: "claude-opus-5-5-kiro",
    price: { input: 1.5, output: 7.5, cacheRead: null, cacheWrite: null },
  });
  await catalog.getByRole("textbox").fill("absent-model");
  await catalog.getByText("没有匹配的已配置模型。", { exact: true }).waitFor();
  await catalog.getByRole("textbox").fill("kiro");
  await catalog.locator("summary").waitFor();
  if (captureDir) { await routes.scrollIntoViewIfNeeded(); await page.screenshot({ path: path.join(captureDir, "routing-diagnostics.png"), fullPage: true }); }
  for (const dark of [false, true]) {
    await page.evaluate(dark => {
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.setAttribute("data-has-custom-bg", "true");
    }, dark);
    // These panels animate their surface color; inspect the settled wallpaper
    // surface rather than the first frame of the transition.
    await page.waitForFunction(() => [...document.querySelectorAll(
      'section[aria-label="请求级路由诊断"], section[aria-label="模型能力目录与匹配预览"]',
    )].every(element => /, 0\.82\)$/.test(getComputedStyle(element).backgroundColor)));
    for (const panel of [routes, catalog]) {
      const surfaces = await panel.evaluate(element => [element, ...element.querySelectorAll("details")]
        .map(surface => getComputedStyle(surface).backgroundColor));
      assert.ok(surfaces.every(color => /, 0\.82\)$/.test(color)),
        `Wallpaper surfaces retain their alpha in ${dark ? "dark" : "light"} mode: ${surfaces}`);
    }
  }
  await page.evaluate(() => document.documentElement.removeAttribute("data-has-custom-bg"));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await routes.scrollIntoViewIfNeeded();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true, "diagnostics should not create horizontal page overflow");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "routing-diagnostics-mobile-dark.png"), fullPage: true });
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 700 });
  console.log("PASS routing trace exposes requested/sent values, failover is opt-in, and capability sources survive stale cache");
}
