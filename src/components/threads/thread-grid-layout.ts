import type { CSSProperties } from "react";

const MIN_COLUMN_WIDTH = 320;
const GAP = 12;
// Wide monitors must not turn a row into a wall of narrow cards. The floor
// term in the template caps the track count at MAX_COLUMNS while `auto-fill`
// still reserves empty tracks, so 1–2 card groups are never stretched.
const MAX_COLUMNS = 4;
const ESTIMATED_CARD_HEIGHT = 180;

export const threadGridStyle: CSSProperties = {
  gridTemplateColumns: `repeat(auto-fill, minmax(max(min(100%, ${MIN_COLUMN_WIDTH}px), calc((100% - ${GAP * (MAX_COLUMNS - 1)}px) / ${MAX_COLUMNS})), 1fr))`,
  gap: GAP,
};

// Mirrors how the browser resolves the template above: below the width where
// the 4-column floor term takes over (~1316px), tracks are plain
// `min(100%, 320px)` columns.
export function threadColumnCount(width: number) {
  return Math.min(
    MAX_COLUMNS,
    Math.max(1, Math.floor((width + GAP) / (MIN_COLUMN_WIDTH + GAP))),
  );
}

// Only an offscreen placeholder. Normal flow and `auto` intrinsic sizing let
// the browser remember the real height after laying out wrapped cards.
export function estimateThreadGridHeight(itemCount: number, width: number) {
  if (itemCount <= 0) return 0;
  const columns = threadColumnCount(width);
  const rows = Math.ceil(itemCount / columns);
  return rows * ESTIMATED_CARD_HEIGHT + Math.max(0, rows - 1) * GAP;
}
