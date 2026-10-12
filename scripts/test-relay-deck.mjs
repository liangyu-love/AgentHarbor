import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

export async function testRelayDeck({ page, reset, captureDir }) {
  const deck = () => page.locator("[data-relay-deck]").first();
  const face = () => deck().locator("[data-relay-card-face]");
  const member = async id => {
    await page.waitForFunction(id => document.querySelector("[data-relay-card-face]")?.getAttribute("data-provider-id") === id, id);
  };
  const settled = async () => {
    await page.waitForFunction(() => {
      const node = document.querySelector("[data-relay-card-face]");
      return node && !node.hasAttribute("data-deck-exiting") && getComputedStyle(node).animationName === "none";
    });
  };
  const turn = async (name, id) => {
    await deck().getByRole("button", { name, exact: true }).click();
    await member(id);
    await settled();
  };
  await reset("groups");
  await member("group-a");
  assert.equal(await page.locator("[data-relay-deck]").count(), 2);
  assert.equal(await deck().locator(".relay-deck-layer").count(), 2);
  assert.equal(await page.locator("[data-relay-deck]").last().locator(".relay-deck-layer").count(), 0);
  assert.equal(await deck().locator("[data-relay-card-face]").count(), 1);
  // Layers peek above the face inside space every deck reserves, so a stacked
  // site and a single relay keep the same footprint.
  const footprint = await page.locator("[data-relay-deck]").evaluateAll(decks => decks.map(deck => {
    const face = deck.querySelector("[data-relay-card-face]").getBoundingClientRect();
    return { deck: deck.getBoundingClientRect().height, face: face.height, faceTop: face.top,
      layerTops: [...deck.querySelectorAll(".relay-deck-layer")].map(layer => layer.getBoundingClientRect().top) };
  }));
  assert.ok(Math.abs(footprint[0].deck - footprint[1].deck) < 1 && Math.abs(footprint[0].face - footprint[1].face) < 1, JSON.stringify(footprint));
  assert.ok(footprint[0].layerTops.every(top => top < footprint[0].faceTop) && footprint[0].layerTops[0] < footprint[0].layerTops[1], JSON.stringify(footprint));
  console.log("PASS normalized same URLs share a deck as tall as a single relay; single relay stays independent");

  await deck().getByRole("button", { name: "下一个站点配置", exact: true }).click();
  await member("group-b");
  assert.equal(await face().evaluate(node => getComputedStyle(node).animationName), "relay-card-flip-next");
  await settled();
  await turn("下一个站点配置", "group-c");
  await turn("下一个站点配置", "group-a");
  await turn("上一个站点配置", "group-c");
  await deck().getByRole("button", { name: "上一个站点配置", exact: true }).focus();
  await page.keyboard.press("Enter");
  await member("group-b");
  await settled();
  assert.equal(await deck().getByRole("button", { name: "上一个站点配置", exact: true }).evaluate(node => node === document.activeElement), true);
  await turn("下一个站点配置", "group-c");
  console.log("PASS next and previous animate, wrap both ways and retain one live face");

  await deck().focus();
  await page.keyboard.press("ArrowLeft");
  await member("group-b");
  await settled();
  assert.equal(await deck().evaluate(node => node === document.activeElement), true);
  await page.keyboard.press("Enter");
  await member("group-c");
  await settled();
  await page.keyboard.press("Space");
  await member("group-a");
  await settled();
  await face().getByRole("heading", { name: "relay.invalid", exact: true }).click();
  await member("group-b");
  await settled();
  console.log("PASS deck keyboard and blank-area turns preserve focus and selection");

  await face().getByRole("button", { name: "编辑", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.locator("#newapi-provider-name").waitFor();
  await eventually(async () => assert.equal(await dialog.locator("#newapi-provider-name").inputValue(), "Relay Beta"));
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "hidden" });
  await member("group-b");
  await face().getByRole("button", { name: "查询余额 Relay Beta", exact: true }).click();
  await dialog.getByText("与「Relay Alpha」共享余额。", { exact: true }).waitFor();
  await page.keyboard.press("Escape");
  await dialog.waitFor({ state: "hidden" });
  await member("group-b");
  await page.evaluate(() => {
    window.review.deckMutations = [];
    window.__TAURI_INTERNALS__.invoke = async (command, args) => {
      // Provider changes also refresh the independently managed Desktop card.
      // This fixture has no Desktop configuration and counts only relay mutations.
      if (command === "get_claude_desktop_status") throw "claudeDesktop.errors.pathsNotDetected";
      window.review.deckMutations.push({ command, args });
      const state = window.review.getState();
      state.providers = state.providers.map(provider => provider.id === args.id ? { ...provider, enabled: args.enabled } : provider);
      return { state: structuredClone(state), applied: null };
    };
  });
  await face().getByRole("switch").click();
  await page.waitForFunction(() => window.review.deckMutations.length === 1);
  assert.equal(await page.evaluate(() => window.review.deckMutations[0].args.id), "group-b");
  await member("group-b");
  await page.evaluate(() => { delete window.__TAURI_INTERNALS__.invoke; });
  console.log("PASS edit, shared balance and enable operations target the displayed member without turning the deck");

  await face().getByRole("button", { name: "展开同中转配置", exact: true }).click();
  await eventually(async () => assert.equal(await deck().locator(".relay-deck-layer").count(), 0));
  await deck().getByRole("button", { name: "Relay Gamma", exact: true }).click();
  await member("group-c");
  await settled();
  assert.equal(await deck().locator(".relay-deck-layer").count(), 2);
  const soloBefore = await page.locator("[data-relay-deck]").last().boundingBox();
  await face().getByRole("button", { name: "展开同中转配置", exact: true }).click();
  const soloAfter = await page.locator("[data-relay-deck]").last().boundingBox();
  assert.ok(Math.abs(soloBefore.height - soloAfter.height) < 1);
  await deck().getByRole("button", { name: "收起层叠堆叠", exact: true }).first().click();
  await settled();
  console.log("PASS expanded members select directly and never stretch independent relay cards");

  // Filter removal cancels the outgoing timer; remount keeps the last selection.
  await deck().getByRole("button", { name: "下一个站点配置", exact: true }).click();
  await page.getByRole("button", { name: /^solo\.invalid 1$/ }).click();
  await page.getByRole("heading", { name: "solo.invalid", exact: true }).waitFor();
  await page.getByRole("button", { name: "全部", exact: true }).click();
  await member("group-c");
  await settled();
  console.log("PASS filtering during a turn cancels the pending selection");

  // Remove the requested member while the outgoing card is still displayed.
  await page.evaluate(async () => {
    const node = document.querySelector("[data-relay-card-face]");
    node.querySelector('button[aria-label="下一个站点配置"]').click();
    await new Promise(resolve => setTimeout(resolve, 40));
    window.review.deckDeleteProbe = { origin: node.getAttribute("data-provider-id"), exit: node.getAttribute("data-deck-exiting") };
    // Real IPC returns a new DTO. Never mutate a fixture object shared with
    // QueryClient before its structural-sharing comparison has run.
    const state = structuredClone(window.review.getState());
    state.providers = state.providers.filter(provider => provider.id !== "group-a");
    window.review.providersClient.setQueryData(["newapi-state"], state);
  });
  await settled();
  assert.equal(await face().getAttribute("data-provider-id"), "group-c", JSON.stringify(await page.evaluate(() => window.review.deckDeleteProbe)) + " " + await deck().innerText());
  console.log("PASS a removed destination cannot be selected by an old animation timer");

  await page.emulateMedia({ reducedMotion: "reduce" });
  await turn("下一个站点配置", "group-b");
  assert.equal(await face().evaluate(node => getComputedStyle(node).animationName), "none");
  assert.equal(await face().getAttribute("data-deck-exiting"), null);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  console.log("PASS reduced motion changes members directly without spatial movement");

  await reset("groups");
  await member("group-a");
  await page.evaluate(() => {
    const node = document.querySelector("[data-relay-card-face]");
    node.querySelector('button[aria-label="下一个站点配置"]').click();
    node.querySelector('button[aria-label="上一个站点配置"]').click();
  });
  await member("group-b");
  await settled();
  await turn("上一个站点配置", "group-a");
  console.log("PASS same-batch repeated page controls keep the first turn and keyboard focus survives paging");
  await reset("compact");
  await page.evaluate(() => {
    const first = { ...window.review.getState().providers[0], baseUrl: "https://relay.invalid/v1" };
    window.review.providerFixtures = [
      ...Array.from({ length: 8 }, (_, index) => ({ ...first, id: "stack-" + index,
        name: index === 0 ? "Long provider name demonstrating a busy grouped relay card" : "Member " + index })),
      { ...first, id: "solo", baseUrl: "https://solo.invalid/v1" },
      { ...first, id: "other-path", baseUrl: "https://relay.invalid/v2" },
    ];
    window.review.mount("groups");
  });
  await member("stack-0");
  assert.equal(await deck().locator(".relay-deck-layer").count(), 3);
  assert.equal(await page.locator("[data-relay-deck]").count(), 3);
  console.log("PASS many-member decks cap visible layers at three and different URL paths remain separate");

  await deck().evaluate(node => { node.style.width = "310px"; });
  const footerBounds = await face().evaluate(node => {
    const card = node.getBoundingClientRect();
    const deck = node.closest("[data-relay-deck]").getBoundingClientRect();
    const footer = node.querySelector(".relay-card-balance-label").closest("button").parentElement;
    return { width: card.width, deck: deck.width, right: card.right, left: card.left,
      buttons: [...footer.querySelectorAll("button")].map(button => {
        const bounds = button.getBoundingClientRect();
        return { left: bounds.left, right: bounds.right };
      }) };
  });
  assert.ok(Math.abs(footerBounds.width - footerBounds.deck) < 1, JSON.stringify(footerBounds));
  assert.ok(footerBounds.buttons.every(bounds => bounds.left >= footerBounds.left && bounds.right <= footerBounds.right), JSON.stringify(footerBounds));
  // The site name stays whole on a narrow face, whatever the display font's metrics.
  const title = await face().getByRole("heading", { name: "relay.invalid", exact: true }).evaluate(node => ({
    width: node.getBoundingClientRect().width, clipped: node.scrollWidth > node.clientWidth + 1 }));
  assert.ok(!title.clipped && title.width >= 60, `Relay title lost its readable width: ${JSON.stringify(title)}`);
  if (captureDir) await deck().screenshot({ path: path.join(captureDir, "relay-deck-compact-actions.png"), animations: "disabled" });
  await deck().evaluate(node => { node.style.removeProperty("width"); });
  console.log("PASS a narrow 310px card face keeps the full deck width and every footer operation without clipping");

  if (captureDir) {
    for (const theme of ["light", "dark"]) {
      for (const width of [1200, 720, 420]) {
        await page.setViewportSize({ width, height: 780 });
        await page.evaluate(theme => document.documentElement.classList.toggle("dark", theme === "dark"), theme);
        await deck().evaluate(node => node.scrollIntoView({ block: "center" }));
        const bounds = await deck().evaluate(node => ({ width: node.getBoundingClientRect().width,
          scroll: node.scrollWidth, viewport: document.documentElement.clientWidth }));
        assert.ok(bounds.scroll <= bounds.width + 2 && bounds.width <= bounds.viewport, JSON.stringify(bounds));
        await page.screenshot({ path: path.join(captureDir, `relay-deck-${theme}-${width}.png`) });
      }
    }
    await page.setViewportSize({ width: 1200, height: 700 });
    await page.evaluate(() => document.documentElement.classList.remove("dark"));
    console.log("PASS light and dark decks fit desktop, intermediate and narrow layouts");
  }
}
