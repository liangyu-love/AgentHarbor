import { cn } from "@/lib/utils";
import { useThemeValue } from "@/hooks/use-theme";
import { Skeleton } from "@/components/ui/skeleton";

export type GlowColor = "blue" | "amber" | "green" | "red" | "purple" | "gold" | "blackgold";

export interface ColorTokens {
  bar: string;
  strong: [string, string]; // [light, dark]
  mid: [string, string];
  edge: [string, string];
}

export const glowPalette: Record<GlowColor, ColorTokens> = {
  blue: {
    bar: "#3b82f6",
    strong: ["rgba(59,130,246,0.10)", "rgba(59,130,246,0.22)"],
    mid: ["rgba(59,130,246,0.04)", "rgba(59,130,246,0.08)"],
    edge: ["rgba(59,130,246,0.02)", "rgba(59,130,246,0.05)"],
  },
  amber: {
    bar: "#f59e0b",
    strong: ["rgba(245,158,11,0.10)", "rgba(245,158,11,0.22)"],
    mid: ["rgba(245,158,11,0.04)", "rgba(245,158,11,0.08)"],
    edge: ["rgba(245,158,11,0.02)", "rgba(245,158,11,0.05)"],
  },
  green: {
    bar: "#10b981",
    strong: ["rgba(16,185,129,0.10)", "rgba(16,185,129,0.22)"],
    mid: ["rgba(16,185,129,0.04)", "rgba(16,185,129,0.08)"],
    edge: ["rgba(16,185,129,0.03)", "rgba(16,185,129,0.06)"],
  },
  red: {
    bar: "#ef4444",
    strong: ["rgba(239,68,68,0.10)", "rgba(239,68,68,0.22)"],
    mid: ["rgba(239,68,68,0.04)", "rgba(239,68,68,0.08)"],
    edge: ["rgba(239,68,68,0.02)", "rgba(239,68,68,0.05)"],
  },
  purple: {
    bar: "#8b5cf6",
    strong: ["rgba(139,92,246,0.10)", "rgba(139,92,246,0.22)"],
    mid: ["rgba(139,92,246,0.04)", "rgba(139,92,246,0.08)"],
    edge: ["rgba(139,92,246,0.02)", "rgba(139,92,246,0.05)"],
  },
  gold: {
    bar: "#d4af37",
    strong: ["rgba(217,119,6,0.12)", "rgba(245,158,11,0.32)"],
    mid: ["rgba(217,119,6,0.05)", "rgba(212,175,55,0.16)"],
    edge: ["rgba(217,119,6,0.02)", "rgba(245,158,11,0.08)"],
  },
  blackgold: {
    bar: "#d4af37",
    strong: ["rgba(217,119,6,0.14)", "rgba(245,158,11,0.38)"],
    mid: ["rgba(217,119,6,0.06)", "rgba(212,175,55,0.18)"],
    edge: ["rgba(217,119,6,0.03)", "rgba(245,158,11,0.10)"],
  },
};

interface GlowCardProps {
  color: GlowColor;
  label: string;
  value: string | number;
  sub: string;
  loading?: boolean;
  className?: string;
}

export function GlowCard({ color, label, value, sub, loading = false, className }: GlowCardProps) {
  const c = glowPalette[color] ?? glowPalette.blue;
  const isDark = useThemeValue() === "dark";
  const i = isDark ? 1 : 0;
  const isGold = color === "gold" || color === "blackgold";

  return (
    <div
      className={cn(
        "group relative overflow-hidden rounded-[15px] border border-border bg-card transition-all duration-300 ease-out hover:-translate-y-0.5 hover:shadow-lg",
        isGold && isDark && "border-[#d4af37]/35 bg-gradient-to-br from-[#18150f] via-[#13110d] to-[#0c0a08] shadow-[0_12px_32px_-16px_rgba(212,175,55,0.22)] hover:border-[#d4af37]/65",
        className,
      )}
      style={{
        padding: "24px 24px 20px",
        "--g2-color": isGold && isDark ? "#e5b85c" : c.bar,
        "--g2-glow-strong": c.strong[i],
        "--g2-glow-mid": c.mid[i],
        "--g2-glow-edge": c.edge[i],
      } as React.CSSProperties}
    >
      {/* Color bar — top-left, 40px, dynamic expand & shadow on hover */}
      <div
        className="absolute left-5 top-0 h-[3px] w-10 rounded-b-[2px] transition-all duration-300 group-hover:w-16 group-hover:shadow-[0_0_12px_var(--g2-color)]"
        style={{
          background: isGold && isDark ? "linear-gradient(90deg, #b45309, #fef08a, #d4af37)" : c.bar,
        }}
      />

      {/* Primary glow — large diffuse aurora from top-left */}
      <div
        className="glow-card-aurora transition-all duration-500 group-hover:opacity-100 group-hover:scale-105"
        style={{
          position: "absolute",
          top: -60,
          left: -40,
          width: 280,
          height: 200,
          borderRadius: "50%",
          pointerEvents: "none",
          opacity: 0.9,
        }}
      />

      {/* Secondary edge glow — subtle right-side reflection */}
      <div
        className="glow-card-edge transition-all duration-500 group-hover:opacity-90"
        style={{
          position: "absolute",
          top: -20,
          right: -60,
          width: 120,
          height: 140,
          borderRadius: "50%",
          pointerEvents: "none",
        }}
      />

      {/* Content */}
      <div className="relative z-10">
        <p className="text-[11px] font-bold uppercase tracking-wider text-muted-foreground">
          {label}
        </p>
        {loading ? (
          <>
            <Skeleton className="mt-2.5 h-7 w-16" />
            <Skeleton className="mt-2 h-3 w-28" />
          </>
        ) : (
          <>
            <p className="mt-2.5 text-[28px] font-bold leading-none tracking-tight">
              {value}
            </p>
            <p className="mt-2 truncate text-xs text-muted-foreground">{sub}</p>
          </>
        )}
      </div>
    </div>
  );
}
