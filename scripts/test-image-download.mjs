import assert from "node:assert/strict";

export async function testImageDownload({ page, reset }) {
  await reset("image");
  const prompt = page.locator('textarea[placeholder^="描述画面主体"]');
  await prompt.waitFor();
  // Previous tests may have saved a manual endpoint. Choose the provider as a
  // user would instead of depending on the persisted default selection.
  await page.getByRole("combobox", { name: "图像供应商" }).click();
  await page.getByRole("option", { name: "Provider image", exact: true }).click();
  await prompt.fill("JPEG result");
  await page.locator("[data-image-generate]").click();
  await page.waitForFunction(() => window.review.imageResolvers.length === 1);
  await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 2;
    window.review.imageResolvers[0]([canvas.toDataURL("image/jpeg")]);
  });
  await page.locator('[data-generation-job-status="completed"]').waitFor();
  const saveLabel = await page.evaluate(() => window.review.t("image.saveAs"));
  await page.evaluate(() => {
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      if (command !== "plugin:dialog|save") throw new Error(`Unexpected IPC: ${command}`);
      window.review.imageSaveOptions = args.options;
      return "C:/synthetic/export.jpg";
    };
  });
  await page.getByRole("button", { name: saveLabel, exact: true }).click();
  await page.waitForFunction(() => window.review.imageFileSaves?.length === 1);
  const saved = await page.evaluate(() => ({ options: window.review.imageSaveOptions, image: window.review.imageFileSaves[0] }));
  assert.match(saved.options.defaultPath, /\.jpg$/);
  assert.deepEqual(saved.options.filters, [{ name: "JPG", extensions: ["jpg"] }]);
  assert.match(saved.image.dataUrl, /^data:image\/jpeg;base64,/);
  await page.evaluate(() => { delete window.__TAURI_INTERNALS__; });
  const downloadReady = page.waitForEvent("download");
  await page.getByRole("button", { name: saveLabel, exact: true }).click();
  const download = await downloadReady;
  assert.match(download.suggestedFilename(), /\.jpg$/);
  assert.equal(await download.failure(), null);
  console.log("PASS JPEG save dialog and browser download use the actual image format");
}
