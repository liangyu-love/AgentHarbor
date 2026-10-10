import type { CSSProperties, ReactNode } from "react";
import { cn } from "@/lib/utils";

/** Layers peek above the face inside space every deck reserves, so a stacked
 * site and a single relay have the same footprint. */
const MAX_LAYERS = 3;

// Inert backing surfaces share the live card's size. Only the current provider
// has a card face, so page turns cannot duplicate actions, IDs or focus targets.
export function RelayDeck({ count, stacked, hue, label, onNext, children }: {
  count: number;
  stacked: boolean;
  hue: number;
  label: string;
  onNext: (offset: number) => void;
  children: ReactNode;
}) {
  const depth = stacked ? Math.max(0, Math.min(count - 1, MAX_LAYERS)) : 0;
  return (
    <div
      className={cn("relay-deck group relative flex min-w-0 flex-col", stacked && "relay-deck-stacked")}
      style={{ "--relay-depth": depth, "--relay-hue": hue } as CSSProperties}
      data-relay-deck
      role={stacked ? "group" : undefined}
      tabIndex={stacked ? 0 : undefined}
      aria-label={stacked ? label : undefined}
      aria-keyshortcuts={stacked ? "ArrowLeft ArrowRight Enter Space" : undefined}
      onKeyDown={event => {
        if (!stacked || event.target !== event.currentTarget || event.altKey || event.ctrlKey || event.metaKey) return;
        if (["ArrowLeft", "ArrowRight", "Enter", " "].includes(event.key)) {
          event.preventDefault();
          onNext(event.key === "ArrowLeft" ? -1 : 1);
        }
      }}
      onClick={event => {
        if (!stacked || !(event.target instanceof Element) || !event.currentTarget.contains(event.target)) return;
        const interactive = event.target.closest("button, a, input, select, textarea, label, summary, [role=button], [role=switch], [role=checkbox], [role=menuitem], [contenteditable], [data-relay-no-turn]");
        if (interactive && event.currentTarget.contains(interactive)) return;
        if (window.getSelection()?.toString()) return;
        onNext(1);
      }}
    >
      <div className="relay-deck-stage">
        {Array.from({ length: depth }, (_, index) => depth - index).map(layer => (
          <div key={layer} aria-hidden className="relay-deck-layer"
            style={{ "--relay-layer": layer, zIndex: 5 - layer } as CSSProperties} />
        ))}
        {children}
      </div>
    </div>
  );
}
