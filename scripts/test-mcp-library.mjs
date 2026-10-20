import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// The MCP page: one list, a switch per server and agent, the add dialog's
// templates, paste and command splitting, imports and the command palette.
export async function testMcpLibrary({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });
  const calls = (operation) => page.evaluate((operation) => window.review.mcpCalls.filter((call) => call.operation === operation), operation);
  const row = (name) => page.locator(`[data-mcp-server="${name}"]`);
  const cell = (name, app) => row(name).locator(`[data-mcp-cell="${app}"]`);
  const capture = async (name) => {
    if (captureDir) await page.screenshot({ path: path.join(captureDir, name), fullPage: true });
  };

  // ---- the matrix shows each agent's state and refuses what an agent cannot take
  await reset("mcp-clients");
  await row("filesystem").waitFor();
  assert.equal(await cell("filesystem", "codex").getAttribute("aria-checked"), "true");
  assert.equal(await cell("filesystem", "codex").getAttribute("data-state"), "synced");
  assert.equal(await cell("filesystem", "pi").isDisabled(), true, "an agent that is not installed is locked");
  assert.equal(await cell("legacy-sse", "codex").getAttribute("data-state"), "unsupported");
  assert.equal(await cell("legacy-sse", "codex").isDisabled(), true, "Codex cannot take SSE");
  await page.locator("[data-mcp-notes]").getByText(await tr("mcp.unmanaged", { app: "Cursor", count: 1, names: "cursor-only" }), { exact: true }).waitFor();
  await capture("mcp-library-top.png");
  console.log("PASS the MCP matrix shows every agent's state and locks what an agent cannot hold");

  // ---- a switch writes one agent and the page waits for it
  await page.evaluate(() => { window.review.mcpHold = true; });
  await cell("docs", "claudeCode").click();
  await page.waitForFunction(() => !!window.review.mcpResolve);
  await eventually(async () => assert.equal(await cell("filesystem", "codex").isDisabled(), true, "every switch waits for the write"));
  assert.equal(await page.getByRole("button", { name: await tr("mcp.addServer") }).first().isDisabled(), true);
  await page.evaluate(() => { window.review.mcpHold = false; window.review.mcpResolve(); });
  await eventually(async () => assert.equal(await cell("docs", "claudeCode").getAttribute("aria-checked"), "true"));
  assert.deepEqual((await calls("set")).at(-1), { operation: "set", names: ["docs"], app: "claudeCode", on: true });
  console.log("PASS a switch writes one agent and locks the page until it lands");

  // ---- a column turns every listed server off for one agent
  await page.locator('[data-mcp-column="claudeCode"]').click();
  await page.getByRole("menuitem", { name: await tr("mcp.column.disableAll") }).click();
  await eventually(async () => assert.equal(await cell("filesystem", "claudeCode").getAttribute("aria-checked"), "false"));
  const column = (await calls("set")).at(-1);
  assert.equal(column.app, "claudeCode");
  assert.equal(column.on, false);
  assert.equal(column.names.length, 6);
  console.log("PASS a column menu switches every listed server for one agent");

  // ---- a template fills the form; the server goes to the chosen agents
  const add = async () => {
    await page.getByRole("button", { name: await tr("mcp.addServer") }).first().click();
    return page.locator("[data-mcp-editor]");
  };
  let dialog = await add();
  await dialog.getByRole("button", { name: "Context7", exact: true }).click();
  assert.equal(await dialog.getByRole("textbox", { name: await tr("mcp.editor.name") }).inputValue(), "context7");
  const writeTo = dialog.locator("[data-mcp-write-to]");
  assert.equal(await writeTo.getByRole("checkbox", { name: "Codex" }).isChecked(), true);
  assert.equal(await writeTo.getByRole("checkbox", { name: "Pi" }).isDisabled(), true);
  if (captureDir) await dialog.screenshot({ path: path.join(captureDir, "mcp-editor.png") });
  await writeTo.getByRole("checkbox", { name: "Claude Code" }).click();
  await dialog.getByRole("button", { name: await tr("mcp.editor.save"), exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  await row("context7").waitFor();
  const saved = (await calls("save")).at(-1);
  assert.equal(saved.original, null);
  assert.deepEqual(saved.server.apps, ["codex"]);
  assert.deepEqual(saved.server.spec.args, ["-y", "@upstash/context7-mcp"]);
  assert.equal(await page.evaluate(() => localStorage.getItem("agentharbor.mcp-targets")), '["codex"]', "the chosen agents are remembered");
  console.log("PASS a template fills the form and the server reaches only the chosen agents");

  // ---- a whole command line typed into the command field is split
  dialog = await add();
  await dialog.getByRole("textbox", { name: await tr("mcp.editor.name") }).fill("cmdline");
  const command = dialog.getByRole("textbox", { name: await tr("mcp.editor.command") });
  await command.fill("npx -y @scope/pkg");
  await command.blur();
  await eventually(async () => assert.equal(await dialog.getByRole("textbox", { name: await tr("mcp.editor.args") }).inputValue(), "-y\n@scope/pkg"));
  assert.equal(await command.inputValue(), "npx");
  await dialog.getByRole("button", { name: await tr("mcp.editor.cancel"), exact: true }).click();
  await dialog.waitFor({ state: "detached" });
  console.log("PASS a command line in the command field is split into arguments");

  // ---- pasting several servers adds the new ones and skips taken names
  dialog = await add();
  await dialog.getByRole("textbox", { name: await tr("mcp.editor.paste") }).fill(JSON.stringify({ mcpServers: {
    docs: { url: "https://other.invalid/mcp" },
    "time-now": { command: "uvx", args: ["mcp-server-time"] },
  } }));
  await dialog.getByRole("button", { name: await tr("mcp.editor.recognize") }).click();
  const pasted = dialog.locator("[data-mcp-pasted]");
  await pasted.getByText(await tr("mcp.editor.pasteMany", { count: 2 }), { exact: true }).waitFor();
  await pasted.getByText(await tr("mcp.editor.exists"), { exact: true }).waitFor();
  await pasted.getByRole("button", { name: await tr("mcp.editor.addAll") }).click();
  await dialog.waitFor({ state: "detached" });
  await row("time-now").waitFor();
  assert.equal((await calls("save")).filter((call) => call.server.name === "docs").length, 0, "a taken name is skipped");
  console.log("PASS pasting several servers adds the new ones and skips taken names");

  // ---- importing brings in what an agent has outside the list
  await page.getByRole("button", { name: await tr("mcp.import") }).click();
  await row("cursor-only").waitFor();
  assert.equal(await cell("cursor-only", "cursor").getAttribute("aria-checked"), "true");
  console.log("PASS importing brings in the servers an agent has outside the list");

  // ---- deleting asks first
  await row("tool-1").getByRole("button", { name: await tr("mcp.more") }).click();
  await page.getByRole("menuitem", { name: await tr("mcp.row.delete") }).click();
  const alert = page.getByRole("alertdialog");
  await alert.getByText(await tr("mcp.confirmDelete", { name: "tool-1" }), { exact: true }).waitFor();
  await alert.getByRole("button", { name: await tr("mcp.delete"), exact: true }).click();
  await row("tool-1").waitFor({ state: "detached" });
  assert.deepEqual((await calls("remove")).at(-1), { operation: "remove", name: "tool-1" });
  console.log("PASS deleting a server asks first and takes it out of the list");

  await capture("mcp-library-desktop.png");
  await page.setViewportSize({ width: 390, height: 820 });
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  assert.equal(
    await page.locator("[data-page-scroll-container]").evaluate((el) => el.scrollWidth <= el.clientWidth),
    true,
    "the matrix scrolls inside its card, not the page",
  );
  await capture("mcp-library-mobile-dark.png");
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await page.setViewportSize({ width: 1200, height: 820 });

  // ---- the command palette lists the library
  await page.getByRole("button", { name: "测试命令面板", exact: true }).click();
  await page.getByText("MCP: context7", { exact: true }).click();
  assert.equal(await page.evaluate(() => window.review.navigatedTo), "mcp");
  console.log("PASS the command palette lists the MCP library");
}
