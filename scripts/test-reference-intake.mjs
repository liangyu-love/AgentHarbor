import assert from "node:assert/strict";
import { eventually } from "./eventually.mjs";

/** Fires a drag or paste carrying files, as Explorer, QQ or a screenshot tool would. */
const send = (page, kind, selector, files, text) => page.evaluate(async ({ kind, selector, files, text }) => {
  // Each picture differs, since a picture already among the references is not added twice:
  // a counter, not chance, picks its colour.
  const picture = async () => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 2;
    const context = canvas.getContext("2d");
    const n = window.__intakePictures = (window.__intakePictures ?? 0) + 1;
    context.fillStyle = `rgb(${(n * 53) % 256}, ${(n * 97) % 256}, ${(n * 151) % 256})`;
    context.fillRect(0, 0, 2, 2);
    return new Promise(resolve => canvas.toBlob(resolve, "image/png"));
  };
  const data = new DataTransfer();
  for (const [name, type] of files) data.items.add(new File(type.startsWith("image/") ? [await picture()] : ["x"], name, { type }));
  if (text) data.setData("text/plain", text);
  const target = document.querySelector(selector);
  if (kind === "paste") {
    target.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true }));
    return;
  }
  for (const type of ["dragenter", "dragover", "drop"]) {
    target.dispatchEvent(new DragEvent(type, { dataTransfer: data, bubbles: true, cancelable: true }));
  }
}, { kind, selector, files, text });

export async function testReferenceIntake({ page, reset }) {
  await reset("image");
  const references = page.locator("[data-image-reference] [data-reference]");
  assert.equal(await references.count(), 0);

  // A picture dropped anywhere on the workbench, not only on the strip, becomes a reference.
  await send(page, "drop", "[data-image-prompt]", [["qq-pic.png", "image/png"]]);
  await eventually(async () => assert.equal(await references.count(), 1));
  assert.equal(await page.locator("[data-image-dropzone]").count(), 0, "the drop overlay is gone after the drop");

  // Files that are not pictures are named and left out; the pictures among them still land.
  await send(page, "drop", "[data-image-reference]", [["notes.txt", "text/plain"], ["photo.jpg", "image/jpeg"]]);
  await eventually(async () => assert.equal(await references.count(), 2));
  await page.waitForFunction(() => window.review.lastToast === "只能放入 PNG / JPG / WebP 图片");
  console.log("PASS pictures dropped anywhere on the workbench become references, other files are named");

  // A pasted screenshot becomes a reference, even with the caret in the prompt box.
  await send(page, "paste", "body", [["image.png", "image/png"]]);
  await eventually(async () => assert.equal(await references.count(), 3));
  await send(page, "paste", "[data-image-prompt]", [["image.png", "image/png"]]);
  await eventually(async () => assert.equal(await references.count(), 4));

  // Text pasted into a field stays text.
  await send(page, "paste", "[data-image-prompt]", [["image.png", "image/png"]], "雨夜橘猫");
  await page.waitForTimeout(150);
  assert.equal(await references.count(), 4, "a paste with text into a field is left to the field");
  console.log("PASS pasted pictures become references while pasted text stays in the field");

  // A non-picture pasted into a field is left to the field.
  const fieldPaste = await page.evaluate(() => {
    const data = new DataTransfer();
    data.items.add(new File(["x"], "notes.txt", { type: "text/plain" }));
    const paste = new ClipboardEvent("paste", { clipboardData: data, bubbles: true, cancelable: true });
    document.querySelector("[data-image-prompt]").dispatchEvent(paste);
    return paste.defaultPrevented;
  });
  assert.equal(fieldPaste, false);

  // The whole window takes a drop while the workbench is shown, not only its column.
  await send(page, "drop", "body", [["sidebar.png", "image/png"]]);
  await eventually(async () => assert.equal(await references.count(), 5));

  // With a modal open (a viewer, a confirmation) a drop is refused, never added nor opened in the window.
  const refused = await page.evaluate(() => {
    const modal = document.createElement("div");
    modal.setAttribute("role", "alertdialog");
    document.body.append(modal);
    const data = new DataTransfer();
    data.items.add(new File(["x"], "stray.png", { type: "image/png" }));
    const drop = new DragEvent("drop", { dataTransfer: data, bubbles: true, cancelable: true });
    modal.dispatchEvent(drop);
    modal.remove();
    return drop.defaultPrevented;
  });
  assert.equal(refused, true, "the window guard refuses what nothing took");
  await page.waitForTimeout(150);
  assert.equal(await references.count(), 5, "a drop onto a modal adds nothing");
  console.log("PASS the whole window takes dropped pictures, and a drop onto a modal is refused");
}
