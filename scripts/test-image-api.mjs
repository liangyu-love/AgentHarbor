import assert from "node:assert/strict";
import path from "node:path";

export async function testImageApi({ page, reset, captureDir }) {
  await reset("image");
  await page.getByRole("combobox", { name: "图像供应商" }).click();
  await page.getByRole("option", { name: "Provider image", exact: true }).click();
  const model = page.getByPlaceholder("gpt-image-2", { exact: true });
  await model.focus();
  assert.equal(await page.getByRole("option", { name: "gpt-image-2.5", exact: true }).count(), 0);
  await page.getByRole("option", { name: "gpt-image-2.5-flare", exact: true }).waitFor();
  await page.getByRole("option", { name: "gpt-image-2.5-sunburst", exact: true }).click();

  const quality = page.getByRole("combobox", { name: "画质", exact: true });
  await quality.click();
  await page.getByRole("option", { name: /^最高 / }).click();
  await page.getByRole("combobox", { name: "画幅尺寸", exact: true }).click();
  await page.getByRole("option", { name: "16:9 广角 (1536x864)", exact: true }).click();
  await page.getByRole("listbox").waitFor({ state: "hidden" });
  await page.locator('textarea[placeholder^="描述画面主体"]').fill("官方接口参数验证");
  await page.locator("[data-image-generate]").click();
  await page.waitForFunction(() => window.review.imageRequests.length === 1);
  assert.deepEqual(await page.evaluate(() => {
    const { model, size, quality } = window.review.imageRequests[0];
    return { model, size, quality };
  }), { model: "gpt-image-2.5-sunburst", size: "1536x864", quality: "max" });
  await page.evaluate(() => window.review.imageResolvers[0]());
  await page.locator('[data-generation-job-status="completed"]').waitFor();
  console.log("PASS official Sunburst/Flare choices and new quality/size reach image requests");

  await model.click();
  await page.getByRole("option", { name: "gpt-image-2", exact: true }).click();
  assert.equal((await quality.textContent()).trim(), "自动");
  await quality.click();
  assert.equal(await page.getByRole("option", { name: /^最高 / }).count(), 0);
  assert.equal(await page.getByRole("option", { name: /^超高 / }).count(), 0);
  await page.keyboard.press("Escape");
  await page.locator("[data-image-generate]").click();
  await page.waitForFunction(() => window.review.imageRequests.length === 2);
  assert.equal(await page.evaluate(() => window.review.imageRequests[1].quality), "auto");
  await page.evaluate(() => window.review.imageResolvers[1]());
  console.log("PASS switching to older models does not send unsupported extended quality");

  await model.fill("gpt-image-2.5-flare-2026-09-08");
  await model.press("Enter");
  await quality.click();
  await page.getByRole("option", { name: /^超高 / }).click();
  assert.equal((await quality.textContent()).trim(), "超高");
  await page.getByRole("combobox", { name: "画幅尺寸", exact: true }).click();
  await page.getByRole("option", { name: "9:16 宣幅 (864x1536)", exact: true }).click();
  await page.getByRole("listbox").waitFor({ state: "hidden" });
  if (captureDir) {
    const viewport = page.viewportSize();
    await page.screenshot({ path: path.join(captureDir, "image-api-desktop.png"), fullPage: true, animations: "disabled" });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: path.join(captureDir, "image-api-mobile.png"), fullPage: true, animations: "disabled" });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), true);
    await page.setViewportSize(viewport);
  }
  console.log("PASS dated GPT Image 2.5 snapshots expose extended quality and portrait dimensions");
}
