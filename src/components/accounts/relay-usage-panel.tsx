import { useMemo, useState, type MouseEvent } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ArrowRight, ChartNoAxesCombined, ListFilter, RefreshCw, Rows3, Wallet } from "lucide-react";

import { UsageBreakdown } from "@/components/accounts/model-usage-breakdown";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import type { RelayUsagePayload, RelayUsageTotals } from "@/types";
import type { NewApiState } from "@/types/newapi";
import { formatCost, costLabel } from "@/lib/format-cost";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";

type View = "trend" | "models" | "requests" | "providers";
const CHART_WIDTH = 1000;
const CHART_BASE = 226;
const CHART_LEFT = 62;
const CHART_RIGHT = 22;

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, { notation: value >= 10_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value);
}

function totalsCost(totals: RelayUsageTotals, t: TFunction, unknown = t("accounts.costUnavailable")) {
  return costLabel(totals.costMicros, totals.pricedCount, totals.unpricedCount,
    count => t("accounts.unpricedRequests", { count }), unknown, {
      partialCount: totals.partialCount ?? 0,
      coverageUnknownCount: totals.coverageUnknownCount ?? 0,
      partialLabel: count => t("accounts.partialRequests", { count }),
      coverageUnknownLabel: count => t("accounts.coverageUnknownRequests", { count }),
    });
}

function relayPath(points: Array<{ x: number; y: number }>) {
  if (!points.length) return "";
  let path = `M ${points[0].x} ${points[0].y}`;
  for (let i = 1; i < points.length; i += 1) {
    const middle = (points[i - 1].x + points[i].x) / 2;
    path += ` C ${middle} ${points[i - 1].y}, ${middle} ${points[i].y}, ${points[i].x} ${points[i].y}`;
  }
  return path;
}

function RelayTrend({ relay }: { relay: RelayUsagePayload }) {
  const { t } = useTranslation();
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const daily = relay.daily;
  const selectedIndex = activeIndex ?? daily.length - 1;
  const chart = useMemo(() => {
    const maximum = Math.max(1, ...daily.map(day => day.totals.totalTokens)) * 1.12;
    const points = daily.map((day, index) => ({
      x: CHART_LEFT + index * (CHART_WIDTH - CHART_LEFT - CHART_RIGHT) / Math.max(1, daily.length - 1),
      y: CHART_BASE - day.totals.totalTokens / maximum * 190,
    }));
    const path = relayPath(points);
    const area = points.length ? `${path} L ${points[points.length - 1].x} ${CHART_BASE} L ${points[0].x} ${CHART_BASE} Z` : "";
    return { maximum, points, path, area };
  }, [daily]);
  if (!daily.length) return <p className="usage-empty-state">{t("accounts.relayEmpty")}</p>;
  const day = daily[selectedIndex];
  const active = chart.points[selectedIndex];
  const onMove = (event: MouseEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const x = (event.clientX - rect.left) / rect.width * CHART_WIDTH;
    setActiveIndex(Math.min(daily.length - 1, Math.max(0, Math.round((x - CHART_LEFT) / (CHART_WIDTH - CHART_LEFT - CHART_RIGHT) * (daily.length - 1)))));
  };
  return <div className="usage-relay-trend">
    <div className="usage-trend-selection"><span>{day.date}</span><strong>{formatTokens(day.totals.totalTokens)} Token</strong></div>
    <svg viewBox="0 0 1000 265" role="img" className="usage-trend-svg" aria-label={t("accounts.relayTrend")}
      onMouseMove={onMove} onMouseLeave={() => setActiveIndex(null)}>
      <defs><linearGradient id="usage-relay-area" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="#ed7c35" stopOpacity="0.22" /><stop offset="100%" stopColor="#ed7c35" stopOpacity="0" /></linearGradient></defs>
      {[0, .25, .5, .75, 1].map(ratio => {
        const y = CHART_BASE - ratio * 190;
        return <g key={ratio}><line x1={CHART_LEFT} x2={CHART_WIDTH - CHART_RIGHT} y1={y} y2={y} className="usage-chart-gridline" />
          <text x={CHART_LEFT - 10} y={y + 4} textAnchor="end" className="usage-chart-axis">{formatTokens(chart.maximum * ratio)}</text></g>;
      })}
      <path d={chart.area} fill="url(#usage-relay-area)" className="usage-area-enter" />
      <path d={chart.path} fill="none" stroke="#ed7c35" strokeWidth="3" strokeLinecap="round" />
      <line x1={active.x} x2={active.x} y1="36" y2={CHART_BASE} className="usage-chart-crosshair" />
      <circle cx={active.x} cy={active.y} r="6.5" fill="#ed7c35" stroke="hsl(var(--card))" strokeWidth="3" />
      {daily.map((item, index) => <g key={item.date}>
        {(index % 2 === 0 || index === daily.length - 1) && <text x={chart.points[index].x} y="255" textAnchor="middle" className="usage-chart-axis">{index === daily.length - 1 ? t("accounts.todayShort") : item.date.slice(5)}</text>}
        <circle cx={chart.points[index].x} cy={chart.points[index].y} r="12" fill="transparent" tabIndex={0}
          aria-label={`${item.date} ${formatTokens(item.totals.totalTokens)} Token`}
          onFocus={() => setActiveIndex(index)} onBlur={() => setActiveIndex(null)} onClick={() => setActiveIndex(index)} />
      </g>)}
    </svg>
    <div className="usage-relay-day-details" aria-live="polite">
      <span><i className="usage-dot usage-dot-sky" />{t("accounts.inputShort")} <strong>{formatTokens(day.totals.inputTokens)}</strong></span>
      <span><i className="usage-dot usage-dot-orange" />{t("accounts.outputShort")} <strong>{formatTokens(day.totals.outputTokens)}</strong></span>
      {day.totals.cacheReadTokens + day.totals.cacheWriteTokens > 0 && (
        <span title={t("accounts.cacheDetail", {
          read: formatTokens(day.totals.cacheReadTokens),
          write: formatTokens(day.totals.cacheWriteTokens),
        })}>
          {t("accounts.cacheShort")} <strong>{formatTokens(day.totals.cacheReadTokens + day.totals.cacheWriteTokens)}</strong>
        </span>
      )}
      {day.totals.pricedCount > 0 && (
        <span>{t("accounts.costEstimate")} <strong>{totalsCost(day.totals, t)}</strong></span>
      )}
    </div>
  </div>;
}

function RelayRequests({ relay }: { relay: RelayUsagePayload }) {
  const { t } = useTranslation();
  const [source, setSource] = useState("all");
  const records = relay.recent.filter(record => source === "all" ||
    (source === "claudeDesktop" ? record.clientSource === "claudeDesktop" : record.clientSource !== "claudeDesktop"));
  if (!relay.recent.length) return <p className="usage-empty-state">{t("accounts.relayEmpty")}</p>;
  return <div className="usage-request-scroll">
    <label className="flex items-center gap-2 px-2 pb-2 text-xs">
      {t("newapi.routeSourceLabel")}
      <select aria-label={t("newapi.routeSourceLabel")} className="rounded-md border bg-background px-2 py-1" value={source} onChange={event => setSource(event.target.value)}>
        <option value="all">{t("newapi.routeSourceAll")}</option>
        <option value="claudeDesktop">Claude Desktop</option>
        <option value="other">{t("newapi.routeSourceOther")}</option>
      </select>
    </label>
    <p className="px-2 pb-2 text-[11px] text-muted-foreground">{t("accounts.relayDetailRetention")}</p>
    <table className="usage-request-table">
      <thead><tr>
        <th>{t("accounts.providerColumn")}</th><th>{t("accounts.billingModelColumn")}</th>
        <th>{t("accounts.inputShort")}</th><th>{t("accounts.outputShort")}</th><th>{t("accounts.totalShort")}</th>
        <th>{t("accounts.costColumn")}</th><th>{t("accounts.timingColumn")}</th>
        <th>{t("accounts.statusColumn")}</th><th>{t("accounts.sourceColumn")}</th><th>{t("accounts.timeColumn")}</th>
      </tr></thead>
      <tbody>{records.map((record, index) => <tr key={`${record.timestamp}-${record.providerId}-${index}`}>
        <td className="font-medium" title={record.providerName}>{record.providerName}</td>
        <td className="font-mono" title={record.model}>
          {record.model || "—"}
          {record.clientSource === "claudeDesktop" && <small className="block text-[10px] text-muted-foreground">
            {record.requestedModel || "—"} → {record.resolvedModel || record.model}
          </small>}
          <small className="block text-[10px] text-muted-foreground">
            {t("accounts.relayServedModel")}: {record.modelObservation?.servedModel ?? "—"}
            {" · "}{t("accounts.relayModelStatus." + (record.modelObservation?.modelMatchStatus || "unknown"))}
          </small>
          <small className="block text-[10px] text-muted-foreground">
            {t("accounts.relayCredential")}: {t("accounts.relayCredentialStatus." + (record.credential?.status || "not_recorded"))}
            {record.credential?.versionFingerprint && ` · ${record.credential.versionFingerprint.slice(0, 12)}`}
          </small>
        </td>
        <td className="font-mono">
          {record.usageReported ? formatTokens(record.inputTokens) : "—"}
          {record.cacheReadTokens + record.cacheWriteTokens > 0 && (
            <small className="ml-1 text-muted-foreground" title={t("accounts.cacheDetail", {
              read: formatTokens(record.cacheReadTokens),
              write: formatTokens(record.cacheWriteTokens),
            })}>
              +{formatTokens(record.cacheReadTokens + record.cacheWriteTokens)}
            </small>
          )}
        </td>
        <td className="font-mono">{record.usageReported ? formatTokens(record.outputTokens) : "—"}</td>
        <td className="font-mono font-semibold">{record.usageReported ? formatTokens(record.totalTokens) : "—"}</td>
        <td
          className="font-mono"
          title={!record.usageReported ? t("accounts.usageNotReported") : record.pricingStatus === "partial"
            ? t("accounts.costPartial")
            : record.pricingSource || t("accounts.costUnavailable")}
        >
          {!record.usageReported ? t("accounts.usageNotReported") : record.costUsd != null ? formatCost(record.costUsd * 1_000_000) : t("accounts.costUnpriced")}
          {record.pricingStatus === "partial" && <small className="ml-1 text-amber-600">*</small>}
        </td>
        <td className="font-mono"><strong>{record.durationMs} ms</strong><small>{t("accounts.firstTokenShort")} {record.firstTokenMs == null ? "—" : `${record.firstTokenMs} ms`}</small></td>
        <td><span className={cn("usage-request-status", record.success ? "is-good" : "is-error")}>{record.statusCode}</span></td>
        <td><span className="usage-request-source">{record.clientSource === "claudeDesktop" ? "Claude Desktop" : t("newapi.routeSourceUnknown")}</span></td>
        <td className="whitespace-nowrap">{new Date(record.timestamp * 1000).toLocaleString()}</td>
      </tr>)}</tbody>
    </table>
    {records.length === 0 && <p className="usage-empty-state">{t("accounts.relayEmpty")}</p>}
  </div>;
}

function ProviderQuota({ relay, onManageProviders }: { relay: RelayUsagePayload; onManageProviders?: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  const providers = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState, staleTime: 30_000 });
  const byId = new Map(providers.data?.providers.map(provider => [provider.id, provider]));
  const refreshBalance = async (providerId: string) => {
    if (refreshingId) return;
    setRefreshingId(providerId);
    try {
      const next = await api.providers.queryBalance(providerId);
      queryClient.setQueryData<NewApiState>(queryKeys.providers, current => current ? {
        ...current, providers: current.providers.map(provider => provider.id === providerId
          ? { ...provider, balance: next.balance, balanceSource: next.balanceSource } : provider),
      } : current);
    } catch (error) {
      toast({ title: t("newapi.balanceFailed"), description: formatInvokeError(error), variant: "destructive" });
    } finally {
      setRefreshingId(null);
    }
  };
  if (!relay.byProvider.length) return <p className="usage-empty-state">{t("accounts.relayEmpty")}</p>;
  return <div className="usage-provider-list">
    {relay.byProvider.map(item => {
      const provider = byId.get(item.providerId);
      const balance = provider?.balance;
      return <div key={item.providerId} className="usage-provider-row">
        <div className="min-w-0"><strong className="truncate" title={item.providerName}>{item.providerName}</strong><span>{t("accounts.relayRequests", { count: item.totals.requestCount })}</span></div>
        <div><span>{t("accounts.totalShort")}</span><strong>{formatTokens(item.totals.totalTokens)}</strong><em className="usage-cost-line">{totalsCost(item.totals, t)}</em></div>
        <div title={balance ? new Date(balance.checkedAt * 1000).toLocaleString() : undefined}>
          <span>{t("accounts.balanceRemaining")}</span>
          <strong>{balance ? balance.conversionAvailable
            ? `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(balance.remaining)} ${balance.unit}`
            : `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(balance.rawQuota)} ${t("newapi.balanceQuota")}` : "—"}</strong>
        </div>
        <button type="button" className="usage-balance-refresh" disabled={!provider || !(provider.hasApiKey || provider.hasAccessToken) || refreshingId !== null}
          onClick={() => void refreshBalance(item.providerId)} aria-label={`${t("accounts.refreshBalance")} ${item.providerName}`} title={t("accounts.refreshBalance")}>
          <RefreshCw className={cn("h-4 w-4", refreshingId === item.providerId && "animate-spin")} />
        </button>
      </div>;
    })}
    {onManageProviders && <button className="usage-provider-manage" type="button" onClick={onManageProviders}>{t("accounts.manageProviderBalance")} <ArrowRight className="h-4 w-4" /></button>}
  </div>;
}

export function RelayUsagePanel({ relay, onManageProviders }: { relay: RelayUsagePayload; onManageProviders?: () => void }) {
  const { t } = useTranslation();
  const [view, setView] = useState<View>("trend");
  const relayModels = relay.byModel.map(item => ({ key: item.providerId + "::" + item.model, title: item.model, subtitle: item.providerName, totals: item.totals }));
  const tabs: Array<{ key: View; label: string; icon: typeof ChartNoAxesCombined }> = [
    { key: "trend", label: t("accounts.trendView"), icon: ChartNoAxesCombined },
    { key: "models", label: t("accounts.modelView"), icon: ListFilter },
    { key: "requests", label: t("accounts.requestsView"), icon: Rows3 },
    { key: "providers", label: t("accounts.providersView"), icon: Wallet },
  ];
  return <section className="usage-relay-panel">
    <div className="usage-panel-heading usage-relay-header">
      <div><h4>{t("accounts.relayUsage")}</h4><p>{t("accounts.relayRequests", { count: relay.allTime.requestCount })} · {t("accounts.relayReported", { count: relay.allTime.reportedCount })}</p></div>
      <div className="usage-relay-summary" title={t("accounts.costEstimatedHint")}>
        <span>{t("accounts.costToday")}</span>
        <strong>{totalsCost(relay.today, t, "—")}<small>USD</small></strong>
      </div>
      <div className="usage-relay-summary" title={t("accounts.costEstimatedHint")}>
        <span>{t("accounts.costWeek")}</span>
        <strong>{totalsCost(relay.lastSevenDays, t, "—")}<small>USD</small></strong>
      </div>
      <div className="usage-relay-summary" title={t("accounts.costEstimatedHint")}>
        <span>{t("accounts.costAllTime")}</span>
        <strong>{totalsCost(relay.allTime, t, "—")}<small>USD</small></strong>
      </div>
      <div className="usage-relay-summary"><span>{t("accounts.allUsage")}</span><strong>{formatTokens(relay.allTime.totalTokens)} <small>Token</small></strong></div>
    </div>
    <div className="usage-view-tabs" role="tablist" aria-label={t("accounts.relayUsage")}>
      {tabs.map(tab => <button key={tab.key} type="button" role="tab" aria-selected={view === tab.key} onClick={() => setView(tab.key)} className={cn(view === tab.key && "is-active")}>
        <tab.icon className="h-4 w-4" />{tab.label}
      </button>)}
    </div>
    <div className="usage-relay-body" role="tabpanel">
      {view === "trend" && <RelayTrend relay={relay} />}
      {view === "models" && <UsageBreakdown title={t("accounts.relayModels")} items={relayModels} emptyLabel={t("accounts.relayEmpty")} mergeDuplicateModels embedded />}
      {view === "requests" && <RelayRequests relay={relay} />}
      {view === "providers" && <ProviderQuota relay={relay} onManageProviders={onManageProviders} />}
    </div>
  </section>;
}
