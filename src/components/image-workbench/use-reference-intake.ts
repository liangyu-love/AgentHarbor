import { useEffect, useRef, useState } from "react";
import { toast } from "@/hooks/use-toast";

const ACCEPTED = ["image/png", "image/jpeg", "image/webp"];
/** A picture to take: some apps send it without a type, so its extension also counts. */
const isPicture = (file: File) => ACCEPTED.includes(file.type) || /\.(png|jpe?g|webp)$/i.test(file.name);
/** Files from Explorer, QQ, a browser…; a drag inside the page carries none. */
const carriesFiles = (event: DragEvent) => Array.from(event.dataTransfer?.types ?? []).includes("Files");
const isEditable = (target: EventTarget | null) =>
  target instanceof HTMLElement && (target.isContentEditable || target.matches("input, textarea, select"));
const MODAL = '[role="dialog"], [role="alertdialog"]';
/** A modal is open (a viewer, a confirmation): what reaches the window is not for the references. */
const modalOpen = () => document.querySelector(MODAL) !== null;

/** The pictures among `files`, naming what was left out. */
function pictures(files: File[]) {
  const taken = files.filter(isPicture);
  if (taken.length < files.length) {
    toast({ title: "只能放入 PNG / JPG / WebP 图片", description: `有 ${files.length - taken.length} 个文件没有加入`, variant: "destructive" });
  }
  return taken;
}

/**
 * References brought in from outside the app while the workbench is the page
 * shown: pictures dropped anywhere in the window, or pasted (a screenshot, a
 * copied picture). With a modal open both are left alone.
 */
export function useReferenceIntake({ active, full, upload }: {
  active: boolean;
  full: boolean;
  upload: (files: File[]) => Promise<void>;
}) {
  const [dropping, setDropping] = useState(false);
  const latest = useRef({ full, upload });
  latest.current = { full, upload };

  useEffect(() => {
    if (!active) return;
    // Entering a child fires enter before the parent's leave, so the drag is counted.
    let depth = 0;
    const take = (event: DragEvent) => carriesFiles(event) && !modalOpen();
    const enter = (event: DragEvent) => {
      if (!take(event)) return;
      depth += 1;
      setDropping(true);
    };
    const over = (event: DragEvent) => {
      if (!take(event)) return;
      // Claimed here, so the window's guard leaves it to the workbench.
      event.preventDefault();
      event.dataTransfer!.dropEffect = latest.current.full ? "none" : "copy";
    };
    const leave = (event: DragEvent) => {
      if (!carriesFiles(event)) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) setDropping(false);
    };
    const drop = (event: DragEvent) => {
      depth = 0;
      setDropping(false);
      if (!take(event)) return;
      event.preventDefault();
      if (!latest.current.full) void latest.current.upload(pictures(Array.from(event.dataTransfer!.files)));
    };
    const paste = (event: ClipboardEvent) => {
      const data = event.clipboardData;
      const files = Array.from(data?.files ?? []);
      if (!data || files.length === 0 || modalOpen()) return;
      // Text pasted into a field stays text.
      if (isEditable(event.target) && data.types.includes("text/plain")) return;
      const taken = files.filter(isPicture);
      // Nothing here is a picture: the paste goes where it was meant to.
      if (taken.length === 0 && isEditable(event.target)) return;
      event.preventDefault();
      if (latest.current.full) {
        toast({ title: "参考图已满", variant: "destructive" });
        return;
      }
      void latest.current.upload(pictures(files));
    };
    // Capture: the workbench claims a drop before the window's guard refuses it.
    window.addEventListener("dragenter", enter, true);
    window.addEventListener("dragover", over, true);
    window.addEventListener("dragleave", leave, true);
    window.addEventListener("drop", drop, true);
    document.addEventListener("paste", paste);
    return () => {
      window.removeEventListener("dragenter", enter, true);
      window.removeEventListener("dragover", over, true);
      window.removeEventListener("dragleave", leave, true);
      window.removeEventListener("drop", drop, true);
      document.removeEventListener("paste", paste);
      setDropping(false);
    };
  }, [active]);

  return { dropping };
}
