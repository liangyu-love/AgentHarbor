import assert from "node:assert/strict";
import path from "node:path";

const alphaOf = color => {
  const match = /rgba?\(([^)]+)\)/.exec(color);
  if (!match) return Number.NaN;
  const parts = match[1].split(/[,\s/]+/).filter(Boolean);
  return parts.length > 3 ? Number(parts[3]) : 1;
};

// Codex access layout, wallpaper surface tokens, floating plates and the
// dark-mode scrim floor.
export async function testUiRound7({ page, reset, captureDir }) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await reset("access");
  const codex = page.locator('section[aria-labelledby="newapi-routing-title"]');
  await codex.locator('[aria-labelledby="codex-subagents-title"]').waitFor();
  const box = selector => codex.locator(selector).first().evaluate(element => {
    const rect = element.getBoundingClientRect();
    return { x: rect.x, y: rect.y, w: rect.width, h: rect.height, right: rect.right, bottom: rect.bottom };
  });
  const connection = await box(".harbor-well:has(#codex-access-mode)");
  const subagents = await box('[aria-labelledby="codex-subagents-title"]');
  const pipeline = await box(".harbor-well:has([data-codex-pipeline])");
  assert.ok(Math.abs(connection.y - subagents.y) < 1 && Math.abs(connection.h - subagents.h) < 1,
    `connection and subagents share one row height: ${JSON.stringify({ connection, subagents })}`);
  assert.ok(Math.abs(connection.w - subagents.w) < 2 && subagents.x > connection.right - 1, "two equal columns");
  assert.ok(pipeline.y >= connection.bottom && Math.abs(pipeline.x - connection.x) < 1 && Math.abs(pipeline.right - subagents.right) < 1,
    "the route spans the full width underneath");
  const nodeTops = () => codex.locator("[data-codex-pipeline] > :not([aria-hidden])")
    .evaluateAll(nodes => nodes.map(node => Math.round(node.getBoundingClientRect().top)));
  const wide = await nodeTops();
  assert.equal(wide.length, 3);
  assert.equal(new Set(wide).size, 1, `a left-to-right chain on wide screens: ${wide}`);
  if (captureDir) await codex.screenshot({ path: path.join(captureDir, "codex-access-layout.png") });
  await page.setViewportSize({ width: 820, height: 900 });
  const narrow = await nodeTops();
  assert.ok(narrow[0] < narrow[1] && narrow[1] < narrow[2], `stacked on narrow screens: ${narrow}`);
  const stackedSubagents = await box('[aria-labelledby="codex-subagents-title"]');
  const stackedConnection = await box(".harbor-well:has(#codex-access-mode)");
  assert.ok(stackedSubagents.y >= stackedConnection.bottom, "subagents move under the connection panel");
  await page.setViewportSize({ width: 1440, height: 900 });
  console.log("PASS Codex access puts subagents beside the connection panel and the route chain underneath");

  // Surfaces multiply their own wallpaper alpha instead of carrying it in the
  // colour channels, so `/NN` modifiers stay valid colours on a wallpaper.
  const surfaces = await page.evaluate(() => {
    const root = document.documentElement;
    const probe = className => {
      const element = document.createElement("div");
      element.className = className;
      document.body.append(element);
      const color = getComputedStyle(element).backgroundColor;
      element.remove();
      return color;
    };
    root.classList.add("dark");
    root.dataset.hasCustomBg = "true";
    const wallpaper = { card: probe("bg-card"), card90: probe("bg-card/90"), background80: probe("bg-background/80"),
      popover: probe("bg-popover"), plate: getComputedStyle(document.querySelector(".wallpaper-plate")).backgroundColor };
    delete root.dataset.hasCustomBg;
    const plain = { card: probe("bg-card"), card90: probe("bg-card/90"),
      plate: getComputedStyle(document.querySelector(".wallpaper-plate")).backgroundColor };
    root.classList.remove("dark");
    return { wallpaper, plain };
  });
  const near = (actual, expected, label) => assert.ok(Math.abs(alphaOf(actual) - expected) < 0.01, `${label}: ${actual}`);
  near(surfaces.wallpaper.card, 0.82, "wallpaper card");
  near(surfaces.wallpaper.card90, 0.9 * 0.82, "wallpaper bg-card/90");
  near(surfaces.wallpaper.background80, 0.8 * 0.5, "wallpaper bg-background/80");
  near(surfaces.wallpaper.popover, 0.9, "wallpaper popover");
  near(surfaces.wallpaper.plate, 0.62, "floating heading plate");
  near(surfaces.plain.card, 1, "plain card");
  near(surfaces.plain.card90, 0.9, "plain bg-card/90");
  assert.equal(alphaOf(surfaces.plain.plate), 0, "no plate without a wallpaper");
  console.log("PASS wallpaper surfaces keep valid translucent colours and floating headings get a frosted plate");

  const floors = await page.evaluate(() => {
    const floor = window.review.darkScrimFloor;
    const samples = [];
    for (let brightness = 0; brightness <= 1.0001; brightness += 0.05) {
      for (let opacity = 0.3; opacity <= 1.0001; opacity += 0.1) {
        const value = floor(brightness, opacity);
        const backdrop = brightness * opacity + 0.06 * (1 - opacity);
        samples.push({ value, dimmed: backdrop * (1 - value) + 0.06 * value });
      }
    }
    return { bright: floor(0.9, 0.98), dark: floor(0.1, 1), faint: floor(0.5, 0.45), white: floor(1, 1), samples };
  });
  assert.ok(Math.abs(floors.bright - 0.71) < 0.02, `a bright wallpaper is dimmed: ${floors.bright}`);
  assert.equal(floors.dark, 0, "a dark wallpaper is left alone");
  assert.equal(floors.faint, 0, "a faint wallpaper is already dim enough");
  assert.ok(floors.white <= 0.85, "never hides the wallpaper completely");
  assert.ok(floors.samples.every(({ value, dimmed }) => value >= 0 && value <= 0.85 && (dimmed <= 0.3 + 1e-9 || value === 0.85)),
    "the floor always reaches the target unless capped");
  console.log("PASS dark mode raises the wallpaper scrim only as far as its brightness needs");
}
