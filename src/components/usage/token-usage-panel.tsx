import { useTranslation } from "react-i18next";
import { Activity, Clock3, RefreshCw } from "lucide-react";

import { DailyTrendPanel } from "@/components/usage/daily-trend-panel";
import { UsageBreakdown } from "@/components/usage/model-usage-breakdown";
import { RelayUsagePanel } from "@/components/usage/relay-usage-panel";
import { RecentTurnsPanel } from "@/components/usage/recent-turns-panel";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import type { TokenUsagePayload } from "@/types";
import { errorMessage } from "@/lib/invoke-error";

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

function formatPercent(value: number, total: number) {
  if (total <= 0) return "0%";
  return `${Math.round((value / total) * 100)}%`;
}

function UsageMetric({ label, value, tone }: { label: string; value: number; tone: "sky" | "orange" | "green" }) {
  return <div className={cn("usage-metric", `usage-metric-${tone}`)}>
    <span>{label}</span><strong>{formatTokens(value)}</strong>
  </div>;
}

function TodayUsageCard({ label, total, count, input, output, cache, accent }: {
  label: string; total: number; count: string; input: number; output: number; cache: number; accent: "local" | "relay";
}) {
  const { t } = useTranslation();
  return <div className={cn("usage-today-card", `usage-today-${accent}`)}>
    <div className="usage-today-head"><span>{label}</span><small>{count}</small></div>
    <div className="usage-today-total">{formatTokens(total)} <small>Token</small></div>
    <div className="usage-metric-row">
      <UsageMetric label={t("accounts.inputShort")} value={input} tone="sky" />
      <UsageMetric label={t("accounts.outputShort")} value={output} tone="orange" />
      <UsageMetric label={t("accounts.cacheShort")} value={cache} tone="green" />
    </div>
  </div>;
}

function SourceUsageCard({
  name,
  badge,
  totalTokens,
  count,
  countLabel,
  inputTokens,
  outputTokens,
  cacheTokens,
  share,
  className,
}: {
  name: string;
  badge: string;
  totalTokens: number;
  count: number;
  countLabel?: string;
  inputTokens: number;
  outputTokens: number;
  cacheTokens: number;
  share?: number;
  className?: string;
}) {
  const { t } = useTranslation();
  return (
    <div className={cn("usage-source-card", className)}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="truncate font-semibold">{name}</p>
          <p className="mt-1 text-[11px] text-muted-foreground">
            {countLabel ?? t("accounts.usageEvents", { count })}
          </p>
        </div>
        <div className="shrink-0 text-right">
          <p className="text-xl font-semibold tracking-tight tabular-nums">{formatTokens(totalTokens)}</p>
          {typeof share === "number" && (
            <p className="mt-0.5 font-mono text-[10px] text-muted-foreground">
              占比 {formatPercent(totalTokens, share)}
            </p>
          )}
        </div>
      </div>
      <div className="usage-metric-row mt-3">
        <UsageMetric label={t("accounts.inputShort")} value={inputTokens} tone="sky" />
        <UsageMetric label={t("accounts.outputShort")} value={outputTokens} tone="orange" />
        <UsageMetric label={t("accounts.cacheShort")} value={cacheTokens} tone="green" />
      </div>
      <div className="mt-2 truncate text-[10px] text-muted-foreground/80" title={badge}>
        {badge}
      </div>
    </div>
  );
}

export function TokenUsagePanel({ data, loading, error, onRetry, onManageProviders }: {
  data: TokenUsagePayload | undefined;
  loading: boolean;
  error: unknown;
  onRetry: () => void;
  onManageProviders: () => void;
}) {
  const { t } = useTranslation();
  if (loading) return <Skeleton className="h-[520px] rounded-2xl" />;
  if (error) return (
    <section className="rounded-2xl border border-destructive/30 bg-destructive/5 p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-semibold text-destructive">{t("accounts.tokenUsageLoadFailed")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">{t("accounts.tokenUsageLoadFailedDesc")}</p>
        </div>
        <Button variant="outline" size="xs" onClick={onRetry}>
          <RefreshCw />
          {t("accounts.retry")}
        </Button>
      </div>
      <details className="mt-3 text-[11px] text-muted-foreground">
        <summary className="cursor-pointer">{t("accounts.technicalDetails")}</summary>
        <p className="mt-2 break-all font-mono">{errorMessage(error)}</p>
      </details>
    </section>
  );
  if (!data) return null;

  const local = data.local;
  const relay = data.relay;
  const localModels = local.byModel.map((item) => ({
    key: item.sourceId + "::" + item.model,
    title: item.model,
    subtitle: item.sourceName,
    totals: item.totals,
  }));
  const localShareBase = Math.max(1, local.allTime.totalTokens);

  return (
    <section className="overflow-hidden rounded-2xl border border-border/75 bg-card/75 shadow-sm">
      <div className="flex flex-col gap-3 border-b border-border/70 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-3">
          <div className="rounded-xl bg-primary/10 p-2.5 text-primary">
            <Activity className="h-5 w-5" />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="font-semibold tracking-tight">{t("accounts.tokenMonitor")}</h3>
            <Badge variant="outline" className="border-primary/20 bg-primary/5 text-[10px] text-primary">
              Codex + Claude Code + Relay
            </Badge>
          </div>
        </div>
        <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <Clock3 className="h-3.5 w-3.5" />
          {t("accounts.lastUpdated", { time: new Date(data.generatedAt * 1000).toLocaleString() })}
        </span>
      </div>

      <div className="space-y-4 p-4 sm:p-5">
        <div className="usage-today-row" aria-label={t("accounts.todayUsage")}>
          <TodayUsageCard label={`${t("accounts.todayUsage")} · ${t("accounts.localAgentUsage")}`} total={local.today.totalTokens}
            count={t("accounts.agentTurns", { count: local.today.turnCount })} input={local.today.inputTokens}
            output={local.today.outputTokens + local.today.reasoningTokens}
            cache={local.today.cacheReadTokens + local.today.cacheWriteTokens} accent="local" />
          <TodayUsageCard label={`${t("accounts.todayUsage")} · ${t("accounts.relayUsage")}`} total={relay.today.totalTokens}
            count={t("accounts.relayRequests", { count: relay.today.requestCount })} input={relay.today.inputTokens}
            output={relay.today.outputTokens} cache={0} accent="relay" />
        </div>

        <div className="usage-range-summary">
          <span>{t("accounts.weekUsage")} <strong>{formatTokens(local.week.totalTokens)}</strong></span>
          <span>{t("accounts.allUsage")} <strong>{formatTokens(local.allTime.totalTokens)}</strong></span>
          <span>{t("accounts.relayUsage")} <strong>{formatTokens(relay.allTime.totalTokens)}</strong></span>
        </div>

        <div>
          <div className="mb-2 flex items-center justify-between gap-2">
            <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
              {t("accounts.localAgentUsage")}
            </h4>
            <span className="font-mono text-[10px] text-muted-foreground">
              {formatTokens(local.allTime.totalTokens)} Token
            </span>
          </div>
          <div className="grid gap-3 md:grid-cols-2">
            {local.bySource.map((source) => (
              <SourceUsageCard
                key={source.sourceId}
                name={source.sourceName}
                badge={
                  source.available
                    ? t("accounts.filesScanned", { count: source.fileCount })
                    : t("accounts.sourceNotFound")
                }
                totalTokens={source.totals.totalTokens}
                count={source.totals.turnCount}
                inputTokens={source.totals.inputTokens}
                outputTokens={source.totals.outputTokens + source.totals.reasoningTokens}
                cacheTokens={source.totals.cacheReadTokens + source.totals.cacheWriteTokens}
                share={localShareBase}
              />
            ))}
          </div>
        </div>

        <DailyTrendPanel daily={local.daily} />
        <UsageBreakdown title={t("accounts.localModels")} items={localModels} emptyLabel={t("accounts.localUsageEmpty")} />

        <RelayUsagePanel relay={relay} onManageProviders={onManageProviders} />

        <RecentTurnsPanel items={local.recent} />

        {local.warningCount > 0 && (
          <details className="rounded-xl border border-amber-500/25 bg-amber-500/5 px-4 py-3 text-xs text-amber-800 dark:text-amber-300">
            <summary className="cursor-pointer font-medium">
              {t("accounts.scanWarnings", { count: local.warningCount })}
            </summary>
            <ul className="mt-2 space-y-1 pl-4">
              {local.warnings.map((warning) => (
                <li key={warning} className="list-disc">{warning}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
    </section>
  );
}
