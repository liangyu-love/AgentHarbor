import { useContext, useState } from "react";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ChartColumn, RefreshCw } from "lucide-react";

import { PageHero } from "@/components/layout/page-hero";
import { PageActiveContext } from "@/components/layout/page-stage";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { toast } from "@/hooks/use-toast";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { cn } from "@/lib/utils";
import type { UsagePayload, UsageRange } from "@/types";
import { UsageChart } from "./usage-chart";
import type { UsageMetric } from "./usage-format";
import { UsageKpis } from "./usage-kpis";
import { UsageRankings } from "./usage-rankings";
import { UsageRequests } from "./usage-requests";
import { UsageSegments } from "./usage-segments";

const RANGES = ["today", "week", "month"] as const;
const METRICS = ["cost", "tokens"] as const;

/** A remembered choice; storage may be unavailable, the default always works. */
function useChoice<T extends string>(key: string, fallback: T, allowed: readonly T[]) {
  const [value, setValue] = useState<T>(() => {
    try {
      const stored = localStorage.getItem(key);
      return allowed.includes(stored as T) ? stored as T : fallback;
    } catch {
      return fallback;
    }
  });
  const choose = (next: T) => {
    setValue(next);
    try { localStorage.setItem(key, next); } catch { /* storage unavailable */ }
  };
  return [value, choose] as const;
}

function Sources({ data }: { data: UsagePayload }) {
  const { t } = useTranslation();
  const { sources } = data;
  const parts = [
    t("usage.updated", { time: new Date(data.generatedAt * 1000).toLocaleString() }),
    ...sources.transcripts.map(source => {
      const name = source.sourceId === "codex" ? "Codex" : "Claude Code";
      return source.available ? t(`usage.sources.${source.sourceId}`, { count: source.fileCount }) : t("usage.sources.missing", { name });
    }),
    sources.relaySince != null
      ? t("usage.sources.relay", { count: sources.relayCount, date: new Date(sources.relaySince * 1000).toLocaleDateString() })
      : t("usage.sources.relayNone"),
    sources.matchedCount > 0 ? t("usage.sources.matched", { count: sources.matchedCount }) : null,
  ].filter(Boolean);
  return <p className="text-[11.5px] leading-relaxed text-muted-foreground" data-usage-sources>{parts.join(" · ")}</p>;
}

/** Every client's tokens and cost in one ledger, polled only while the page is active. */
export function UsagePage({ refreshInterval, onManageProviders, onOpenThreads }: {
  refreshInterval: RefreshInterval;
  onManageProviders: () => void;
  onOpenThreads?: (client: string) => void;
}) {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const [range, setRange] = useChoice<UsageRange>(storageKeys.usageRange, "week", RANGES);
  const [metric, setMetric] = useChoice<UsageMetric>(storageKeys.usageMetric, "cost", METRICS);
  const usage = useQuery({
    queryKey: [...queryKeys.tokenUsage, range],
    queryFn: () => api.usage.overview(range),
    enabled: pageActive,
    // A new range keeps the last one on screen until it arrives.
    placeholderData: keepPreviousData,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
  const refresh = async () => {
    const result = await usage.refetch();
    if (result.data) toast({ title: t("usage.refreshSuccess"), variant: "success" });
  };
  const data = usage.data;

  return (
    <div className="mx-auto max-w-7xl space-y-5 pb-8" data-usage-page>
      <PageHero kicker="Usage" icon={<ChartColumn />} titleAs="h2" title={t("usage.title")}
        description={t("usage.heroDesc")}
        actions={<>
          <UsageSegments label={t("usage.rangeLabel")} value={range} onChange={setRange}
            items={RANGES.map(value => ({ value, label: t(`usage.range.${value}`) }))} />
          <UsageSegments label={t("usage.metricLabel")} value={metric} onChange={setMetric}
            items={METRICS.map(value => ({ value, label: t(`usage.metric.${value}`) }))} />
          <Button variant="outline" onClick={() => void refresh()} disabled={usage.isFetching}>
            <RefreshCw className={cn(usage.isFetching && "animate-spin")} />
            {t("common.refresh")}
          </Button>
        </>}>
        {data && <Sources data={data} />}
      </PageHero>

      {usage.isPending ? <div className="space-y-5" aria-busy>
        <Skeleton className="h-[148px] rounded-3xl" />
        <Skeleton className="h-[360px] rounded-3xl" />
      </div> : usage.error && !data ? (
        <section className="rounded-3xl border border-destructive/30 bg-destructive/5 p-5">
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div>
              <h3 className="font-semibold text-destructive">{t("accounts.tokenUsageLoadFailed")}</h3>
              <p className="mt-1 text-xs text-muted-foreground">{t("accounts.tokenUsageLoadFailedDesc")}</p>
            </div>
            <Button variant="outline" size="xs" onClick={() => void usage.refetch()}>
              <RefreshCw />
              {t("accounts.retry")}
            </Button>
          </div>
          <details className="mt-3 text-[11px] text-muted-foreground">
            <summary className="cursor-pointer">{t("accounts.technicalDetails")}</summary>
            <p className="mt-2 break-all font-mono">{errorMessage(usage.error)}</p>
          </details>
        </section>
      ) : data && data.allTime.requestCount === 0 ? (
        <section className="harbor-card route-rise py-12 text-center">
          <p className="poster-kicker text-[10px] text-primary">Empty</p>
          <h3 className="mt-1 font-display text-lg font-semibold">{t("usage.empty.title")}</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-muted-foreground">{t("usage.empty.desc")}</p>
        </section>
      ) : data && <div className={cn("space-y-5 transition-opacity", usage.isPlaceholderData && "opacity-60")}>
        <UsageKpis data={data} />
        <UsageChart data={data} metric={metric} />
        <UsageRankings data={data} metric={metric} onManageProviders={onManageProviders}
          onOpenThreads={client => onOpenThreads?.(client)} />
        <UsageRequests key={data.range} entries={data.recent} />
        {data.warningCount > 0 && (
          <details className="rounded-2xl border border-amber-500/25 bg-amber-500/5 px-4 py-3 text-xs text-amber-800 dark:text-amber-300">
            <summary className="cursor-pointer font-medium">{t("accounts.scanWarnings", { count: data.warningCount })}</summary>
            <ul className="mt-2 space-y-1 pl-4">
              {data.warnings.map(warning => <li key={warning} className="list-disc">{warning}</li>)}
            </ul>
          </details>
        )}
      </div>}
    </div>
  );
}
