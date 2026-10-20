import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

const transform = viewer => viewer.locator("img").evaluate(element => {
  const match = /translate\((-?[\d.]+)px, (-?[\d.]+)px\) scale\(([\d.]+)\)/.exec(element.style.transform);
  return match ? { x: Number(match[1]), y: Number(match[2]), scale: Number(match[3]) } : null;
});

export async function testImageViewer({ page, reset, captureDir }) {
  await reset("image");
  await page.evaluate(() => { window.review.largeOriginal = true; });
  const mainPrompt = page.locator("[data-image-prompt]");
  await page.getByRole("button", { name: "历史画册", exact: true }).click();
  const card = name => page.locator("[data-gallery-card]").filter({ has: page.locator(`button[title="${name}"]`) });

  // Gallery times come in milliseconds; each fixture image is an hour older than the last.
  await card("gallery-2.png").getByText("2 小时前", { exact: true }).waitFor();
  await card("gallery-30.png").getByText("1 天前", { exact: true }).waitFor();
  assert.match(await card("gallery-2.png").getByText("2 小时前").getAttribute("title"), /^\d{4}\/\d{2}\/\d{2} \d{2}:\d{2}$/);
  console.log("PASS gallery times read the backend's milliseconds");

  // Cards show the prompt that made them, are grouped by day and can be searched.
  await card("gallery-0.png").getByText("窗边侧光人像，胶片质感").waitFor();
  await card("gallery-50.png").getByText("gallery-50.png", { exact: true }).waitFor();
  await page.locator("[data-gallery-section]").getByRole("region", { name: "今天" }).waitFor();
  assert.ok(await page.locator("[data-gallery-section] section").count() >= 4);
  const search = page.getByRole("textbox", { name: "检索画册" });
  await search.fill("橘猫");
  await eventually(async () => assert.equal(await page.locator("[data-gallery-card]").count(), 1));
  await card("gallery-1.png").waitFor();
  await search.fill("flare");
  await eventually(async () => assert.equal(await page.locator("[data-gallery-card]").count(), 1));
  if (captureDir) await page.locator("[data-gallery-section]").screenshot({ path: path.join(captureDir, "image-gallery-search.png"), animations: "disabled" });
  await search.fill("");
  await card("gallery-1.png").hover();
  await card("gallery-1.png").getByRole("button", { name: "填入提示词" }).click();
  assert.equal(await mainPrompt.inputValue(), "雨夜霓虹街头的橘猫");
  await page.waitForFunction(() => window.review.lastToast === "已填入提示词");
  console.log("PASS gallery cards show their prompt, group by day, search, and fill the prompt box");

  // A saved image becomes the next generation's reference in one click.
  const reference = page.locator("[data-image-reference] img");
  await card("gallery-1.png").hover();
  await card("gallery-1.png").getByRole("button", { name: "用作底本" }).click();
  await page.waitForFunction(() => window.review.lastToast === "已载入为底本");
  assert.match(await reference.getAttribute("src"), /^data:image\//);
  await page.getByRole("button", { name: "移除画稿" }).click();
  await reference.waitFor({ state: "detached" });
  console.log("PASS a gallery image loads as the reference for image-to-image");

  await page.locator('button[title="gallery-0.png"]').click();
  const viewer = page.getByRole("dialog", { name: "成图放大预览", exact: true });
  await viewer.waitFor();
  const zoomLabel = viewer.locator("[data-viewer-zoom]");
  // Once measured, a 2400×1600 original fitted into a 700px window shows well under its real size.
  await eventually(async () => assert.notEqual(await zoomLabel.textContent(), "100%"));
  const fit = await zoomLabel.textContent();
  assert.ok(parseInt(fit, 10) < 50, fit);
  const box = await viewer.locator("img").boundingBox();
  const centre = { x: box.x + box.width / 2, y: box.y + box.height / 2 };

  // The wheel zooms at the cursor; dragging pans without closing.
  await page.mouse.move(centre.x + 100, centre.y + 50);
  await page.mouse.wheel(0, -500);
  await eventually(async () => assert.ok((await transform(viewer)).scale > 1.5));
  const zoomed = await transform(viewer);
  await page.mouse.move(centre.x, centre.y);
  await page.mouse.down();
  await page.mouse.move(centre.x - 120, centre.y - 60, { steps: 6 });
  await page.mouse.up();
  const panned = await transform(viewer);
  assert.ok(panned.x < zoomed.x && panned.y < zoomed.y, JSON.stringify({ zoomed, panned }));
  assert.equal(await viewer.count(), 1);
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "image-viewer-zoomed.png"), animations: "disabled" });

  // Toolbar, double-click and keys move between fit and real size.
  await viewer.getByRole("button", { name: "原始尺寸" }).click();
  await eventually(async () => assert.equal(await zoomLabel.textContent(), "100%"));
  await viewer.getByRole("button", { name: "适应窗口" }).click();
  await eventually(async () => assert.deepEqual(await transform(viewer), { x: 0, y: 0, scale: 1 }));
  await viewer.locator("img").dblclick();
  await eventually(async () => assert.equal(await zoomLabel.textContent(), "100%"));
  await page.keyboard.press("0");
  await eventually(async () => assert.equal(await zoomLabel.textContent(), fit));
  await page.keyboard.press("+");
  await eventually(async () => assert.ok(parseInt(await zoomLabel.textContent(), 10) > parseInt(fit, 10)));
  console.log("PASS the preview zooms with the wheel, toolbar, double-click and keys, and pans by dragging");

  // The record travels with the preview, which steps through the gallery.
  await page.keyboard.press("0");
  const info = viewer.getByRole("region", { name: "生成信息" });
  const position = viewer.locator("[data-viewer-position]");
  await info.getByText("窗边侧光人像，胶片质感").waitFor();
  await info.getByText("gpt-image-2", { exact: true }).waitFor();
  await info.getByText("2:3 立轴 · 1024x1536").waitFor();
  await info.getByText("画质 高").waitFor();
  assert.equal(await position.textContent(), "1 / 100");
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "image-viewer-info.png"), animations: "disabled" });
  await page.keyboard.press("ArrowRight");
  await eventually(async () => assert.equal(await position.textContent(), "2 / 100"));
  await info.getByText("雨夜霓虹街头的橘猫").waitFor();
  await info.getByRole("button", { name: "填入提示词" }).click();
  await info.getByRole("button", { name: "已填入" }).waitFor();
  assert.equal(await mainPrompt.inputValue(), "雨夜霓虹街头的橘猫");
  await viewer.getByRole("button", { name: "下一张" }).click();
  await eventually(async () => assert.equal(await position.textContent(), "3 / 100"));
  assert.equal(await info.count(), 0, "an image saved before records has no information card");
  await viewer.getByRole("button", { name: "上一张" }).click();
  await eventually(async () => assert.equal(await position.textContent(), "2 / 100"));
  await info.getByRole("button", { name: "收起生成信息" }).click();
  await viewer.getByRole("button", { name: "提示词", exact: true }).click();
  await info.waitFor();
  console.log("PASS the preview shows how each image was made and steps through the gallery");

  // A click on the picture keeps the preview; a click beside it closes it.
  await viewer.locator("img").click();
  assert.equal(await viewer.count(), 1);
  await page.mouse.click(100, 650);
  await viewer.waitFor({ state: "detached" });
  console.log("PASS clicking the picture keeps the preview open and clicking beside it closes it");

  // A new image is saved with the prompt and parameters that made it.
  await mainPrompt.fill("雪山晨光，航拍");
  await page.locator("[data-image-generate]").click();
  await page.waitForFunction(() => window.review.imageRequests.length === 1);
  await page.evaluate(() => window.review.imageResolvers[0]());
  await page.waitForFunction(() => window.review.galleryMetas?.length === 1);
  // Earlier suites may leave another model or size chosen; the record follows the request.
  const sent = await page.evaluate(() => {
    const { model, size, quality } = window.review.imageRequests[0];
    return { model, size, quality };
  });
  assert.deepEqual(await page.evaluate(() => window.review.galleryMetas[0]), {
    prompt: "雪山晨光，航拍", ...sent, revisedPrompts: [],
  });
  const canvas = page.locator('[data-generation-job-status="completed"]');
  await canvas.getByText(/^画质 /).waitFor();
  await canvas.locator("img").first().click();
  await viewer.waitFor();
  await info.getByText("雪山晨光，航拍").waitFor();
  assert.equal(await position.count(), 0, "a single image has nothing to step to");
  await page.keyboard.press("Escape");
  await viewer.waitFor({ state: "detached" });
  await mainPrompt.fill("");
  await canvas.getByRole("button", { name: "复用这条提示词" }).click();
  assert.equal(await mainPrompt.inputValue(), "雪山晨光，航拍");
  console.log("PASS generated images keep their prompt and parameters, from the canvas to the gallery");

  // A generated image becomes the reference from the canvas or its preview, and the next request edits it.
  const generated = await canvas.locator("img").first().getAttribute("src");
  await canvas.getByRole("button", { name: "用作底本" }).click();
  await eventually(async () => assert.equal(await reference.getAttribute("src"), generated));
  await page.getByRole("button", { name: "移除画稿" }).click();
  await reference.waitFor({ state: "detached" });
  await canvas.locator("img").first().click();
  await viewer.waitFor();
  await viewer.getByRole("button", { name: "用作底本" }).click();
  await viewer.waitFor({ state: "detached" });
  assert.equal(await reference.getAttribute("src"), generated);
  await page.locator("[data-image-generate]").click();
  await page.waitForFunction(() => window.review.imageRequests.length === 2);
  assert.equal(await page.evaluate(() => window.review.imageRequests[1].imageData), generated);
  await page.evaluate(() => window.review.imageResolvers[1]());
  await page.getByRole("button", { name: "移除画稿" }).click();
  console.log("PASS a generated image loads as the reference from the canvas and the preview");

  // Generating again resends the canvas's own request, reference included, in that canvas's place.
  await page.waitForFunction(() => window.review.galleryMetas?.length === 2);
  const jobs = page.locator("[data-generation-job-status]");
  assert.equal(await jobs.count(), 2);
  await jobs.first().getByRole("button", { name: "用相同请求重新生成，替换本次画布" }).click();
  await page.waitForFunction(() => window.review.imageRequests.length === 3);
  const resent = await page.evaluate(() => window.review.imageRequests[2]);
  assert.equal(resent.imageData, generated, "the reference sent then, not the empty slot now");
  assert.equal(resent.prompt, "雪山晨光，航拍");
  assert.equal(await jobs.count(), 2);
  assert.equal(await jobs.first().getAttribute("data-generation-job-status"), "running");
  await page.evaluate(() => window.review.imageResolvers[2]());
  await page.waitForFunction(() => document.querySelectorAll('[data-generation-job-status="completed"]').length === 2);
  console.log("PASS generating a canvas again resends its request and reference in its place");
}
