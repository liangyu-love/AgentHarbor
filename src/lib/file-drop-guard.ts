/**
 * With Tauri's own drop handler off (so pages can take dropped files), a file
 * dropped where no page takes it would open in the webview and replace the
 * app. Files no drop zone claimed are refused instead.
 */
export function installFileDropGuard() {
  const refuse = (event: DragEvent) => {
    if (event.defaultPrevented || !event.dataTransfer?.types.includes("Files")) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "none";
  };
  window.addEventListener("dragover", refuse);
  window.addEventListener("drop", refuse);
}
