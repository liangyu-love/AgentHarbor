import { useEffect, useRef, useState } from "react";

import { reducedMotion } from "@/lib/motion";

/** A number that eases from its previous value (0 at first) to `value`. */
export function CountUp({ value, duration = 700, format = (n: number) => String(Math.round(n)), className }: {
  value: number;
  duration?: number;
  format?: (value: number) => string;
  className?: string;
}) {
  const [shown, setShown] = useState(() => reducedMotion() ? value : 0);
  const from = useRef(shown);
  useEffect(() => {
    if (reducedMotion()) {
      from.current = value;
      setShown(value);
      return;
    }
    const origin = from.current;
    const start = performance.now();
    let frame = 0;
    const tick = (now: number) => {
      const progress = Math.min(1, (now - start) / duration);
      const current = origin + (value - origin) * (1 - (1 - progress) ** 3);
      from.current = current;
      setShown(current);
      if (progress < 1) frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [value, duration]);
  return <span className={className}>{format(shown)}</span>;
}
