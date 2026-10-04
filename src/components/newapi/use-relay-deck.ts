import { useLayoutEffect, useRef, useState } from "react";

type Direction = "prev" | "next";
type Turn = { origin: string; id: string; direction: Direction };

// Only visual timing lives here. Provider selection and all provider operations
// stay in their existing stores. No duplicate interactive card is rendered.
export function useRelayDeck({ activeId, memberIds, stacked, onSelect }: {
  activeId: string;
  memberIds: string[];
  stacked: boolean;
  onSelect: (id: string, direction: Direction) => void;
}) {
  const [pending, setPending] = useState<Turn | null>(null);
  const pendingRef = useRef<Turn | null>(null);
  const select = useRef(onSelect);
  useLayoutEffect(() => { select.current = onSelect; }, [onSelect]);
  const targetExists = !!pending && memberIds.includes(pending.id);

  // Cancel before paint when refresh, expansion or a changed selection removes
  // either end of a turn. The timer must never select a removed member later.
  useLayoutEffect(() => {
    if (!pending) return;
    if (!stacked || pending.origin !== activeId || !targetExists) {
      pendingRef.current = null;
      setPending(null);
      return;
    }
    const timer = window.setTimeout(() => {
      pendingRef.current = null;
      setPending(null);
      select.current(pending.id, pending.direction);
    }, 180);
    return () => window.clearTimeout(timer);
  }, [pending, activeId, stacked, targetExists]);

  const turn = (id: string, direction: Direction) => {
    if (pendingRef.current || id === activeId || !memberIds.includes(id)) return;
    if (!stacked || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      select.current(id, direction);
    } else {
      const next = { origin: activeId, id, direction };
      pendingRef.current = next;
      setPending(next);
    }
  };
  return { turn, exitDirection: stacked && targetExists && pending?.origin === activeId ? pending.direction : null };
}
