import { useState, type CSSProperties } from "react";
import { ChartPie, List } from "lucide-react";
import { useTranslation } from "react-i18next";

import { costLabel } from "@/lib/format-cost";
import { cn } from "@/lib/utils";

type UsageTotals = { totalTokens: number; costMicros?: number; pricedCount?: number; unpricedCount?: number; partialCount?: number; coverageUnknownCount?: number };
type UsageItem = { key: string; title: string; subtitle?: string; totals: UsageTotals };
type DisplayItem = UsageItem & { memberCount: number; providerLabels: string[] };
const COLORS = ["#ed7c35", "#54a7d9", "#54b98b", "#a67be7", "#e66880", "#e6ad4f"];
const OTHER_COLOR = "#94a3b8";
const CIRCUMFERENCE = Math.PI * 2 * 43;

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, { notation: value >= 10_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value);
}

function cleanModelTitle(title: string) {
  if (!title.startsWith("aimami_relay_")) return title;
  const relayModel = title.split("::").pop();
  return relayModel && relayModel !== title ? relayModel : title;
}

function emptyTotals(totals: UsageTotals): UsageTotals {
  return { totalTokens: totals.totalTokens, costMicros: totals.costMicros ?? 0, pricedCount: totals.pricedCount ?? 0, unpricedCount: totals.unpricedCount ?? 0,
    partialCount: totals.partialCount ?? 0, coverageUnknownCount: totals.coverageUnknownCount ?? 0 };
}

function addTotals(target: UsageTotals, other: UsageTotals) {
  target.costMicros = (target.costMicros ?? 0) + (other.costMicros ?? 0);
  target.pricedCount = (target.pricedCount ?? 0) + (other.pricedCount ?? 0);
  target.unpricedCount = (target.unpricedCount ?? 0) + (other.unpricedCount ?? 0);
  target.partialCount = (target.partialCount ?? 0) + (other.partialCount ?? 0);
  target.coverageUnknownCount = (target.coverageUnknownCount ?? 0) + (other.coverageUnknownCount ?? 0);
}

function prepareItems(items: UsageItem[], mergeDuplicateModels: boolean): DisplayItem[] {
  if (!mergeDuplicateModels) return items.map(item => ({ ...item, memberCount: 1, providerLabels: item.subtitle ? [item.subtitle] : [] }));
  const groups = new Map<string, DisplayItem>();
  for (const item of items) {
    const title = cleanModelTitle(item.title);
    const key = title.trim().toLowerCase();
    const current = groups.get(key);
    if (!current) {
      groups.set(key, { ...item, title, totals: emptyTotals(item.totals), memberCount: 1, providerLabels: item.subtitle ? [item.subtitle] : [] });
      continue;
    }
    current.totals.totalTokens += item.totals.totalTokens;
    addTotals(current.totals, item.totals);
    current.memberCount += 1;
    if (item.subtitle && !current.providerLabels.includes(item.subtitle)) current.providerLabels.push(item.subtitle);
  }
  return [...groups.values()];
}

export function UsageBreakdown({ title, items, emptyLabel, mergeDuplicateModels = false, embedded = false }: {
  title: string;
  items: UsageItem[];
  emptyLabel: string;
  mergeDuplicateModels?: boolean;
  embedded?: boolean;
}) {
  const { t } = useTranslation();
  const [view, setView] = useState<"chart" | "list">("chart");
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const sorted = prepareItems(items, mergeDuplicateModels).sort((a, b) => b.totals.totalTokens - a.totals.totalTokens);
  const total = sorted.reduce((sum, item) => sum + item.totals.totalTokens, 0);
  const sumCosts = (items: DisplayItem[]) => items.reduce((sum, item) => { addTotals(sum, item.totals); return sum; }, emptyTotals({ totalTokens: 0 }));
  const totals = sumCosts(sorted);
  const label = (totals: UsageTotals) => costLabel(totals.costMicros ?? 0, totals.pricedCount ?? 0, totals.unpricedCount ?? 0,
    count => t("accounts.unpricedRequests", { count }), t("accounts.costUnavailable"), {
      partialCount: totals.partialCount ?? 0, coverageUnknownCount: totals.coverageUnknownCount ?? 0,
      partialLabel: count => t("accounts.partialRequests", { count }),
      coverageUnknownLabel: count => t("accounts.coverageUnknownRequests", { count }),
    });
  const visible = sorted.slice(0, COLORS.length);
  const other = sorted.slice(COLORS.length).reduce((sum, item) => sum + item.totals.totalTokens, 0);
  const otherTotals = sumCosts(sorted.slice(COLORS.length));
  const segments = visible.map((item, index) => ({ key: item.key, label: item.title, value: item.totals.totalTokens, totals: item.totals, color: COLORS[index] }));
  if (other > 0) segments.push({ key: "other", label: t("accounts.otherModels"), value: other, totals: otherTotals, color: OTHER_COLOR });
  const active = segments.find(item => item.key === activeKey) ?? null;
  let offset = 0;
  const showCost = (totals.pricedCount ?? 0) > 0 || (totals.unpricedCount ?? 0) > 0;

  return <section className={cn("usage-breakdown-panel", embedded && "is-embedded")}>
    <div className="usage-panel-heading">
      <div>
        <h4>{title}</h4>
        <p>
          {formatTokens(total)} Token
          {showCost && (
            <span>
              {" · "}{label(totals)}
            </span>
          )}
        </p>
      </div>
      {sorted.length > 0 && <div className="usage-view-tabs usage-view-tabs-compact" role="group" aria-label={title}>
        <button type="button" aria-pressed={view === "chart"} className={cn(view === "chart" && "is-active")} onClick={() => setView("chart")}><ChartPie className="h-4 w-4" />{t("accounts.chartView")}</button>
        <button type="button" aria-pressed={view === "list"} className={cn(view === "list" && "is-active")} onClick={() => setView("list")}><List className="h-4 w-4" />{t("accounts.listView")}</button>
      </div>}
    </div>

    {sorted.length === 0 ? <p className="usage-empty-state">{emptyLabel}</p> : view === "chart" ?
      <div className="usage-breakdown-content">
        <div className="usage-donut-wrap" onMouseLeave={() => setActiveKey(null)}>
          <svg viewBox="0 0 120 120" role="img" aria-label={`${title} · ${formatTokens(total)} Token`}>
            <circle cx="60" cy="60" r="43" fill="none" stroke="hsl(var(--border) / 0.6)" strokeWidth="12" />
            {segments.map(segment => {
              const length = total > 0 ? segment.value / total * CIRCUMFERENCE : 0;
              const start = offset;
              offset += length;
              return <circle key={segment.key} cx="60" cy="60" r="43" fill="none" stroke={segment.color}
                strokeWidth={active?.key === segment.key ? 16 : 12}
                strokeDasharray={`${length} ${CIRCUMFERENCE}`}
                strokeDashoffset={-start} transform="rotate(-90 60 60)"
                className={cn("usage-donut-segment", active && active.key !== segment.key && "is-muted", active?.key === segment.key && "is-active")}
                tabIndex={0} aria-label={`${segment.label} ${formatTokens(segment.value)} Token`}
                onMouseEnter={() => setActiveKey(segment.key)} onFocus={() => setActiveKey(segment.key)} onBlur={() => setActiveKey(null)} />;
            })}
          </svg>
          <div className="usage-donut-center" aria-live="polite"><span>{active?.label ?? t("accounts.totalShort")}</span><strong>{formatTokens(active?.value ?? total)}</strong><small>{active ? `${Math.round(active.value / Math.max(1, total) * 100)}%` : "Token"}</small></div>
        </div>
        <div className="usage-model-list">
          {segments.map(segment => <div key={segment.key} className={cn("usage-model-row", active?.key === segment.key && "is-active")}
            style={{ "--model-color": segment.color } as CSSProperties}
            onMouseEnter={() => setActiveKey(segment.key)} onMouseLeave={() => setActiveKey(null)}
            onFocus={() => setActiveKey(segment.key)} onBlur={() => setActiveKey(null)} tabIndex={0}>
            <i /><span title={segment.label}>{segment.label}</span>
            <strong>{formatTokens(segment.value)}</strong>
            <span className="usage-model-cost" title={showCost ? label(segment.totals) : undefined}>{showCost ? label(segment.totals) : ""}</span>
            <em>{Math.round(segment.value / Math.max(1, total) * 100)}%</em>
          </div>)}
        </div>
      </div> :
      <div className="usage-model-detail-list">
        {sorted.map((item, index) => <div key={item.key} className="usage-model-detail-row" style={{ "--model-color": COLORS[index % COLORS.length] } as CSSProperties}>
          <i /><div className="min-w-0"><strong title={item.title}>{item.title}</strong><small title={item.providerLabels.join(", ")}>{item.providerLabels.join(" · ")}</small></div>
          <span>{formatTokens(item.totals.totalTokens)} Token</span>
          <span className="usage-model-cost" title={showCost ? label(item.totals) : undefined}>{showCost ? label(item.totals) : ""}</span>
          <em>{Math.round(item.totals.totalTokens / Math.max(1, total) * 100)}%</em>
        </div>)}
      </div>}
  </section>;
}
