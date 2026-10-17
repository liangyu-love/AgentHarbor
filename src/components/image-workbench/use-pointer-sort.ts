import { useRef, useState, type KeyboardEvent, type PointerEvent, type RefObject } from "react";

/** Pointer travel before a press turns into a drag, so clicks still click. */
const DRAG_THRESHOLD = 4;
/** Distance from a scroll container's edge that scrolls it while dragging. */
const EDGE_SCROLL_ZONE = 28;

type Press = { index: number; x: number; y: number; pointerId: number };

/**
 * Reorder a list by dragging with the pointer, or with the arrow keys on a focused handle.
 * HTML5 drag and drop does not reach the page in Tauri's WebView2 on Windows,
 * so this follows pointer events and hit-tests the items itself.
 */
export function usePointerSort({ onMove, disabled = false, scrollRef }: {
  onMove: (from: number, to: number) => void;
  disabled?: boolean;
  scrollRef?: RefObject<HTMLElement>;
}) {
  const [drag, setDrag] = useState<{ from: number; over: number } | null>(null);
  const items = useRef(new Map<number, HTMLElement>());
  const press = useRef<Press | null>(null);

  const hit = (x: number, y: number) => {
    for (const [index, node] of items.current) {
      const rect = node.getBoundingClientRect();
      if (x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom) return index;
    }
    return null;
  };

  const edgeScroll = (y: number) => {
    const container = scrollRef?.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    if (y < rect.top + EDGE_SCROLL_ZONE) container.scrollBy({ top: -12 });
    else if (y > rect.bottom - EDGE_SCROLL_ZONE) container.scrollBy({ top: 12 });
  };

  const finish = () => {
    press.current = null;
    setDrag(null);
  };

  /** Attach to each sortable item so the pointer can be hit-tested against it. */
  const itemRef = (index: number) => (node: HTMLElement | null) => {
    if (node) items.current.set(index, node);
    else items.current.delete(index);
  };

  /** Spread onto the element that starts a drag: a grip, or the whole item. */
  const handleProps = (index: number, count: number) => ({
    onPointerDown: (event: PointerEvent<HTMLElement>) => {
      if (disabled || event.button !== 0) return;
      event.currentTarget.setPointerCapture(event.pointerId);
      press.current = { index, x: event.clientX, y: event.clientY, pointerId: event.pointerId };
    },
    onPointerMove: (event: PointerEvent<HTMLElement>) => {
      const start = press.current;
      if (!start || start.pointerId !== event.pointerId) return;
      if (!drag) {
        if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD) return;
        setDrag({ from: start.index, over: start.index });
        return;
      }
      edgeScroll(event.clientY);
      const over = hit(event.clientX, event.clientY);
      if (over !== null && over !== drag.over) setDrag({ ...drag, over });
    },
    onPointerUp: () => {
      if (drag && drag.over !== drag.from) onMove(drag.from, drag.over);
      finish();
    },
    onPointerCancel: finish,
    onLostPointerCapture: () => {
      if (!drag) press.current = null;
    },
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      if (disabled || !event.altKey) return;
      const step = event.key === "ArrowUp" || event.key === "ArrowLeft" ? -1
        : event.key === "ArrowDown" || event.key === "ArrowRight" ? 1 : 0;
      const to = index + step;
      if (!step || to < 0 || to >= count) return;
      event.preventDefault();
      onMove(index, to);
    },
  });

  return { drag, itemRef, handleProps };
}
