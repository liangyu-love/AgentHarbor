import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// The library as the fixture backend last saved it.
const saved = page => page.evaluate(() => window.review.promptLibrary);
const titles = async (page, group = 0) => (await saved(page)).groups[group].templates.map(template => template.title);

/** Drag with real pointer moves, as the WebView sees it. */
async function drag(page, from, to) {
  const start = await from.boundingBox();
  const end = await to.boundingBox();
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
  await page.mouse.down();
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2 + 8, { steps: 2 });
  await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 8 });
  await page.mouse.up();
}

export async function testImagePrompts({ page, reset, captureDir }) {
  await reset("image");
  const panel = page.locator("[data-prompt-library]");
  await panel.getByText("画谱还是空白的", { exact: false }).waitFor();
  const mainPrompt = page.locator('textarea[placeholder^="描述画面主体"]').first();
  const card = title => page.locator(`[data-prompt-template="${title}"]`);
  const tab = name => page.locator(`[data-prompt-group="${name}"]`);
  const capture = async (name, target = page) => {
    if (captureDir) await target.screenshot({ path: path.join(captureDir, `image-prompts-${name}.png`), animations: "disabled" });
  };
  await capture("empty", panel);

  // Groups are created inline in the tab bar.
  await panel.getByRole("button", { name: "新建分组" }).click();
  await page.getByRole("textbox", { name: "分组名称" }).fill("人像");
  await page.keyboard.press("Enter");
  await tab("人像").waitFor();
  assert.deepEqual((await saved(page)).groups.map(group => group.name), ["人像"]);
  console.log("PASS a group is created inline and saved");

  // The dialog editor saves with Ctrl+Enter.
  await panel.getByRole("button", { name: "新建模版" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("标题").fill("窗边侧光");
  await dialog.getByLabel("备注").fill("适合半身像");
  await dialog.getByLabel("提示词").fill("窗边侧光人像，胶片质感");
  await capture("dialog");
  await page.keyboard.press("Control+Enter");
  await dialog.waitFor({ state: "detached" });
  await card("窗边侧光").getByText("适合半身像").waitFor();

  // The prompt box saves itself as a template; an empty title comes from the prompt.
  await mainPrompt.fill("海边日落剪影\n逆光，长焦压缩");
  await page.getByRole("button", { name: "存为模版" }).click();
  assert.equal(await dialog.getByLabel("提示词").inputValue(), "海边日落剪影\n逆光，长焦压缩");
  assert.equal((await dialog.getByRole("combobox", { name: "分组" }).textContent()).trim(), "人像");
  await dialog.getByRole("button", { name: "保存模版" }).click();
  await dialog.waitFor({ state: "detached" });
  await card("海边日落剪影").waitFor();
  assert.deepEqual(await titles(page), ["窗边侧光", "海边日落剪影"]);
  console.log("PASS templates are saved from the dialog and from the prompt box");

  // Filling replaces the prompt; pinning moves a template to the top.
  await card("窗边侧光").getByRole("button", { name: "填入提示词" }).click();
  assert.equal(await mainPrompt.inputValue(), "窗边侧光人像，胶片质感");
  await card("海边日落剪影").getByRole("button", { name: "置顶" }).click();
  await eventually(async () => assert.deepEqual(await titles(page), ["海边日落剪影", "窗边侧光"]));
  assert.equal((await saved(page)).groups[0].templates[0].pinned, true);
  // A dragged template stays inside its pinned or unpinned part.
  await drag(page, card("窗边侧光").getByRole("button", { name: /拖动排序/ }), card("海边日落剪影"));
  await page.waitForTimeout(100);
  assert.deepEqual(await titles(page), ["海边日落剪影", "窗边侧光"]);
  await card("海边日落剪影").getByRole("button", { name: "取消置顶" }).click();
  await eventually(async () => assert.equal((await saved(page)).groups[0].templates[0].pinned, false));
  await drag(page, card("窗边侧光").getByRole("button", { name: /拖动排序/ }), card("海边日落剪影"));
  await eventually(async () => assert.deepEqual(await titles(page), ["窗边侧光", "海边日落剪影"]));
  // Alt + arrow keys reorder from the keyboard.
  await card("窗边侧光").getByRole("button", { name: /拖动排序/ }).focus();
  await page.keyboard.press("Alt+ArrowDown");
  await eventually(async () => assert.deepEqual(await titles(page), ["海边日落剪影", "窗边侧光"]));
  console.log("PASS fill, pin and pointer/keyboard reordering keep pinned templates first");

  // Inline editing saves on the card.
  await card("窗边侧光").hover();
  await card("窗边侧光").getByRole("button", { name: "在卡片上编辑" }).click();
  const inline = page.locator('[data-prompt-template-editor="窗边侧光"]');
  await inline.getByLabel("提示词").fill("窗边侧光人像，柔和阴影");
  await capture("inline", panel);
  await page.keyboard.press("Control+Enter");
  await inline.waitFor({ state: "detached" });
  await eventually(async () => assert.equal((await saved(page)).groups[0].templates[1].prompt, "窗边侧光人像，柔和阴影"));

  // Inline edits carry into the dialog, and closing it with changes asks first.
  await card("窗边侧光").hover();
  await card("窗边侧光").getByRole("button", { name: "在卡片上编辑" }).click();
  await inline.getByLabel("提示词").fill("未保存的改动");
  await inline.getByRole("button", { name: "在弹窗中继续编辑" }).click();
  assert.equal(await dialog.getByLabel("提示词").inputValue(), "未保存的改动");
  await page.keyboard.press("Escape");
  const alert = page.getByRole("alertdialog");
  await alert.getByText("放弃未保存的修改？").waitFor();
  await alert.getByRole("button", { name: "放弃修改", exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  assert.equal((await saved(page)).groups[0].templates[1].prompt, "窗边侧光人像，柔和阴影");
  console.log("PASS inline and dialog editing save, and unsaved dialog edits are confirmed before closing");

  // Groups rename on double-click and reorder by dragging their tabs.
  await panel.getByRole("button", { name: "新建分组" }).click();
  await page.getByRole("textbox", { name: "分组名称" }).fill("海报");
  await page.keyboard.press("Enter");
  await tab("海报").waitFor();
  assert.equal(await tab("海报").getAttribute("aria-selected"), "true");
  await panel.getByText("这个分组还没有模版", { exact: false }).waitFor();
  await tab("人像").dblclick();
  const rename = page.getByRole("textbox", { name: "分组名称" });
  await rename.fill("海报");
  await page.keyboard.press("Enter");
  assert.equal(await rename.getAttribute("aria-invalid"), "true");
  await rename.fill("人物");
  await page.keyboard.press("Enter");
  await tab("人物").waitFor();
  await drag(page, tab("海报"), tab("人物"));
  await eventually(async () => assert.deepEqual((await saved(page)).groups.map(group => group.name), ["海报", "人物"]));
  console.log("PASS groups rename inline, refuse duplicate names and reorder by dragging");

  // Search spans every group and jumps back to a result's group.
  await panel.getByRole("textbox", { name: "检索画谱" }).fill("日落");
  await card("海边日落剪影").waitFor();
  assert.equal(await page.locator("[data-prompt-template]").count(), 1);
  await capture("search", panel);
  await card("海边日落剪影").getByRole("button", { name: "人物" }).click();
  assert.equal(await panel.getByRole("textbox", { name: "检索画谱" }).inputValue(), "");
  assert.equal(await tab("人物").getAttribute("aria-selected"), "true");
  await capture("panel", panel);

  // Deleting asks first; a failed save shows an error and reloads what is on disk.
  await card("窗边侧光").hover();
  await card("窗边侧光").getByRole("button", { name: "删除模版" }).click();
  await alert.getByRole("button", { name: "删除模版", exact: true }).click();
  await card("窗边侧光").waitFor({ state: "detached" });
  assert.deepEqual(await titles(page, 1), ["海边日落剪影"]);
  await page.evaluate(() => { window.review.failPromptSave = true; });
  await panel.getByRole("button", { name: "删除分组" }).click();
  await alert.getByRole("button", { name: "删除分组", exact: true }).click();
  await page.waitForFunction(() => window.review.lastToast === "画谱保存失败");
  assert.match(await page.evaluate(() => window.review.lastToastDescription), /无法读写画谱文件/);
  await tab("人物").waitFor();
  await page.evaluate(() => { window.review.failPromptSave = false; });
  await panel.getByRole("button", { name: "删除分组" }).click();
  await alert.getByRole("button", { name: "删除分组", exact: true }).click();
  await tab("人物").waitFor({ state: "detached" });
  assert.deepEqual((await saved(page)).groups.map(group => group.name), ["海报"]);
  console.log("PASS deleting templates and groups confirms first, and a failed save rolls back");

  // The dialog can file a template under a group it creates.
  await mainPrompt.fill("雪山晨光，航拍");
  await page.getByRole("button", { name: "存为模版" }).click();
  await dialog.getByRole("combobox", { name: "分组" }).click();
  await page.getByRole("option", { name: "＋ 新建分组…" }).click();
  await dialog.getByRole("textbox", { name: "新分组名称" }).fill("海报");
  await dialog.getByText("已有同名分组，请直接选择它").waitFor();
  assert.equal(await dialog.getByRole("button", { name: "保存模版" }).isDisabled(), true);
  await dialog.getByRole("textbox", { name: "新分组名称" }).fill("风景");
  await capture("dialog-new-group");
  await dialog.getByRole("button", { name: "保存模版" }).click();
  await dialog.waitFor({ state: "detached" });
  await eventually(async () => {
    const { groups } = await saved(page);
    assert.deepEqual(groups.map(group => group.name), ["海报", "风景"]);
    assert.deepEqual(groups[1].templates.map(template => template.title), ["雪山晨光，航拍"]);
  });
  console.log("PASS the dialog creates a group for a new template and refuses a taken name");

  // The materials book sits beside the templates; a material joins the end of the prompt.
  await reset("image");
  await mainPrompt.fill("雨夜少女");
  await panel.getByRole("tab", { name: "素材" }).click();
  await card("JK 制服").getByRole("button", { name: "追加到提示词" }).click();
  assert.equal(await mainPrompt.inputValue(), "雨夜少女，水手服，百褶裙");
  await card("JK 制服").getByRole("button", { name: "已在提示词中" }).waitFor();
  await capture("materials", panel);
  await tab("视角").click();
  await panel.getByText("这个分类还没有素材", { exact: false }).waitFor();
  await panel.getByRole("button", { name: "新建素材" }).click();
  const materialDialog = page.getByRole("dialog", { name: "新建素材" });
  await materialDialog.getByRole("textbox", { name: "内容" }).fill("低机位仰拍，天空作背景");
  await materialDialog.getByRole("button", { name: "保存素材" }).click();
  await materialDialog.waitFor({ state: "detached" });
  await eventually(async () => assert.deepEqual(
    await page.evaluate(() => window.review.promptMaterials.groups[2].templates.map(template => template.prompt)),
    ["低机位仰拍，天空作背景"],
  ));
  assert.equal(await page.evaluate(() => window.review.promptSaves), 0, "the templates were not touched");
  await panel.getByRole("tab", { name: "提示词灵感" }).click();
  await panel.getByText("画谱还是空白的", { exact: false }).waitFor();
  console.log("PASS the materials book keeps its own categories and appends a material to the prompt");
}
