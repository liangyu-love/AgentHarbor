/**
 * Feeds the pointer position to the card under it as `--mx` / `--my`, which
 * `[data-spotlight]` styles turn into a soft light. One listener for the app,
 * at most one style write per frame.
 */
export function installSpotlight() {
  let frame = 0;
  let last: PointerEvent | null = null;
  const apply = () => {
    frame = 0;
    const target = last?.target instanceof Element ? last.target.closest<HTMLElement>("[data-spotlight]") : null;
    if (!last || !target) return;
    const rect = target.getBoundingClientRect();
    target.style.setProperty("--mx", `${last.clientX - rect.left}px`);
    target.style.setProperty("--my", `${last.clientY - rect.top}px`);
  };
  const move = (event: PointerEvent) => {
    last = event;
    if (!frame) frame = requestAnimationFrame(apply);
  };
  window.addEventListener("pointermove", move, { passive: true });
  return () => {
    window.removeEventListener("pointermove", move);
    cancelAnimationFrame(frame);
  };
}
