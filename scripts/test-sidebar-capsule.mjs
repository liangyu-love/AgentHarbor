import assert from "node:assert/strict";
import path from "node:path";
import { eventually } from "./eventually.mjs";

// The sidebar is a capsule centred on the left edge. One highlight glides
// between items, the capsule springs between widths, and over a wallpaper it
// turns to frosted glass.
export async function testSidebarCapsule({ page, reset, captureDir }) {
  const capture = async name => { if (captureDir) await page.screenshot({ path: path.join(captureDir, name), animations: "disabled" }); };
  const capsule = page.locator('[data-sidebar="sidebar"]');
  const indicator = page.locator("[data-nav-indicator]");
  const navButton = route => page.locator(`[data-nav-route="${route}"]`);
  const box = async locator => locator.evaluate(element => {
    const { top, bottom, left, right, width, height } = element.getBoundingClientRect();
    return { top, bottom, left, right, width, height };
  });
  // Finite animations and transitions only; the routing status light pings forever.
  const settled = () => page.waitForFunction(() => document.getAnimations()
    .every(animation => animation.effect?.getTiming().iterations === Infinity || animation.playState !== "running"));
  const assertIndicatorUnder = async route => {
    const [mark, button] = [await box(indicator), await box(navButton(route))];
    for (const key of ["top", "left", "width", "height"]) {
      assert.ok(Math.abs(mark[key] - button[key]) <= 1, `indicator ${key} ${mark[key]} sits under ${route} ${button[key]}`);
    }
  };
  const assertCentred = async () => {
    const rect = await box(capsule);
    const viewport = page.viewportSize();
    assert.ok(rect.top >= 18 && viewport.height - rect.bottom >= 18, JSON.stringify(rect));
    assert.ok(Math.abs(rect.top - (viewport.height - rect.bottom)) <= 1, `capsule is vertically centred: ${JSON.stringify(rect)}`);
    return rect;
  };

  await page.setViewportSize({ width: 1200, height: 800 });
  await reset("sidebar");
  await capsule.waitFor();
  assert.ok(await capsule.evaluate(element => getComputedStyle(element).animationName === "sidebar-capsule-in"), "The capsule slides in");
  await settled();
  const open = await assertCentred();
  assert.equal(Math.round(open.width), 148);
  assert.ok(open.height < 800 - 36, "The capsule is as tall as its content, not the window");
  assert.equal(await capsule.evaluate(element => getComputedStyle(element).borderTopLeftRadius), "28px");
  assert.equal(await page.locator(".poster-kicker", { hasText: /^(Home|Connect|Work|Extend|System)$/ }).count(), 0, "Hairlines part the groups instead of headings");
  await assertIndicatorUnder("relays");
  assert.equal(await navButton("relays").getAttribute("data-active"), "true");

  await navButton("threads").click();
  assert.equal(await page.evaluate(() => window.review.navigatedTo), "threads");
  assert.equal(await indicator.getAttribute("data-instant"), null, "A new page glides the highlight there");
  assert.ok(await indicator.evaluate(element => element.getAnimations().some(animation => animation.transitionProperty === "transform")),
    "The highlight is in motion");
  await settled();
  await assertIndicatorUnder("threads");
  await navButton("mcp").hover();
  await eventually(async () => assert.notEqual(await navButton("mcp").locator("svg").evaluate(element => getComputedStyle(element).transform), "none",
    "A hovered icon lifts"));
  await page.mouse.move(600, 400);
  await capture("sidebar-capsule-open.png");

  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  assert.ok(await page.locator(".sidebar-capsule-frame").evaluate(element => getComputedStyle(element).transitionTimingFunction.includes("1.22")),
    "The width springs past its mark");
  await settled();
  const closed = await assertCentred();
  assert.equal(Math.round(closed.width), 52);
  assert.ok(Number.parseFloat(await capsule.evaluate(element => getComputedStyle(element).borderTopLeftRadius)) >= closed.width / 2, "Closed, both ends are round");
  await assertIndicatorUnder("threads");
  assert.equal(await page.locator("[data-route-light]").isVisible(), true, "Closed, the routing light still shows");
  const icon = await box(navButton("threads"));
  assert.ok(Math.abs(icon.left + icon.width / 2 - (closed.left + closed.width / 2)) <= 1, "Closed icons sit on the capsule's centre line");
  await capture("sidebar-capsule-closed.png");

  // A short window keeps the capsule inside its margins and scrolls the items.
  await page.setViewportSize({ width: 1200, height: 520 });
  await settled();
  const short = await box(capsule);
  assert.ok(short.top >= 18 && 520 - short.bottom >= 18, JSON.stringify(short));
  await eventually(async () => assert.equal(await page.locator('[data-sidebar="content"]').evaluate(element => element.scrollHeight > element.clientHeight
    && getComputedStyle(element).overflowY === "auto"), true, "Items scroll when the capsule cannot grow"));
  await page.getByRole("button", { name: "Toggle Sidebar" }).click();
  await settled();
  await eventually(async () => assert.equal(await page.locator('[data-sidebar="content"]').evaluate(element => element.scrollHeight > element.clientHeight), true));
  await page.locator('[data-sidebar="content"]').evaluate(element => { element.scrollTop = element.scrollHeight; });
  await navButton("settings").click();
  await settled();
  await assertIndicatorUnder("settings");
  await page.setViewportSize({ width: 1200, height: 800 });

  // Over a wallpaper the capsule is glass and the canvas around it is clear.
  await reset("sidebar");
  await page.evaluate(() => {
    document.documentElement.dataset.hasCustomBg = "true";
    const wallpaper = document.createElement("div");
    wallpaper.dataset.wallpaperLayer = "";
    wallpaper.style.cssText = "position:fixed;inset:0;z-index:0;background:linear-gradient(135deg,#1e3a8a,#0ea5e9 45%,#f59e0b)";
    document.body.prepend(wallpaper);
    // As in the app, the page sheet is clear over a wallpaper.
    document.querySelector("[data-sidebar-fixture-page]").style.background = "transparent";
  });
  await settled();
  const glass = await capsule.evaluate(element => ({
    filter: getComputedStyle(element).backdropFilter,
    background: getComputedStyle(element).backgroundColor,
    canvas: getComputedStyle(element.closest(".group\\/sidebar-wrapper")).backgroundColor,
  }));
  assert.match(glass.filter, /blur\(14px\)/);
  assert.match(glass.background, /\/ 0\.34\)|, 0\.34\)/, `The capsule lets the wallpaper through: ${glass.background}`);
  assert.equal(glass.canvas, "rgba(0, 0, 0, 0)", "The canvas around the capsule shows the wallpaper");
  await capture("sidebar-capsule-wallpaper.png");
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await capture("sidebar-capsule-wallpaper-dark.png");
  await page.evaluate(() => {
    document.documentElement.classList.remove("dark");
    delete document.documentElement.dataset.hasCustomBg;
  });

  await reset("sidebar");
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await settled();
  await capture("sidebar-capsule-dark.png");
  await page.evaluate(() => document.documentElement.classList.remove("dark"));

  // Reduced motion: no arrival, no glide, no spring.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await reset("sidebar");
  await capsule.waitFor();
  assert.equal(await capsule.evaluate(element => getComputedStyle(element).animationName), "none");
  await navButton("usage").click();
  assert.equal(await indicator.evaluate(element => element.getAnimations().length), 0, "The highlight jumps");
  await assertIndicatorUnder("usage");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  assert.equal(await page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches), false,
    "Restore the suite's motion setting rather than the host OS preference");
  console.log("PASS sidebar is a centred capsule with a gliding highlight, springy width, scrollable items and wallpaper glass");
}
