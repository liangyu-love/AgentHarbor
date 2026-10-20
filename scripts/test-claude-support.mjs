import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testClaudeSupport({ page, reset, captureDir }) {
  const label = (key) => page.evaluate((key) => window.review.t(key), key);
  const radio = (name) =>
    page.getByRole("radio", { name, exact: true, includeHidden: true });
  const button = async (key) =>
    page.getByRole("button", { name: await label(key), exact: true });
  const capture = async (name) => {
    if (captureDir)
      await page.screenshot({
        path: path.join(captureDir, name),
        animations: "disabled",
      });
  };

  await reset("instructions");
  const editor = () => page.locator("textarea[id^=instruction-editor-]");
  const currentFile = () => page.locator("[data-instruction-current]");
  await editor().waitFor();
  await eventually(async () => assert.equal(await editor().inputValue(), "codex rules"));
  assert.match(await currentFile().textContent(), /User rules[\s\S]*codex rules/);
  await radio("Claude Code").click();
  await page
    .getByText("C:/synthetic/.claude/CLAUDE.md", { exact: true })
    .waitFor();
  await eventually(async () => assert.equal(await editor().inputValue(), "claudeCode rules"));
  await (await button("customInstructions.chooseTemplate")).click();
  const card = page.locator('[data-instruction-template="implementation"]');
  await card
    .getByRole("button", {
      name: await label("customInstructions.loadToEditor"),
      exact: true,
    })
    .click();
  await editor().waitFor();
  await eventually(async () => assert.match(await editor().inputValue(), /阅读项目指令/));
  await editor().fill("My Claude instructions");
  await eventually(async () => assert.match(await currentFile().textContent(), /claudeCode rules/));
  assert.doesNotMatch(await currentFile().textContent(), /My Claude instructions/);
  await page
    .getByRole("textbox", {
      name: await label("customInstructions.presetName"),
    })
    .fill("My Claude preset");
  await (await button("customInstructions.savePreset")).click();
  await (await button("customInstructions.previewAndApply")).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByText("C:/synthetic/.claude/CLAUDE.md", { exact: true })
    .waitFor();
  await dialog
    .getByText(await label("customInstructions.resultingFile"), { exact: true })
    .click();
  assert.match(
    await dialog
      .getByRole("textbox", {
        name: await label("customInstructions.resultingFile"),
      })
      .inputValue(),
    /User rules/,
  );
  await page.evaluate(() => {
    window.review.instructionApplyHold = true;
  });
  await dialog
    .getByRole("button", {
      name: await label("customInstructions.confirmApply"),
      exact: true,
    })
    .click();
  await page.waitForFunction(() => !!window.review.instructionApplyResolve);
  await eventually(async () => assert.equal(await radio("Codex").isDisabled(), true));
  await page.evaluate(() => {
    window.review.instructionApplyHold = false;
    window.review.instructionApplyResolve();
  });
  await dialog.waitFor({ state: "hidden" });
  await eventually(async () => assert.match(await currentFile().textContent(), /User rules[\s\S]*My Claude instructions/));
  const applied = await page.evaluate(() =>
    window.review.instructionCalls.find((call) => call.operation === "apply"),
  );
  assert.equal(applied.target, "claudeCode");
  assert.equal(applied.expectedContent, "claudeCode rules");
  await radio("Codex").click();
  await eventually(async () => assert.equal(await editor().inputValue(), "codex rules"));
  await (await button("customInstructions.chooseTemplate")).click();
  await (await button("customInstructions.categories.saved")).click();
  assert.equal(
    await page.getByRole("heading", { name: "My Claude preset" }).count(),
    0,
  );
  await radio("Claude Code").click();
  await eventually(async () => assert.equal(await editor().inputValue(), "My Claude instructions"));
  await editor().fill("Preserved scratch draft");
  await radio("Codex").click();
  await radio("Claude Code").click();
  await eventually(async () => assert.equal(await editor().inputValue(), "Preserved scratch draft"));
  assert.doesNotMatch(await currentFile().textContent(), /Preserved scratch draft/);
  await (await button("customInstructions.chooseTemplate")).click();
  await (await button("customInstructions.categories.saved")).click();
  await page
    .getByRole("heading", { name: "My Claude preset", exact: true })
    .waitFor();
  await capture("claude-instruction-presets-desktop.png");
  await radio(await label("customInstructions.configureTab")).click();
  await (await button("customInstructions.clearManagedBlock")).click();
  await page
    .getByRole("alertdialog")
    .getByRole("button", {
      name: await label("customInstructions.clearManagedBlock"),
      exact: true,
    })
    .click();
  await page.getByRole("alertdialog").waitFor({ state: "hidden" });
  await eventually(async () => assert.equal(await currentFile().textContent(), "User rules\n"));
  await page.evaluate(() => {
    const current = window.review.instructionStates.claudeCode.current;
    current.fileContent = '<script>external rules</script>\n' + 'long-rule-'.repeat(80);
    current.protectionState = "protected";
    current.issueMessage = "Incomplete managed markers";
  });
  await (await button("common.refresh")).click();
  await page.waitForFunction(() => document.querySelector("[data-instruction-current]")?.textContent.includes("external rules"));
  await eventually(async () => assert.equal(await currentFile().locator("script").count(), 0));
  assert.equal(await editor().isDisabled(), true);
  assert.equal(
    (
      await page.evaluate(() =>
        window.review.instructionCalls.filter(
          (call) => call.operation === "clear",
        ),
      )
    ).at(-1).source,
    "claudeCode",
  );
  await page.setViewportSize({ width: 390, height: 820 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(
    await page
      .locator("[data-instruction-workspace]")
      .evaluate((el) => el.scrollWidth <= el.clientWidth),
    true,
  );
  await capture("claude-instructions-mobile-dark.png");
  await page.setViewportSize({ width: 1200, height: 820 });

  console.log(
    "PASS Claude instructions: client isolation, templates, saved presets, draft, preview, apply lock, clear",
  );
}
