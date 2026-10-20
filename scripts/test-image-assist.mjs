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
  const attach = panel.getByRole("checkbox", { name: "附带底本" });
  await eventually(async () => assert.equal(await attach.getAttribute("aria-checked"), "true"));
  await panel.getByRole("button", { name: "润色", exact: true }).click();
  await eventually(async () => assert.equal((await requests(page)).length, 4));
  const sent = await page.evaluate(() => window.review.assistRequests[3].reference);
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
  assert.equal(await page.evaluate(() => window.review.assistRequests[4].reference), null);
  console.log("PASS AI polish sends a shrunk reference by default and remembers when it is left out");

  // Material categories go along only once ticked, and the ticks are remembered.
  const materials = panel.locator("[data-prompt-assist-materials]");
  const outfits = materials.getByRole("checkbox", { name: "参考素材：服装" });
  await outfits.waitFor();
  assert.equal(await materials.getByRole("checkbox").count(), 2, "an empty category is not offered");
  assert.deepEqual(await page.evaluate(() => window.review.assistRequests[4].materials), [], "nothing goes along unticked");
  await answer(page, 4, "保留橘猫，背景换成霓虹雨夜");
  await materials.getByRole("checkbox", { name: "参考素材：动作" }).click();
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

  // The materials are one click away from the polishing panel.
  await materials.getByRole("button", { name: "管理素材" }).click();
  const library = page.locator("[data-prompt-library]");
  await eventually(async () => assert.equal(await library.getByRole("tab", { name: "素材" }).getAttribute("aria-selected"), "true"));
  await page.locator('[data-prompt-template="JK 制服"]').waitFor();
  await library.getByRole("tab", { name: "提示词灵感" }).click();
  console.log("PASS managing materials opens the materials book of the 画谱");
}
