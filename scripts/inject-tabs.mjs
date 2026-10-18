// The injection page shows one group at a time; tests reach a group's content
// through its tab ("clients", "gateway" or "routing"). The highlight settles
// first: its spring overshoot is transient and must not read as overflow.
export async function showInjectTab(page, id) {
  await page.locator(`[data-inject-tab="${id}"]`).click();
  await page.locator(`[data-inject-section="${id}"]`).waitFor({ state: "visible" });
  await page.locator(".group-tabs-indicator").evaluate(element => Promise.all(element.getAnimations().map(animation => animation.finished)));
}
