import type { CSSProperties } from "react";

const MIN_COLUMN_WIDTH = 250;
const GAP = 12;
const ESTIMATED_CARD_HEIGHT = 180;

export const threadGridStyle: CSSProperties = {
  gridTemplateColumns: `repeat(auto-fill, minmax(min(100%, ${MIN_COLUMN_WIDTH}px), 1fr))`,
  gap: GAP,
};

// Only an offscreen placeholder. Normal flow and `auto` intrinsic sizing let
// the browser remember the real height after laying out wrapped cards.
export function estimateThreadGridHeight(itemCount: number, width: number) {
  if (itemCount <= 0) return 0;
  const columns = Math.max(1, Math.floor((width + GAP) / (MIN_COLUMN_WIDTH + GAP)));
  const rows = Math.ceil(itemCount / columns);
  return rows * ESTIMATED_CARD_HEIGHT + Math.max(0, rows - 1) * GAP;
}
