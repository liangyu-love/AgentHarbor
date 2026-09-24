import type { CSSProperties, HTMLAttributes } from "react";

import { useThemeValue } from "@/hooks/use-theme";
import { cn } from "@/lib/utils";
import { ShineBorder } from "@/components/ui/shine-border";
import { glowPalette, type GlowColor } from "@/components/ui/glow-card";

export const LIQUID_GOLD_SHINE = [
  "#b45309",
  "#fef08a",
  "#d4af37",
  "#fef9c3",
  "#b45309",
];

interface AuroraCardProps extends HTMLAttributes<HTMLDivElement> {
  color: GlowColor;
  shine?: boolean;
  shineColor?: string | string[];
  shineDuration?: number;
  innerClassName?: string;
}

export function AuroraCard({
  color,
  shine = false,
  shineColor,
  shineDuration = 7,
  className,
  innerClassName,
  children,
  ...props
}: AuroraCardProps) {
  const c = glowPalette[color] ?? glowPalette.blue;
  const isDark = useThemeValue() === "dark";
  const i = isDark ? 1 : 0;
  const isGold = color === "gold" || color === "blackgold";
  const resolvedShineColor = shineColor ?? (isGold ? LIQUID_GOLD_SHINE : "#f59e0b");

  return (
    <div className={cn("group/aurora relative transition-all duration-300 ease-out", className)} {...props}>
      <div
        className={cn(
          "relative h-full overflow-hidden rounded-2xl border border-border bg-card shadow-[0_1px_2px_rgba(15,23,42,0.04),0_14px_32px_-18px_rgba(15,23,42,0.14)] dark:shadow-[0_14px_32px_-18px_rgba(0,0,0,0.55)] transition-all duration-300",
          isGold && isDark && "border-[#d4af37]/35 bg-gradient-to-br from-[#191610] via-[#13110e] to-[#0c0b09] shadow-[0_14px_36px_-18px_rgba(212,175,55,0.25)] hover:border-[#d4af37]/65",
          innerClassName,
        )}
        style={
          {
            "--g2-color": isGold && isDark ? "#e5b85c" : c.bar,
            "--g2-glow-strong": c.strong[i],
            "--g2-glow-mid": c.mid[i],
            "--g2-glow-edge": c.edge[i],
          } as CSSProperties
        }
      >
        <div
          aria-hidden
          className={cn(
            "absolute left-5 top-0 h-[3px] w-10 rounded-b-[2px] transition-all duration-300 group-hover/aurora:w-16",
            isGold && isDark ? "shadow-[0_0_12px_rgba(245,158,11,0.5)]" : "group-hover/aurora:shadow-[0_0_10px_var(--g2-color)]",
          )}
          style={{
            background: isGold && isDark ? "linear-gradient(90deg, #b45309, #fef08a, #d4af37)" : c.bar,
          }}
        />
        <div
          aria-hidden
          className="glow-card-aurora pointer-events-none absolute -left-10 -top-16 h-52 w-72 rounded-full opacity-90 transition-all duration-500 group-hover/aurora:opacity-100 group-hover/aurora:scale-105"
        />
        <div
          aria-hidden
          className="glow-card-edge pointer-events-none absolute -right-16 -top-6 h-36 w-40 rounded-full transition-all duration-500 group-hover/aurora:opacity-90"
        />
        <div className="relative z-10 flex h-full flex-col">{children}</div>
      </div>
      {shine && (
        <ShineBorder
          borderWidth={1.5}
          duration={shineDuration}
          shineColor={resolvedShineColor}
          className="rounded-2xl"
        />
      )}
    </div>
  );
}
