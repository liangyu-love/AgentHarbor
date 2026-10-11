import { useEffect, useRef, useState } from "react";

/** The user asked the system for less motion. */
export const reducedMotion = () =>
  typeof window !== "undefined" && (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false);

const GLYPHS = "abcdefghijklmnopqrstuvwxyz0123456789";

/**
 * Text that decodes into a new value: characters settle left to right while
 * the rest flicker through the key alphabet, so a revealed or replaced secret
 * reads as being unlocked rather than swapped. Instant under reduced motion.
 */
export function useDecodedText(target: string, duration = 520) {
  const [text, setText] = useState(target);
  const shown = useRef(target);
  useEffect(() => {
    const from = shown.current;
    if (from === target || reducedMotion()) {
      shown.current = target;
      setText(target);
      return;
    }
    const start = performance.now();
    const length = Math.max(from.length, target.length);
    let frame = 0;
    const tick = (now: number) => {
      const progress = Math.min(1, (now - start) / duration);
      const settled = Math.floor(progress * target.length);
      const size = Math.round(from.length + (target.length - from.length) * progress);
      let next = "";
      for (let index = 0; index < Math.min(size, length); index += 1) {
        const fixed = target[index];
        next += index < settled || fixed === undefined || "-…".includes(fixed)
          ? fixed ?? ""
          : GLYPHS[Math.floor(Math.random() * GLYPHS.length)];
      }
      shown.current = progress >= 1 ? target : next;
      setText(shown.current);
      if (progress < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(frame);
      shown.current = target;
    };
  }, [target, duration]);
  return text;
}
