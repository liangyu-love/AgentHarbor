import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// The injection page's newer pieces: the router port holder notice, the
// folding group bands, each client's start button and the two-pane model
// dialog.
export async function testClientAccess({ page, reset, captureDir }) {
  const tr = (key, options) => page.evaluate(({ key, options }) => window.review.t(key, options), { key, options });

  // ---- the program holding the router's port
  await reset("access");
  await page.evaluate(() => {
    window.review.portOwner = { port: 43127, pid: 4242, name: "Cursor.exe" };
    window.review.mount("access");
  });
  const notice = page.locator("[data-port-owner]");
  await notice.getByText(await tr("newapi.portOwner.title", { port: 43127, pid: 4242, name: "Cursor.exe" }), { exact: true }).waitFor();
  await notice.getByRole("button", { name: await tr("newapi.portOwner.end"), exact: true }).click();
  const alert = page.getByRole("alertdialog");
  await alert.getByText(await tr("newapi.portOwner.confirmTitle", { name: "Cursor.exe" }), { exact: true }).waitFor();
  await alert.getByRole("button", { name: await tr("newapi.portOwner.end"), exact: true }).click();
  await page.waitForFunction(() => window.review.endedPortOwner === 4242);
  await notice.waitFor({ state: "detached" });
  console.log("PASS a program holding the router port is named and ended only after confirmation");

  // ---- a port the system reserved has no program to end; another one is taken in place
  await reset("access");
  await page.evaluate(() => {
    window.review.portOwner = { port: 43127, reserved: true };
    window.review.mount("access");
  });
  await notice.getByText(await tr("newapi.portOwner.reservedTitle", { port: 43127 }), { exact: true }).waitFor();
  assert.equal(await notice.getByRole("button", { name: await tr("newapi.portOwner.end"), exact: true }).count(), 0);
  const portField = notice.getByRole("textbox", { name: await tr("settings.routerPort.label") });
  const applyPort = notice.getByRole("button", { name: await tr("settings.routerPort.apply"), exact: true });
  for (const refused of ["80", "70000", "43a"]) {
    await portField.fill(refused);
    assert.equal(await applyPort.isDisabled(), true, refused);
  }
  await portField.fill("43200");
  await applyPort.click();
  await page.waitForFunction(() => window.review.routerPorts?.at(-1) === 43200);
  await page.waitForFunction(() => window.review.lastToast === window.review.t("settings.routerPort.changed", { port: 43200 }));
  await notice.waitFor({ state: "detached" });
  console.log("PASS a reserved router port offers another port instead of a program to end");

  // ---- groups are tabs: one shows at a time, and every tab keeps its summary
  await reset("access");
  const tab = id => page.locator(`[data-inject-tab="${id}"]`);
  const panel = id => page.locator(`[data-inject-section="${id}"]`);
  const selected = async id => assert.equal(await tab(id).getAttribute("aria-selected"), "true");
  await selected("clients");
  await page.locator("[data-client-console]").waitFor({ state: "visible" });
  assert.equal(await panel("gateway").isVisible(), false);
  await tab("gateway").locator("[data-section-summary]").getByText(await tr("gateway.stopped"), { exact: false }).waitFor();
  await tab("gateway").click();
  await selected("gateway");
  await panel("gateway").locator("[data-gateway]").waitFor({ state: "visible" });
  assert.equal(await page.locator("[data-client-console]").isVisible(), false);
  await page.waitForFunction(() => document.getAnimations().every(animation => animation.playState !== "running"
    || animation.effect?.getTiming().iterations === Infinity));
  const [mark, chosen] = await Promise.all([page.locator(".group-tabs-indicator"), tab("gateway")].map(locator =>
    locator.evaluate(element => { const { left, width } = element.getBoundingClientRect(); return { left, width }; })));
  assert.ok(Math.abs(mark.left - chosen.left) <= 1 && Math.abs(mark.width - chosen.width) <= 1, JSON.stringify({ mark, chosen }));
  await tab("gateway").press("ArrowRight");
  await selected("routing");
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-inject-tab")), "routing");
  // A shortcut into another group switches to it.
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("agentharbor:select-client", { detail: "pi" })));
  await page.locator("[data-client-console]").waitFor({ state: "visible" });
  await selected("clients");
  assert.equal(await page.locator('[data-client-tab="pi"]').getAttribute("aria-selected"), "true");
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("agentharbor:expand-section", { detail: "aggregation" })));
  await eventually(() => selected("routing"));
  if (captureDir) {
    const shot = name => page.locator(".group-tabs").evaluate(element => {
      const scroller = element.closest("[data-page-scroll-container]");
      scroller.scrollTop += element.getBoundingClientRect().top - scroller.getBoundingClientRect().top - 16;
    })
      .then(() => page.screenshot({ path: path.join(captureDir, name), animations: "disabled" }));
    await tab("gateway").click();
    await page.mouse.move(0, 0);
    await shot("inject-tabs-light.png");
    await page.evaluate(() => document.documentElement.classList.add("dark"));
    await shot("inject-tabs-dark.png");
    await page.evaluate(() => {
      document.documentElement.classList.remove("dark");
      document.documentElement.dataset.hasCustomBg = "true";
      const wallpaper = document.createElement("div");
      wallpaper.style.cssText = "position:fixed;inset:0;z-index:-1;background:linear-gradient(135deg,#c7d2fe,#fbcfe8 50%,#fde68a)";
      document.body.prepend(wallpaper);
      document.body.style.background = "transparent";
    });
    await shot("inject-tabs-wallpaper.png");
    await page.evaluate(() => { delete document.documentElement.dataset.hasCustomBg; document.body.firstElementChild.remove(); document.body.style.background = ""; });
    await page.setViewportSize({ width: 720, height: 800 });
    await shot("inject-tabs-narrow.png");
    await page.setViewportSize({ width: 1200, height: 700 });
  }
  console.log("PASS groups are tabs with live summaries; the highlight follows, and keys and shortcuts switch them");

  // ---- start buttons
  await reset("clients-existing");
  await page.evaluate(() => {
    localStorage.setItem("agentharbor.launch-folders.v1", JSON.stringify({ recent: ["D:/work/alpha", "D:/work/beta"], last: { claudeCode: "D:/work/alpha" } }));
    window.review.launchCalls = [];
    window.review.mount("clients-existing");
  });
  const launch = page.locator('#client-config-claudeCode [data-client-launch="terminal"]');
  await launch.getByText("alpha", { exact: true }).waitFor();
  await launch.getByRole("button", { name: await tr("clients.launch.terminal") }).click();
  await page.waitForFunction(() => window.review.launchCalls.length === 1);
  assert.deepEqual(await page.evaluate(() => window.review.launchCalls[0]), { targetId: "claudeCode", harnessProfile: null, folder: "D:/work/alpha" });
  await launch.getByRole("button", { name: await tr("clients.launch.folders") }).click();
  await page.getByRole("menuitem").filter({ hasText: "D:/work/beta" }).click();
  await page.waitForFunction(() => window.review.launchCalls.length === 2);
  assert.equal(await page.evaluate(() => window.review.launchCalls[1].folder), "D:/work/beta");
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem("agentharbor.launch-folders.v1")).last.claudeCode), "D:/work/beta");
  await page.evaluate(() => {
    window.review.launchPlans = { claudeCode: { kind: "terminal", command: "claude", installed: false } };
    window.review.mount("clients-existing");
  });
  await eventually(async () => assert.equal(await page.locator('#client-config-claudeCode [data-client-launch="terminal"] button').first().isDisabled(), true));
  await page.evaluate(() => {
    window.review.launchPlans = { claudeCode: { kind: "app", installed: true, running: false } };
    window.review.mount("clients-existing");
  });
  await page.locator('#client-config-claudeCode [data-client-launch="app"]')
    .getByText(await tr("clients.launch.start", { name: "Claude Code" }), { exact: true }).waitFor();
  await page.evaluate(() => {
    localStorage.removeItem("agentharbor.launch-folders.v1");
    window.review.launchPlans = null;
  });
  console.log("PASS CLI clients start in a remembered folder, recent folders are one click away, and missing CLIs are disabled");

  // ---- a router configuration withdrawn at exit waits to come back
  await reset("clients");
  await page.evaluate(() => {
    window.review.clientStatusFixtures = [{ targetId: "opencode", name: "OpenCode", path: "C:/synthetic/opencode.json",
      protocols: ["responses", "chatCompletions", "anthropicMessages"], configured: false, conflict: false, conflictFields: [],
      error: null, providerId: null, model: null, request: null, resumePending: true }];
    window.review.mount("clients");
  });
  const resume = page.locator("#client-config-opencode [data-client-resume]");
  await resume.getByText(await tr("clients.resume.note"), { exact: true }).waitFor();
  await resume.getByRole("button", { name: await tr("clients.resume.discard"), exact: true }).click();
  await page.waitForFunction(() => window.review.discardedResume === "opencode");
  await page.evaluate(() => { window.review.clientStatusFixtures = null; });
  console.log("PASS a router configuration withdrawn at exit says it will come back and can be dropped");

  // ---- the two-pane model dialog
  await reset("clients-opencode");
  await page.getByRole("button", { name: await tr("clients.configure"), exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("[data-client-config-sources]").waitFor();
  await dialog.getByRole("button", { name: /Provider alternate/ }).waitFor();
  await dialog.getByRole("textbox", { name: await tr("clients.dialog.searchSources") }).fill("alternate");
  await eventually(async () => assert.equal(await dialog.getByRole("button", { name: /Provider mixed/ }).count(), 0));
  await dialog.getByRole("textbox", { name: await tr("clients.dialog.searchSources") }).fill("");
  await dialog.locator("[data-client-config-models] details").filter({ hasText: "Provider mixed" }).locator("summary").click();
  await dialog.getByRole("button", { name: await tr("clients.dialog.setDefault", { model: "chat-model" }), exact: true }).click();
  await eventually(async () => assert.equal(await dialog.locator("#client-model").textContent(), "chat-model"));
  await dialog.locator("[data-client-config-summary]").getByText("Provider mixed / chat-model", { exact: true }).waitFor();
  if (captureDir) await page.screenshot({ path: path.join(captureDir, "client-config-two-pane.png"), animations: "disabled" });
  await dialog.getByRole("button", { name: await tr("clients.preview"), exact: true }).click();
  await page.waitForFunction(() => window.review.previewed?.model === "chat-model");
  await dialog.locator("[data-client-config-preview] pre").waitFor();
  await dialog.getByRole("button", { name: await tr("common.cancel"), exact: true }).click();
  console.log("PASS the model dialog searches its sources and a star picks the default model");
}
