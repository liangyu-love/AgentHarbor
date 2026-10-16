import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { Bot, Braces, Briefcase, Code, Command, Cpu, Feather, Ghost, Globe, Globe2, Leaf, Link2, MessageCircle, MousePointer2, Orbit, Rocket, Sparkles, SquareTerminal, Waves, Wind, Zap, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AccountSourceKind } from "@/types/account-sources";

export type DockValue = "codex" | AccountSourceKind;

/** One account's light on its berth. */
export type BerthLight = "active" | "ready" | "lapsed";

export interface Berth {
  value: DockValue;
  name: string;
  lights: BerthLight[];
  /** Shared with model inject (account sources only). */
  shared: boolean;
}

/** Channels grouped by who runs them; the order is also the keyboard order. */
const PIERS: { id: string; berths: DockValue[] }[] = [
  { id: "openai", berths: ["codex", "chatgpt-api"] },
  { id: "tencent", berths: ["workbuddy", "workbuddy-ai"] },
  { id: "github", berths: ["copilot"] },
  { id: "google", berths: ["gemini-cli", "antigravity"] },
  { id: "aws", berths: ["kiro"] },
  { id: "xai", berths: ["grok"] },
  { id: "cursor", berths: ["cursor"] },
  { id: "alibaba", berths: ["qoder", "qoder-cn"] },
  { id: "bytedance", berths: ["trae-cn", "trae-global"] },
  { id: "minimax", berths: ["minimax-code", "minimax-code-global"] },
  { id: "zhipu", berths: ["zcode"] },
  { id: "opencode", berths: ["opencode-zen", "opencode-go"] },
  { id: "commandcode", berths: ["commandcode"] },
];
export const DOCK_ORDER: DockValue[] = PIERS.flatMap(pier => pier.berths);

const GLYPHS: Record<DockValue, LucideIcon> = {
  codex: SquareTerminal,
  "chatgpt-api": MessageCircle,
  workbuddy: Briefcase,
  "workbuddy-ai": Globe2,
  copilot: Bot,
  "gemini-cli": Sparkles,
  antigravity: Orbit,
  kiro: Ghost,
  grok: Zap,
  cursor: MousePointer2,
  qoder: Braces,
  "qoder-cn": Code,
  "trae-cn": Feather,
  "trae-global": Wind,
  "minimax-code": Waves,
  "minimax-code-global": Globe,
  zcode: Cpu,
  "opencode-zen": Leaf,
  "opencode-go": Rocket,
  commandcode: Command,
};

/** At most this many lights; the rest read as "+n". */
const MAX_LIGHTS = 6;

type Hull = { left: number; right: number; top: number; bottom: number; center: number; ready: boolean };

/**
 * The account channels as berths along their vendors' piers. Each berth shows
 * one light per account (current, standing by, expired) and whether it is
 * shared with model inject, so the whole harbor reads at a glance. The chosen
 * berth's hull flows to the next one, leading edge first, and a mooring line
 * drops from the harbor's edge below it to the panel it opens.
 */
export function SourceDock({ berths, value, onChange }: {
  berths: Berth[];
  value: DockValue;
  onChange: (value: DockValue) => void;
}) {
  const { t } = useTranslation();
  const board = useRef<HTMLDivElement>(null);
  const buttons = useRef(new Map<DockValue, HTMLButtonElement | null>());
  const [hull, setHull] = useState<Hull>({ left: 0, right: 0, top: 0, bottom: 0, center: 0, ready: false });
  const [heading, setHeading] = useState<"forward" | "back">("forward");
  // The hull appears in place first, and flows only between later choices.
  const [afloat, setAfloat] = useState(false);
  const previous = useRef(value);
  const present = new Map(berths.map(berth => [berth.value, berth]));
  const order = DOCK_ORDER.filter(entry => present.has(entry));

  const measure = useCallback(() => {
    const container = board.current;
    const button = buttons.current.get(value);
    if (!container || !button) return;
    const box = container.getBoundingClientRect();
    const rect = button.getBoundingClientRect();
    setHull({
      left: rect.left - box.left,
      right: box.right - rect.right,
      top: rect.top - box.top,
      bottom: box.bottom - rect.bottom,
      center: rect.left - box.left + rect.width / 2,
      ready: true,
    });
  }, [value]);

  useLayoutEffect(() => {
    if (previous.current !== value) {
      setHeading(order.indexOf(value) >= order.indexOf(previous.current) ? "forward" : "back");
      previous.current = value;
    }
    measure();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value, measure, berths.length]);

  useEffect(() => {
    if (!hull.ready || afloat) return;
    const frame = requestAnimationFrame(() => setAfloat(true));
    return () => cancelAnimationFrame(frame);
  }, [hull.ready, afloat]);

  useEffect(() => {
    const container = board.current;
    if (!container || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(container);
    return () => observer.disconnect();
  }, [measure]);

  const move = (event: KeyboardEvent<HTMLButtonElement>) => {
    const index = order.indexOf(value);
    const next = {
      ArrowRight: index + 1, ArrowDown: index + 1, ArrowLeft: index - 1, ArrowUp: index - 1, Home: 0, End: order.length - 1,
    }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    const target = order[(next + order.length) % order.length];
    onChange(target);
    buttons.current.get(target)?.focus();
  };

  const hullStyle = {
    left: hull.left, right: hull.right, top: hull.top, bottom: hull.bottom,
    // The edge in the direction of travel leaves first; the other catches up.
    "--lag-left": heading === "forward" ? "70ms" : "0ms",
    "--lag-right": heading === "forward" ? "0ms" : "70ms",
  } as CSSProperties;

  return (
    <div ref={board} role="radiogroup" aria-label={t("accountSources.tabs")} data-accounts-tabs data-source-dock
      className="source-dock relative rounded-2xl border border-border/70 bg-card/70 p-2 shadow-sm backdrop-blur-md">
      <span aria-hidden className={cn("source-dock-hull", hull.ready && "is-ready", afloat && "is-afloat")} style={hullStyle} />
      {/* Dropped from the harbor's edge below the chosen berth, so a wrapped row is never crossed. */}
      {hull.ready && (
        <span key={value} aria-hidden data-dock-mooring className="source-dock-mooring" style={{ left: hull.center }} />
      )}
      <div className="source-piers relative flex flex-wrap gap-y-2">
        {PIERS.map(pier => {
          const docked = pier.berths.filter(entry => present.has(entry));
          if (docked.length === 0) return null;
          return (
            <div key={pier.id} data-pier={pier.id} className="source-pier min-w-0">
              <p className="source-pier-label">{t(`accountSources.vendor.${pier.id}`)}</p>
              <div className="flex flex-wrap gap-1">
                {docked.map(entry => {
                  const berth = present.get(entry)!;
                  const Glyph = GLYPHS[entry];
                  const chosen = entry === value;
                  const shown = berth.lights.slice(0, MAX_LIGHTS);
                  return (
                    <button key={entry} type="button" role="radio" aria-checked={chosen} tabIndex={chosen ? 0 : -1}
                      ref={node => { buttons.current.set(entry, node); }}
                      aria-label={t("accountSources.berth", { name: berth.name, count: berth.lights.length })}
                      data-berth={entry} data-chosen={chosen || undefined}
                      onClick={() => onChange(entry)} onKeyDown={move}
                      className="source-berth">
                      <span key={chosen ? `${entry}-docked` : entry} className={cn("source-berth-glyph", chosen && "is-docking")} data-pier-tint={pier.id}>
                        <Glyph className="h-4 w-4" strokeWidth={2} />
                      </span>
                      <span className="min-w-0 flex-1 text-left">
                        <span className="flex items-center gap-1">
                          <span className="truncate text-[13px] font-semibold leading-5">{berth.name}</span>
                          {berth.shared && <Link2 aria-hidden className="h-3 w-3 shrink-0 text-primary" data-berth-shared />}
                        </span>
                        <span className="flex h-3 items-center gap-[3px]" data-berth-lights={berth.lights.length}>
                          {berth.lights.length === 0
                            ? <span className="text-[10px] leading-3 text-muted-foreground">{t("accountSources.berthEmpty")}</span>
                            : <>
                              {shown.map((light, index) => (
                                <span key={index} className="source-berth-light" data-light={light}
                                  style={{ animationDelay: chosen ? `${index * 45}ms` : undefined }} />
                              ))}
                              {berth.lights.length > MAX_LIGHTS && (
                                <span className="font-mono text-[9px] leading-3 text-muted-foreground">+{berth.lights.length - MAX_LIGHTS}</span>
                              )}
                            </>}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
