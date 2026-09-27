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
        "group/aurora relative rounded-2xl border border-border bg-card transition-[border-color,box-shadow] duration-200 motion-reduce:transition-none",
        shine && "border-primary/45 ring-1 ring-primary/15",
        className,
      )}
      {...props}
    >
      <div className={cn("relative h-full overflow-hidden rounded-[inherit]", innerClassName)}>
        <span
          aria-hidden
          className="absolute left-5 top-0 h-0.5 w-8 rounded-b-sm opacity-75 transition-[width] duration-200 group-hover/aurora:w-12 motion-reduce:transition-none"
          style={{ backgroundColor: color === "green" || color === "red" || color === "amber" ? glowPalette[color].bar : "hsl(var(--primary))" }}
        />
        <div className="relative flex h-full flex-col">{children}</div>
      </div>
    </div>
  );
}
