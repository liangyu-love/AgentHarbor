import type { CSSProperties, HTMLAttributes } from "react";

import { useThemeValue } from "@/hooks/use-theme";
import { cn } from "@/lib/utils";
import { ShineBorder } from "@/components/ui/shine-border";
import { glowPalette, type GlowColor } from "@/components/ui/glow-card";

interface AuroraCardProps extends HTMLAttributes<HTMLDivElement> {
  color: GlowColor;
  shine?: boolean;
  shineColor?: string | string[];
  shineDuration?: number;
}

export function AuroraCard({
  color,
  shine = false,
  shineColor = "#f59e0b",
  shineDuration = 7,
  className,
  children,
  ...props
}: AuroraCardProps) {
  const c = glowPalette[color];
  const i = useThemeValue() === "dark" ? 1 : 0;

  return (
    <div className={cn("relative", className)} {...props}>
      <div
        className="relative h-full overflow-hidden rounded-2xl border border-border bg-card shadow-[0_1px_2px_rgba(15,23,42,0.04),0_14px_32px_-18px_rgba(15,23,42,0.14)] dark:shadow-[0_14px_32px_-18px_rgba(0,0,0,0.55)]"
        style={
          {
            "--g2-color": c.bar,
            "--g2-glow-strong": c.strong[i],
            "--g2-glow-mid": c.mid[i],
            "--g2-glow-edge": c.edge[i],
          } as CSSProperties
        }
      >
        <div
          aria-hidden
          className="absolute left-5 top-0 h-[3px] w-10 rounded-b-[2px]"
          style={{ background: c.bar }}
        />
        <div
          aria-hidden
          className="glow-card-aurora pointer-events-none absolute -left-10 -top-16 h-52 w-72 rounded-full opacity-90"
        />
        <div
          aria-hidden
          className="glow-card-edge pointer-events-none absolute -right-16 -top-6 h-36 w-40 rounded-full"
        />
        <div className="relative z-10 flex h-full flex-col">{children}</div>
      </div>
      {shine && (
        <ShineBorder
          borderWidth={1.5}
          duration={shineDuration}
          shineColor={shineColor}
          className="rounded-2xl"
        />
      )}
    </div>
  );
}
