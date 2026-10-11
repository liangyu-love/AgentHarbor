import { useCallback, useId, useLayoutEffect, useRef, useState, type RefObject } from "react";

import { reducedMotion } from "@/lib/motion";

type Box = { x: number; y: number; w: number; h: number };
export type Anchors = { width: number; height: number; boxes: Record<string, Box> };

/**
 * Measures registered nodes inside `container` so lines can join them. Re-measures
 * when the container resizes; boxes are relative to the container.
 */
export function useAnchors(container: RefObject<HTMLElement | null>) {
  const nodes = useRef(new Map<string, HTMLElement>());
  const [anchors, setAnchors] = useState<Anchors | null>(null);
  const measure = useCallback(() => {
    const root = container.current;
    if (!root) return;
    const origin = root.getBoundingClientRect();
    const boxes: Record<string, Box> = {};
    for (const [id, node] of nodes.current) {
      const rect = node.getBoundingClientRect();
      boxes[id] = { x: rect.left - origin.left, y: rect.top - origin.top, w: rect.width, h: rect.height };
    }
    setAnchors({ width: origin.width, height: origin.height, boxes });
  }, [container]);
  useLayoutEffect(() => {
    measure();
    const root = container.current;
    if (!root || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(root);
    return () => observer.disconnect();
  }, [container, measure]);
  const register = useCallback((id: string) => (node: HTMLElement | null) => {
    if (node) nodes.current.set(id, node);
    else nodes.current.delete(id);
  }, []);
  return { anchors, register, measure };
}

/** A soft S-curve from one node's edge to another's, across or down the board. */
export function linkPath(anchors: Anchors | null, from: string, to: string, vertical = false): string | null {
  const a = anchors?.boxes[from];
  const b = anchors?.boxes[to];
  if (!a || !b) return null;
  if (vertical) {
    const [x1, y1, x2, y2] = [a.x + a.w / 2, a.y + a.h, b.x + b.w / 2, b.y];
    const mid = (y1 + y2) / 2;
    return `M${x1},${y1} C${x1},${mid} ${x2},${mid} ${x2},${y2}`;
  }
  const [x1, y1, x2, y2] = [a.x + a.w, a.y + a.h / 2, b.x, b.y + b.h / 2];
  const mid = (x1 + x2) / 2;
  return `M${x1},${y1} C${mid},${y1} ${mid},${y2} ${x2},${y2}`;
}

/**
 * One signal line: a dashed hairline while idle; once live it draws itself in,
 * carries a moving dash and a packet travelling toward its end. `ignition`
 * replays the draw (a state change re-lights the line).
 */
export function SignalPath({ d, live, ignition = 0, packets = 1, speed = 1, tone = "primary" }: {
  d: string | null;
  live: boolean;
  ignition?: number;
  packets?: number;
  speed?: number;
  tone?: "primary" | "muted";
}) {
  const id = useId().replace(/:/g, "");
  if (!d) return null;
  const color = tone === "primary" ? "hsl(var(--primary))" : "hsl(var(--muted-foreground))";
  const duration = 1.8 / speed;
  const reduced = reducedMotion();
  return (
    <g data-signal={live ? "live" : "idle"}>
      <path d={d} fill="none" className="stroke-border" strokeWidth={1.25} strokeDasharray={live ? undefined : "3 6"} opacity={live ? 0.4 : 1} />
      {live && <>
        <path key={`draw-${ignition}`} d={d} fill="none" stroke={color} strokeWidth={1.75} strokeLinecap="round" pathLength={1} className="route-draw" />
        <path d={d} fill="none" stroke="hsl(var(--card))" strokeOpacity={0.85} strokeWidth={1.25} strokeLinecap="round" className="route-flow" />
        {!reduced && Array.from({ length: packets }, (_, index) => (
          <circle key={`${id}-${index}-${d}`} r={2.4} fill={color}>
            <animateMotion dur={`${duration}s`} begin={`${-(index / packets) * duration}s`} repeatCount="indefinite" path={d} />
          </circle>
        ))}
      </>}
    </g>
  );
}
