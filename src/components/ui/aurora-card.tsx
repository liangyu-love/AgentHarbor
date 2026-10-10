import type { HTMLAttributes } from "react";

import { cn } from "@/lib/utils";
import { glowPalette, type GlowColor } from "@/components/ui/glow-card";

interface AuroraCardProps extends HTMLAttributes<HTMLDivElement> {
  color: GlowColor;
  shine?: boolean;
  innerClassName?: string;
}

export function AuroraCard({
  color,
  shine = false,
  className,
  innerClassName,
  children,
  ...props
}: AuroraCardProps) {
  return (
    <div
      className={cn(
        "group/aurora relative rounded-[22px] bg-card shadow-[0_0_0_1px_hsl(var(--border)/0.85)] transition-shadow duration-200 motion-reduce:transition-none",
        shine && "shadow-[0_0_0_1px_hsl(var(--primary)/0.5)]",
        className,
      )}
      {...props}
    >
      <div className={cn("relative h-full overflow-hidden rounded-[inherit]", innerClassName)}>
        {/* Only status colors keep a marker; neutral cards stay plain. */}
        {(color === "green" || color === "red" || color === "amber") && <span
          aria-hidden
          className="absolute left-5 top-0 h-0.5 w-8 rounded-b-sm opacity-75"
          style={{ backgroundColor: glowPalette[color].bar }}
        />}
        <div className="relative flex h-full flex-col">{children}</div>
      </div>
    </div>
  );
}
