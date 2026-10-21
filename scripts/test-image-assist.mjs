import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

const requests = page => page.evaluate(() => window.review.assistRequests.map(({ model, prompt, request }) => ({ model, prompt, request })));
const answer = (page, index, text) => page.evaluate(([at, reply]) => window.review.assistRequests[at].resolve(reply), [index, text]);
const fail = (page, index, message) => page.evaluate(([at, reason]) => window.review.assistRequests[at].reject(reason), [index, message]);

export async function testImageAssist({ page, reset, captureDir }) {
  await reset("image");
  const mainPrompt = page.locator('textarea[placeholder^="描述画面主体"]').first();
  const panel = page.locator("[data-prompt-assist]");
  const modelSelect = panel.getByRole("combobox", { name: "润色模型" });
  await mainPrompt.fill("雨夜橘猫");
  await page.getByRole("button", { name: "AI 润色", exact: true }).click();
  await eventually(async () => assert.match(await modelSelect.textContent(), /GLM 5 · Relay A/));

  // Models come grouped by where they come from; the pick is remembered.
  await modelSelect.click();
  for (const group of ["中转", "账号", "聚合"]) await page.getByRole("group", { name: group }).waitFor();
  await page.getByRole("option", { name: /GPT-5\.5/ }).click();
  await page.waitForFunction(() => window.review.promptAssist.settings.model === "codex/gpt-5.5");

  // A one-off requirement travels with the prompt; the answer is shown, not applied.
  await panel.getByRole("textbox", { name: "额外要求" }).fill("赛博朋克");
  await page.keyboard.press("Enter");
  await panel.getByText("正在请求 GPT-5.5", { exact: false }).waitFor();
  assert.deepEqual(await requests(page), [{ model: "codex/gpt-5.5", prompt: "雨夜橘猫", request: "赛博朋克" }]);
  await answer(page, 0, "霓虹雨夜里的橘猫，赛博朋克街景");
  const result = panel.getByRole("textbox", { name: "润色结果" });
  await eventually(async () => assert.equal(await result.inputValue(), "霓虹雨夜里的橘猫，赛博朋克街景"));
  assert.equal(await mainPrompt.inputValue(), "雨夜橘猫");
  if (captureDir) await panel.screenshot({ path: path.join(captureDir, "image-assist-result.png"), animations: "disabled" });

  // The result can be edited before it is adopted, and adopting can be undone.
  await result.fill("霓虹雨夜里的橘猫，赛博朋克街景，电影感");
  await panel.getByRole("button", { name: "采用", exact: true }).click();
  assert.equal(await mainPrompt.inputValue(), "霓虹雨夜里的橘猫，赛博朋克街景，电影感");
  await panel.getByRole("button", { name: "撤销" }).click();
  assert.equal(await mainPrompt.inputValue(), "雨夜橘猫");
  console.log("PASS AI polish sends the chosen model and requirement, then adopts an edited result with undo");

  // A failure shows the router's own words and can be retried; a cancelled answer is dropped.
  await panel.getByRole("button", { name: "润色", exact: true }).click();
  await fail(page, 1, "unknown model; it may have been removed or turned off since the client was configured");
  await panel.getByRole("alert").getByText("润色失败：unknown model", { exact: false }).waitFor();
  await panel.getByRole("alert").getByRole("button", { name: "重试" }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 3));
  await panel.getByRole("button", { name: "取消", exact: true }).click();
  await answer(page, 2, "迟到的结果");
  await page.waitForTimeout(100);
  assert.equal(await panel.getByRole("textbox", { name: "润色结果" }).count(), 0);
  console.log("PASS AI polish failures can be retried and cancelled answers are ignored");

  // The instruction is the user's to change, and can go back to the default.
  await panel.getByRole("button", { name: "润色指令" }).click();
  const dialog = page.getByRole("dialog", { name: "润色指令" });
  const instruction = dialog.getByRole("textbox", { name: "润色指令" });
  assert.equal(await instruction.inputValue(), "默认润色指令");
  await instruction.fill("只输出英文提示词");
  await page.keyboard.press("Control+Enter");
  await dialog.waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.review.promptAssist.settings.instruction), "只输出英文提示词");
  await panel.getByRole("button", { name: "自定义指令" }).click();
  assert.equal(await instruction.inputValue(), "只输出英文提示词");
  await dialog.getByRole("button", { name: "恢复默认" }).click();
  await dialog.getByRole("button", { name: "保存指令" }).click();
  await dialog.waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.review.promptAssist.settings.instruction), "");
  await panel.getByRole("button", { name: "润色指令" }).waitFor();
  console.log("PASS the polishing instruction can be customised and restored to the default");

  // A loaded reference goes along by default as a shrunk copy; leaving it out is remembered.
  const large = await page.evaluate(() => {
    const canvas = Object.assign(document.createElement("canvas"), { width: 2400, height: 1200 });
    canvas.getContext("2d").fillRect(0, 0, 10, 10);
    return canvas.toDataURL("image/png").split(",")[1];
  });
  await page.locator("[data-image-reference] input[type=file]")
    .setInputFiles({ name: "reference.png", mimeType: "image/png", buffer: Buffer.from(large, "base64") });
  const attach = panel.getByRole("checkbox", { name: "附带参考图" });
  await eventually(async () => assert.equal(await attach.getAttribute("aria-checked"), "true"));
  await panel.getByRole("button", { name: "润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 4));
  const sent = await page.evaluate(() => window.review.assistRequests[3].references[0].data);
  assert.match(sent, /^data:image\/jpeg;base64,/);
  const size = await page.evaluate(async src => {
    const image = new Image();
    image.src = src;
    await image.decode();
    return [image.naturalWidth, image.naturalHeight];
  }, sent);
  assert.deepEqual(size, [1568, 784]);
  await answer(page, 3, "保留橘猫和构图，把背景换成霓虹雨夜");
  await panel.getByText("已对照底本", { exact: false }).waitFor();
  if (captureDir) await panel.screenshot({ path: path.join(captureDir, "image-assist-reference.png"), animations: "disabled" });
  await attach.click();
  await page.waitForFunction(() => window.review.promptAssist.settings.skipReference === true);
  await panel.getByRole("button", { name: "重新润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 5));
  assert.deepEqual(await page.evaluate(() => window.review.assistRequests[4].references), []);
  console.log("PASS AI polish sends a shrunk reference by default and remembers when it is left out");

  // Material categories go along only once ticked, and the ticks are remembered.
  const materials = panel.locator("[data-prompt-assist-materials]");
  const outfits = materials.getByRole("checkbox", { name: "参考素材：服装", exact: true });
  await outfits.waitFor();
  assert.equal(await materials.getByRole("checkbox").count(), 2, "an empty category is not offered");
  assert.deepEqual(await page.evaluate(() => window.review.assistRequests[4].materials), [], "nothing goes along unticked");
  await answer(page, 4, "保留橘猫，背景换成霓虹雨夜");
  await materials.getByRole("checkbox", { name: "参考素材：动作", exact: true }).click();
  await outfits.click();
  await page.waitForFunction(() => window.review.promptAssist.settings.materials.length === 2);
  await materials.getByText(/约 \d+ 字/).waitFor();
  await panel.getByRole("button", { name: "重新润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 6));
  assert.deepEqual(await page.evaluate(() => window.review.assistRequests[5].materials), ["m-outfit", "m-pose"], "in library order");
  await answer(page, 5, "雨夜里回眸的少女，水手服");
  await panel.getByText("参考了服装、动作素材", { exact: false }).waitFor();
  if (captureDir) await panel.screenshot({ path: path.join(captureDir, "image-assist-materials.png"), animations: "disabled" });
  await outfits.click();
  await page.waitForFunction(() => JSON.stringify(window.review.promptAssist.settings.materials) === '["m-pose"]');
  await panel.getByRole("button", { name: "重新润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 7));
  assert.deepEqual(await page.evaluate(() => window.review.assistRequests[6].materials), ["m-pose"]);
  await answer(page, 6, "回眸的少女");
  await materials.getByRole("button", { name: "清空" }).click();
  await page.waitForFunction(() => window.review.promptAssist.settings.materials.length === 0);
  console.log("PASS AI polish sends only the ticked material categories and remembers them");

  // A category opens to tick single materials: only those go along, and ticking all of them is the whole category.
  await materials.getByRole("button", { name: "挑选服装里的素材" }).click();
  const outfitItems = materials.locator('[data-prompt-assist-category="服装"]');
  await outfitItems.getByRole("checkbox", { name: "参考素材：服装 · 老钱静奢" }).click();
  await page.waitForFunction(() => JSON.stringify(window.review.promptAssist.settings.materials) === '["m3"]');
  await eventually(async () => assert.equal(await outfits.getAttribute("aria-checked"), "mixed"));
  await materials.getByText("1/2").waitFor();
  await materials.getByRole("checkbox", { name: "参考素材：动作", exact: true }).click();
  await panel.getByRole("button", { name: "重新润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 8));
  assert.deepEqual(await page.evaluate(() => window.review.assistRequests[7].materials), ["m3", "m-pose"], "single materials in library order");
  await answer(page, 7, "回眸的少女，米白真丝衬衫");
  await panel.getByText("参考了服装（1 条）、动作素材", { exact: false }).waitFor();
  if (captureDir) await panel.screenshot({ path: path.join(captureDir, "image-assist-material-items.png"), animations: "disabled" });
  await outfitItems.getByRole("checkbox", { name: "参考素材：服装 · JK 制服" }).click();
  await page.waitForFunction(() => JSON.stringify(window.review.promptAssist.settings.materials) === '["m-pose","m-outfit"]');
  await eventually(async () => assert.equal(await outfits.getAttribute("aria-checked"), "true"));
  await outfitItems.getByRole("button", { name: "全不选" }).click();
  await page.waitForFunction(() => JSON.stringify(window.review.promptAssist.settings.materials) === '["m-pose"]');
  await materials.getByRole("button", { name: "清空" }).click();
  await page.waitForFunction(() => window.review.promptAssist.settings.materials.length === 0);
  console.log("PASS a material category opens to send only the materials ticked in it");

  // The materials are one click away from the polishing panel.
  await materials.getByRole("button", { name: "管理素材" }).click();
  const library = page.locator("[data-prompt-library]");
  await eventually(async () => assert.equal(await library.getByRole("tab", { name: "素材" }).getAttribute("aria-selected"), "true"));
  await page.locator('[data-prompt-template="JK 制服"]').waitFor();
  await library.getByRole("tab", { name: "提示词灵感" }).click();
  console.log("PASS managing materials opens the materials book of the 画谱");

  // Pictures from the image book join the references after the uploaded one, named for the polishing model.
  await library.getByRole("tab", { name: "图片" }).click();
  const girl = page.locator('[data-image-material="短发女生"]');
  await girl.getByRole("button", { name: "短发女生：加入参考图" }).click();
  const strip = page.locator("[data-image-reference]");
  await eventually(async () => assert.equal(await strip.locator("[data-reference]").count(), 2));
  await strip.locator('[data-reference="图2"]').getByText("短发女生").waitFor();
  await girl.getByText("图2").waitFor();
  if (captureDir) await strip.screenshot({ path: path.join(captureDir, "image-references.png"), animations: "disabled" });
  await attach.click();
  await page.waitForFunction(() => window.review.promptAssist.settings.skipReference === false);
  await panel.getByRole("button", { name: "重新润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 9));
  const references = await page.evaluate(() => window.review.assistRequests[8].references.map(({ name, note }) => ({ name, note })));
  assert.deepEqual(references, [{ name: "", note: "" }, { name: "短发女生", note: "只参考脸和发型" }]);
  const firstSize = await page.evaluate(async () => {
    const image = new Image();
    image.src = window.review.assistRequests[8].references[0].data;
    await image.decode();
    return [image.naturalWidth, image.naturalHeight];
  });
  assert.deepEqual(firstSize, [1024, 512], "several references go at a smaller size");
  await answer(page, 8, "图1的橘猫换上图2女生的发型");
  await panel.getByText("已对照 2 张参考图", { exact: false }).waitFor();
  console.log("PASS AI polish sends every reference in order, with the image material's name and note");

  // Generating sends the references in their order; dragging with the keyboard reorders them.
  await panel.getByRole("button", { name: "采用", exact: true }).click();
  await strip.locator('[data-reference="图1"]').getByRole("button", { name: /^图1/ }).press("Alt+ArrowRight");
  await strip.locator('[data-reference="图1"]').getByText("短发女生").waitFor();
  const before = await page.evaluate(() => window.review.imageRequests.length);
  await page.locator("[data-image-generate]").click();
  await page.waitForFunction(count => window.review.imageRequests.length === count + 1, before);
  const images = await page.evaluate(() => window.review.imageRequests.at(-1).images);
  assert.equal(images.length, 2);
  assert.equal(images[0], await page.evaluate(() => window.review.imageMaterialData["img-girl"]));
  await page.evaluate(() => window.review.imageResolvers.at(-1)());
  // Clicking the picture again takes it back out.
  await girl.getByRole("button", { name: "短发女生：已是图1，点击移出参考图" }).click();
  await eventually(async () => assert.equal(await strip.locator("[data-reference]").count(), 1));
  console.log("PASS references are generated in order, reordered from the keyboard, and toggled from the image book");

  // Pictures are added to a group by file, renamed with a note, and deleted.
  await library.locator('input[aria-label="添加图片素材"]').setInputFiles([
    { name: "碎花裙.png", mimeType: "image/png", buffer: Buffer.from(large, "base64") },
  ]);
  const dress = page.locator('[data-image-material="碎花裙"]');
  await dress.waitFor();
  if (captureDir) await library.screenshot({ path: path.join(captureDir, "image-materials-book.png"), animations: "disabled" });
  assert.deepEqual(await page.evaluate(() => window.review.imageMaterials.groups[0].items.map(item => item.name)), ["短发女生", "碎花裙"]);
  await dress.hover();
  await dress.getByRole("button", { name: "编辑「碎花裙」" }).click();
  const editDialog = page.getByRole("dialog", { name: "图片素材" });
  await editDialog.getByRole("textbox", { name: "图片备注" }).fill("只参考裙子");
  await editDialog.getByRole("button", { name: "保存" }).click();
  await editDialog.waitFor({ state: "detached" });
  await page.waitForFunction(() => window.review.imageMaterials.groups[0].items[1].note === "只参考裙子");
  await dress.hover();
  await dress.getByRole("button", { name: "删除「碎花裙」" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "删除图片" }).click();
  await dress.waitFor({ state: "detached" });
  assert.equal(await page.evaluate(() => window.review.imageMaterials.groups[0].items.length), 1);
  await strip.getByRole("button", { name: "移除图1" }).click();
  await library.getByRole("tab", { name: "提示词灵感" }).click();
  console.log("PASS image materials are added, annotated and deleted in the image book");

  // A busy photo is sent smaller, within the budget, instead of past a relay's input limit.
  const shrunk = await page.evaluate(async () => {
    const canvas = Object.assign(document.createElement("canvas"), { width: 2400, height: 1600 });
    const context = canvas.getContext("2d");
    const noise = context.createImageData(canvas.width, canvas.height);
    for (let i = 0; i < noise.data.length; i += 1) noise.data[i] = (i * 2654435761 >>> 13) & 255;
    context.putImageData(noise, 0, 0);
    const data = await window.review.shrinkReference(canvas.toDataURL("image/png"), 1568, 200_000);
    const image = new Image();
    image.src = data;
    await image.decode();
    return { length: data.length, width: image.naturalWidth };
  });
  assert.ok(shrunk.length <= 200_000, `the copy fits the budget: ${shrunk.length}`);
  assert.ok(shrunk.width < 1568 && shrunk.width >= 512, `only as small as it must be: ${shrunk.width}`);
  console.log("PASS a busy reference is shrunk to fit the polishing budget");
}
