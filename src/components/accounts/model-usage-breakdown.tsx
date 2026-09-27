import { useState } from "react";
import { ChartPie, KeyRound, List } from "lucide-react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

type UsageMeasure = { totalTokens: number };
type UsageItem = {
  key: string;
  title: string;
  subtitle?: string;
  totals: UsageMeasure;
};

type DisplayItem = UsageItem & {
  memberCount: number;
  providerLabels: string[];
};

type ModelColor = {
  stroke: string;
  dot: string;
  bar: string;
  text: string;
};

const MODEL_COLORS: ModelColor[] = [
  { stroke: "stroke-primary", dot: "bg-primary", bar: "bg-primary", text: "text-primary" },
  {
    stroke: "stroke-sky-500",
    dot: "bg-sky-500",
    bar: "bg-sky-500",
    text: "text-sky-600 dark:text-sky-400",
  },
  {
    stroke: "stroke-emerald-500",
    dot: "bg-emerald-500",
    bar: "bg-emerald-500",
    text: "text-emerald-600 dark:text-emerald-400",
  },
  {
    stroke: "stroke-violet-500",
    dot: "bg-violet-500",
    bar: "bg-violet-500",
    text: "text-violet-600 dark:text-violet-400",
  },
  {
    stroke: "stroke-rose-500",
    dot: "bg-rose-500",
    bar: "bg-rose-500",
    text: "text-rose-600 dark:text-rose-400",
  },
  {
    stroke: "stroke-amber-500",
    dot: "bg-amber-500",
    bar: "bg-amber-500",
    text: "text-amber-600 dark:text-amber-400",
  },
];

const OTHER_COLOR: ModelColor = {
  stroke: "stroke-slate-400",
  dot: "bg-slate-400",
  bar: "bg-slate-400",
  text: "text-slate-500",
};

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

function formatPercent(value: number, total: number) {
  if (total <= 0) return "0%";
  return Math.round((value / total) * 100) + "%";
}

function cleanModelTitle(title: string) {
  if (!title.startsWith("aimami_relay_")) return title;

  const relayModel = title.split("::").pop();
  if (relayModel && relayModel !== title) return relayModel;

  const parts = title.split(" ");
  return parts.length > 1 ? parts.slice(1).join(" ") : title;
}

function prepareItems(items: UsageItem[], mergeDuplicateModels: boolean): DisplayItem[] {
  if (!mergeDuplicateModels) {
    return items.map((item) => ({
      ...item,
      memberCount: 1,
      providerLabels: item.subtitle ? [item.subtitle] : [],
    }));
  }

  const groups = new Map<string, DisplayItem>();
  for (const item of items) {
    const modelTitle = cleanModelTitle(item.title);
    const groupKey = modelTitle.trim().toLowerCase();
    const current = groups.get(groupKey);
    if (!current) {
      groups.set(groupKey, {
        ...item,
        title: modelTitle,
        totals: { totalTokens: item.totals.totalTokens },
        memberCount: 1,
        providerLabels: item.subtitle ? [item.subtitle] : [],
      });
      continue;
    }

    current.totals.totalTokens += item.totals.totalTokens;
    current.memberCount += 1;
    current.key += "+" + item.key;
    if (item.subtitle && !current.providerLabels.includes(item.subtitle)) {
      current.providerLabels.push(item.subtitle);
    }
  }
  return Array.from(groups.values());
}

export function UsageBreakdown({
  title,
  items,
  emptyLabel,
  mergeDuplicateModels = false,
  embedded = false,
}: {
  title: string;
  items: UsageItem[];
  emptyLabel: string;
  mergeDuplicateModels?: boolean;
  embedded?: boolean;
}) {
  const { t } = useTranslation();
  const [view, setView] = useState<"chart" | "list">("chart");
  const preparedItems = prepareItems(items, mergeDuplicateModels);
  const sortedItems = preparedItems.sort((left, right) => right.totals.totalTokens - left.totals.totalTokens);
  const visibleItems = sortedItems.slice(0, MODEL_COLORS.length);
  const hiddenItems = sortedItems.slice(MODEL_COLORS.length);
  const sum = sortedItems.reduce((total, item) => total + item.totals.totalTokens, 0);
  const max = Math.max(1, ...visibleItems.map((item) => item.totals.totalTokens));
  const otherTokens = hiddenItems.reduce((total, item) => total + item.totals.totalTokens, 0);
  const circumference = Math.PI * 2 * 44;
  const donutParts = visibleItems.map((item, index) => ({
    key: item.key,
    value: item.totals.totalTokens,
    color: MODEL_COLORS[index],
  }));
  if (otherTokens > 0) {
    donutParts.push({ key: "other-models", value: otherTokens, color: OTHER_COLOR });
  }
  let donutOffset = 0;

  return (
    <section
      className={cn(
        "flex min-w-0 flex-col",
        !embedded && "rounded-2xl border border-border/80 bg-card/85 p-4 shadow-xs backdrop-blur-md sm:p-5",
      )}
    >
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/50 pb-3">
        <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-foreground/90">{title}</h4>
        <div className="flex items-center gap-2">
          {items.length > 0 && (
            <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-[11px] font-semibold text-foreground">
              {formatTokens(sum)}
            </span>
          )}
          {items.length > 0 && (
            <div className="inline-flex rounded-lg bg-muted/70 p-0.5 ring-1 ring-border/60" role="group" aria-label={title}>
              <button
                type="button"
                onClick={() => setView("chart")}
                className={cn(
                  "inline-flex h-7 items-center gap-1 rounded-md px-2 text-[10px] font-medium transition-colors",
                  view === "chart" ? "bg-background text-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
                )}
                aria-pressed={view === "chart"}
              >
                <ChartPie className="h-3 w-3" />
                {t("accounts.chartView")}
              </button>
              <button
                type="button"
                onClick={() => setView("list")}
                className={cn(
                  "inline-flex h-7 items-center gap-1 rounded-md px-2 text-[10px] font-medium transition-colors",
                  view === "list" ? "bg-background text-foreground shadow-xs" : "text-muted-foreground hover:text-foreground",
                )}
                aria-pressed={view === "list"}
              >
                <List className="h-3 w-3" />
                {t("accounts.listView")}
              </button>
            </div>
          )}
        </div>
      </div>

      {items.length === 0 ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{emptyLabel}</p>
      ) : view === "chart" ? (
        <div className="mt-4 grid flex-1 gap-4 xl:grid-cols-[220px_minmax(0,1fr)] xl:items-center">
          <div className="flex items-center justify-center rounded-xl border border-border/55 bg-background/40 px-3 py-5">
            <div className="relative h-48 w-48 sm:h-52 sm:w-52">
              <svg
                viewBox="0 0 120 120"
                className="h-full w-full"
                role="img"
                aria-label={title + " · " + formatTokens(sum) + " Token"}
              >
                <circle cx="60" cy="60" r="44" fill="none" strokeWidth="11" className="stroke-muted/70" />
                {donutParts.map((part) => {
                  const segmentLength = sum > 0 ? (part.value / sum) * circumference : 0;
                  const offset = donutOffset;
                  donutOffset += segmentLength;
                  return (
                    <circle
                      key={part.key}
                      cx="60"
                      cy="60"
                      r="44"
                      fill="none"
                      strokeWidth="11"
                      strokeDasharray={segmentLength + " " + circumference}
                      strokeDashoffset={-offset}
                      transform="rotate(-90 60 60)"
                      className={part.color.stroke}
                    />
                  );
                })}
              </svg>
              <div className="absolute inset-0 flex flex-col items-center justify-center text-center">
                <span className="text-[10px] text-muted-foreground">{t("accounts.totalShort")}</span>
                <strong className="mt-1 font-mono text-xl tracking-tight">{formatTokens(sum)}</strong>
                <span className="mt-1 text-[9px] text-muted-foreground">Token</span>
              </div>
            </div>
          </div>

          <div className="grid min-w-0 gap-2 sm:grid-cols-2 xl:grid-cols-1">
            {visibleItems.map((item, index) => {
              const color = MODEL_COLORS[index];
              const overlapped = item.memberCount > 1;
              return (
                <div key={item.key} className="flex min-w-0 items-center justify-between gap-3 rounded-lg border border-border/45 bg-background/35 px-3 py-2.5">
                  <div className="flex min-w-0 items-center gap-2">
                    <i className={cn("h-2.5 w-2.5 shrink-0 rounded-full", color.dot)} />
                    <span className="truncate text-xs font-medium" title={item.title}>{item.title}</span>
                    {overlapped && <span className="shrink-0 font-mono text-[9px] text-primary">×{item.memberCount}</span>}
                  </div>
                  <div className="shrink-0 text-right">
                    <span className="font-mono text-[10px] text-muted-foreground">{formatTokens(item.totals.totalTokens)}</span>
                    <strong className={cn("ml-2 font-mono text-[10px]", color.text)}>{formatPercent(item.totals.totalTokens, sum)}</strong>
                  </div>
                </div>
              );
            })}
            {otherTokens > 0 && (
              <div className="flex items-center justify-between gap-3 rounded-lg border border-border/45 bg-background/35 px-3 py-2.5">
                <span className="flex items-center gap-2 text-xs text-muted-foreground"><i className="h-2.5 w-2.5 rounded-full bg-slate-400" />{t("accounts.otherModels")}</span>
                <strong className="font-mono text-[10px] text-slate-500">{formatPercent(otherTokens, sum)}</strong>
              </div>
            )}
          </div>
        </div>
      ) : (
        <div className="mt-4 min-w-0 space-y-2">
          {visibleItems.map((item, index) => {
            const color = MODEL_COLORS[index];
            const overlapped = item.memberCount > 1;
            const layerCount = Math.min(item.memberCount, 3);
            return (
              <div
                key={item.key}
                className={cn(
                  "group relative min-w-0 rounded-lg border px-3 py-2.5 transition-colors",
                  overlapped ? "border-primary/15 bg-primary/[0.025] shadow-[2px_2px_0_hsl(var(--primary)/0.05)]" : "border-border/35 bg-background/25",
                )}
              >
                <div className="flex items-center justify-between gap-2 text-xs">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <div className="relative h-5 w-8 shrink-0" aria-hidden="true">
                      {Array.from({ length: layerCount }).map((_, layerIndex) => (
                        <span
                          key={layerIndex}
                          className={cn(
                            "absolute top-1/2 flex h-4 w-4 -translate-y-1/2 items-center justify-center rounded-md border border-background bg-card shadow-xs",
                            layerIndex === 0 && "left-0",
                            layerIndex === 1 && "left-1.5",
                            layerIndex === 2 && "left-3",
                          )}
                          style={{ zIndex: layerIndex + 1 }}
                        >
                          <KeyRound className={cn("h-2.5 w-2.5", color.text)} />
                        </span>
                      ))}
                    </div>
                    <i className={cn("h-2 w-2 shrink-0 rounded-full", color.dot)} />
                    <p className="truncate font-medium text-foreground/90" title={item.title}>{item.title}</p>
                    {item.providerLabels.length > 0 && (
                      <span className="max-w-24 shrink-0 truncate rounded bg-muted/70 px-1.5 py-0.5 font-mono text-[9px] text-muted-foreground" title={item.providerLabels.join(" + ")}>{item.providerLabels[0]}</span>
                    )}
                    {overlapped && <span className="shrink-0 rounded-full border border-primary/15 bg-primary/8 px-1.5 py-0.5 font-mono text-[9px] font-medium text-primary">×{item.memberCount}</span>}
                  </div>
                  <div className="shrink-0 text-right">
                    <span className="font-mono text-[11px] font-medium text-foreground">{formatTokens(item.totals.totalTokens)}</span>
                    <span className={cn("ml-1.5 font-mono text-[10px] font-semibold", color.text)}>{formatPercent(item.totals.totalTokens, sum)}</span>
                  </div>
                </div>
                <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-muted/80 ring-1 ring-border/40">
                  <div className={cn("h-full rounded-full transition-all duration-300 group-hover:brightness-110", color.bar)} style={{ width: Math.max(3, (item.totals.totalTokens / max) * 100) + "%" }} />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </section>
  );
}
