import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { RelayUsagePayload } from "@/types";
import { cacheHitRate, formatCacheRate } from "@/lib/cache-usage";
import { cn } from "@/lib/utils";

const tokens = (value: number | undefined) => value == null ? "—"
  : new Intl.NumberFormat(undefined, { notation: value >= 10000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value);

export function CacheUsagePanel({ relay }: { relay: RelayUsagePayload }) {
  const { t } = useTranslation();
  const [group, setGroup] = useState<"providers" | "models">("providers");
  const rows = group === "providers" ? relay.byProvider : relay.byModel;
  return <div className="space-y-4" data-testid="cache-usage-panel">
    <div className="flex flex-wrap items-center gap-x-8 gap-y-3">
      {([['today', relay.today], ['lastSevenDays', relay.lastSevenDays], ['allTime', relay.allTime]] as const).map(([period, totals]) =>
        <div key={period} className="flex items-baseline gap-2" title={t("accounts.cacheRateFormula")}>
          <span className="text-xs text-muted-foreground">{t("accounts.cachePeriod." + period)}</span>
          <strong className="text-lg tabular-nums">{formatCacheRate(cacheHitRate(totals.cacheUsage))}</strong>
        </div>)}
    </div>
    <div className="flex flex-wrap items-center justify-between gap-2">
      <span className="text-xs text-muted-foreground">{t("accounts.cacheCumulative")}</span>
      <div className="usage-view-tabs usage-view-tabs-compact" role="tablist" aria-label={t("accounts.cacheGrouping")}>
        {(["providers", "models"] as const).map(key => <button key={key} type="button" role="tab" aria-selected={group === key}
          className={cn(group === key && "is-active")} onClick={() => setGroup(key)}>{t("accounts.cacheGroup." + key)}</button>)}
      </div>
    </div>
    <div className="min-w-0 overflow-x-auto">
      <table className="usage-request-table">
        <thead><tr><th>{t("accounts.providerColumn")}</th>{group === "models" && <th>{t("accounts.billingModelColumn")}</th>}
          <th title={t("accounts.cacheRateFormula")}>{t("accounts.cacheHitRate")}</th>
          <th>{t("accounts.cacheRead")}</th><th>{t("accounts.cacheWrite")}</th><th>{t("accounts.cachePrompt")}</th>
          <th>{t("accounts.cacheCoverage")}</th></tr></thead>
        <tbody>{rows.map(row => {
          const observed = row.totals.cacheUsage?.reportedCount ? row.totals.cacheUsage : undefined;
          return <tr key={row.providerId + ("model" in row ? `::${row.model}` : "")}>
          <td className="font-medium">{row.providerName}</td>{group === "models" && <td className="font-mono">{"model" in row ? String(row.model) : "—"}</td>}
          <td className="font-mono">{formatCacheRate(cacheHitRate(row.totals.cacheUsage))}</td>
          <td className="font-mono">{tokens(observed?.readTokens)}</td>
          <td className="font-mono">{tokens(observed?.writeTokens)}</td>
          <td className="font-mono">{tokens(observed?.promptTokens)}</td>
          <td className="font-mono" title={t("accounts.cacheCoverageHint")}>
            {row.totals.cacheUsage?.reportedCount ?? 0} / {row.totals.requestCount}
          </td>
        </tr>; })}</tbody>
      </table>
      {rows.length === 0 && <p className="usage-empty-state">{t("accounts.relayEmpty")}</p>}
    </div>
  </div>;
}
