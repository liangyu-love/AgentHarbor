import assert from "node:assert/strict";

/** The album remembered as the view lights its stage the first time the gallery opens. */
export async function testGalleryAlbum({ page, reset }) {
  await reset("image");
  await page.evaluate(() => localStorage.setItem("agentharbor.gallery-view", "album"));
  const labels = await page.evaluate(() => window.review.labels);
  // Thumbnails arrive late, as a real gallery's do, after the album has settled.
  await page.evaluate(() => { window.review.holdThumbnails = true; window.review.slowGallery = true; });
  await page.getByRole("button", { name: labels.galleryTitle, exact: true }).click();
  const album = page.getByRole("region", { name: "图集", exact: true });
  await album.waitFor();
  await page.waitForTimeout(800);
  await page.evaluate(() => { window.review.holdThumbnails = false; window.review.thumbnailResolvers.splice(0).forEach(resolve => resolve()); });
  const onAir = album.locator(".gallery-album-slide[data-on-air] img").first();
  await onAir.waitFor({ timeout: 5000 });
  assert.equal(await album.locator(".gallery-album-backdrop").count(), 1, "the stage takes the on-air image's colours");
  await page.waitForFunction(() => window.review.thumbnailEdges.includes(1600));
  await page.evaluate(() => localStorage.removeItem("agentharbor.gallery-view"));
  console.log("PASS an album remembered as the view shows its images the first time the gallery opens");
}
