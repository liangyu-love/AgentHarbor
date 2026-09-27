import { useContext, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { PageActiveContext } from "@/components/layout/page-stage";
import {
  Activity,
  BarChart3,
  Calendar,
  Check,
  Clock3,
  Copy,
  Database,
  Loader2,
  LogIn,
  RefreshCw,
  Search,
  Shield,
  ShieldCheck,
  Sparkles,
  Trash2,
  User,
  UsersRound,
} from "lucide-react";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/hooks/use-toast";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { api } from "@/lib/api";
import { formatRelative } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import type {
  AccountRateLimitWindow,
  CodexAccountSummary,
  TokenUsagePayload,
  TokenUsageTotals,
} from "@/types";

const QUERY_KEY = ["codex-accounts"] as const;
const TOKEN_USAGE_QUERY_KEY = ["token-usage"] as const;

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

function CompositionBar({
  input,
  output,
  cache,
  className,
}: {
  input: number;
  output: number;
  cache: number;
  className?: string;
}) {
  const total = Math.max(1, input + output + cache);
  const parts = [
    { key: "input", value: input, className: "bg-sky-500/75" },
    { key: "output", value: output, className: "bg-primary/75" },
    { key: "cache", value: cache, className: "bg-emerald-500/70" },
  ];
  return (
    <div className={cn("flex h-1.5 w-full overflow-hidden rounded-full bg-muted/80", className)}>
      {parts.map((part) => (
        <div
          key={part.key}
          className={cn("h-full transition-all duration-300", part.className)}
          style={{ width: `${(part.value / total) * 100}%` }}
        />
      ))}
    </div>
  );
}

function TokenTotalCard({
  label,
  totals,
  emphasize,
}: {
  label: string;
  totals: TokenUsageTotals;
  emphasize?: boolean;
}) {
  const { t } = useTranslation();
  const cache = totals.cacheReadTokens + totals.cacheWriteTokens;
  return (
    <div className={cn("min-w-0 p-3.5 sm:p-4", emphasize && "bg-primary/[0.04]")}>
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-muted-foreground">{label}</p>
        <span className="font-mono text-[10px] text-muted-foreground/80">
          {t("accounts.agentTurns", { count: totals.turnCount })}
        </span>
      </div>
      <p className="mt-1 text-2xl font-semibold tracking-tight tabular-nums sm:text-[28px] leading-none">
        {formatTokens(totals.totalTokens)}
        <span className="ml-1 text-xs font-normal text-muted-foreground">Token</span>
      </p>
      <CompositionBar
        className="mt-2.5"
        input={totals.inputTokens}
        output={totals.outputTokens + totals.reasoningTokens}
        cache={cache}
      />
      <div className="mt-2 grid grid-cols-3 gap-1 text-[10px] text-muted-foreground">
        <span className="flex items-center gap-1">
          <i className="h-1.5 w-1.5 rounded-full bg-sky-500/75" />
          {t("accounts.inputShort")} {formatTokens(totals.inputTokens)}
        </span>
        <span className="flex items-center gap-1">
          <i className="h-1.5 w-1.5 rounded-full bg-primary/75" />
          {t("accounts.outputShort")} {formatTokens(totals.outputTokens + totals.reasoningTokens)}
        </span>
        <span className="flex items-center gap-1">
          <i className="h-1.5 w-1.5 rounded-full bg-emerald-500/70" />
          {t("accounts.cacheShort")} {formatTokens(cache)}
        </span>
      </div>
    </div>
  );
}

type UsageMeasure = { totalTokens: number };

function cleanModelTitle(title: string) {
  if (title.startsWith("aimami_relay_")) {
    const parts = title.split(" ");
    if (parts.length > 1) {
      return parts.slice(1).join(" ");
    }
  }
  return title;
}

function UsageBreakdown({ title, items, emptyLabel }: {
  title: string;
  items: Array<{ key: string; title: string; subtitle?: string; totals: UsageMeasure }>;
  emptyLabel: string;
}) {
  const max = Math.max(1, ...items.map((item) => item.totals.totalTokens));
  const sum = items.reduce((acc, item) => acc + item.totals.totalTokens, 0);
  return (
    <div className="flex flex-col justify-between rounded-2xl border border-border/80 bg-card/85 p-4 sm:p-5 backdrop-blur-md shadow-xs">
      <div className="flex items-center justify-between gap-2 border-b border-border/50 pb-3">
        <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-foreground/90">{title}</h4>
        {items.length > 0 && (
          <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-[11px] font-semibold text-foreground">
            {formatTokens(sum)}
          </span>
        )}
      </div>
      {items.length === 0 ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{emptyLabel}</p>
      ) : (
        <div className="mt-3 space-y-3">
          {items.slice(0, 6).map((item, index) => {
            const rank = index + 1;
            const cleanedTitle = cleanModelTitle(item.title);
            return (
              <div key={item.key} className="group">
                <div className="flex items-center justify-between gap-2 text-xs">
                  <div className="flex min-w-0 items-center gap-2">
                    <span
                      className={cn(
                        "flex h-4 w-4 shrink-0 items-center justify-center rounded text-[10px] font-bold font-mono",
                        rank === 1 && "bg-amber-500/20 text-amber-700 dark:text-amber-300 ring-1 ring-amber-500/35",
                        rank === 2 && "bg-slate-300/40 text-slate-700 dark:text-slate-200 ring-1 ring-slate-400/30",
                        rank === 3 && "bg-amber-700/15 text-amber-800 dark:text-amber-400 ring-1 ring-amber-700/30",
                        rank > 3 && "bg-muted text-muted-foreground",
                      )}
                    >
                      {rank}
                    </span>
                    <p className="truncate font-medium text-foreground/90" title={item.title}>
                      {cleanedTitle}
                    </p>
                    {item.subtitle && (
                      <span className="shrink-0 rounded bg-muted/70 px-1.5 py-0.2 text-[9px] font-mono text-muted-foreground">
                        {item.subtitle}
                      </span>
                    )}
                  </div>
                  <div className="shrink-0 text-right">
                    <span className="font-mono text-[11px] font-medium text-foreground">
                      {formatTokens(item.totals.totalTokens)}
                    </span>
                    <span className="ml-1.5 rounded bg-primary/10 px-1 py-0.2 font-mono text-[10px] font-medium text-primary">
                      {formatPercent(item.totals.totalTokens, sum)}
                    </span>
                  </div>
                </div>
                <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted/80 ring-1 ring-border/40">
                  <div
                    className={cn(
                      "h-full rounded-full transition-all duration-300",
                      rank === 1
                        ? "bg-gradient-to-r from-primary to-amber-500 shadow-[0_0_8px_hsl(var(--primary)/0.35)]"
                        : "bg-primary/75 group-hover:bg-primary",
                    )}
                    style={{ width: `${Math.max(3, (item.totals.totalTokens / max) * 100)}%` }}
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
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
    <div className={cn("rounded-xl border border-border/70 bg-background/65 p-4", className)}>
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
      <CompositionBar
        className="mt-3"
        input={inputTokens}
        output={outputTokens}
        cache={cacheTokens}
      />
      <div className="mt-2.5 grid grid-cols-3 gap-2 text-[10px] text-muted-foreground">
        <div>
          <p>{t("accounts.inputShort")}</p>
          <p className="mt-0.5 font-mono text-foreground">{formatTokens(inputTokens)}</p>
        </div>
        <div>
          <p>{t("accounts.outputShort")}</p>
          <p className="mt-0.5 font-mono text-foreground">{formatTokens(outputTokens)}</p>
        </div>
        <div>
          <p>{t("accounts.cacheShort")}</p>
          <p className="mt-0.5 font-mono text-foreground">{formatTokens(cacheTokens)}</p>
        </div>
      </div>
      <div className="mt-2 truncate text-[10px] text-muted-foreground/80" title={badge}>
        {badge}
      </div>
    </div>
  );
}

function TokenUsagePanel({ data, loading, error, onRetry }: {
  data: TokenUsagePayload | undefined;
  loading: boolean;
  error: unknown;
  onRetry: () => void;
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
        <p className="mt-2 break-all font-mono">{String(error)}</p>
      </details>
    </section>
  );
  if (!data) return null;

  const local = data.local;
  const relay = data.relay;
  const maxDaily = Math.max(1, ...local.daily.map((day) => day.totals.totalTokens));
  const localModels = local.byModel.map((item) => ({
    key: item.sourceId + "::" + item.model,
    title: item.model,
    subtitle: item.sourceName,
    totals: item.totals,
  }));
  const relayModels = relay.byModel.map((item) => ({
    key: item.providerId + "::" + item.model,
    title: item.model,
    subtitle: item.providerName,
    totals: item.totals,
  }));
  const localShareBase = Math.max(1, local.allTime.totalTokens);

  return (
    <section className="overflow-hidden rounded-2xl border border-border/75 bg-card/75 shadow-sm">
      <div className="flex flex-col gap-3 border-b border-border/70 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <div className="rounded-xl bg-primary/10 p-2.5 text-primary"><Activity className="h-5 w-5" /></div>
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="font-semibold tracking-tight">{t("accounts.tokenMonitor")}</h3>
              <Badge variant="outline" className="border-primary/20 bg-primary/5 text-[10px] text-primary">
                Codex + Claude Code + Relay
              </Badge>
            </div>
            <p className="mt-1 max-w-3xl text-xs leading-relaxed text-muted-foreground">
              {t("accounts.tokenMonitorDesc")}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1.5">
            <Database className="h-3.5 w-3.5" />
            {t("accounts.incrementalLedger")}
          </span>
          <span className="flex items-center gap-1.5">
            <Clock3 className="h-3.5 w-3.5" />
            {t("accounts.lastUpdated", { time: new Date(data.generatedAt * 1000).toLocaleString() })}
          </span>
        </div>
      </div>

      <div className="space-y-4 p-4 sm:p-5">
        <div className="grid divide-y divide-border/70 overflow-hidden rounded-xl border border-border/70 bg-background/65 sm:grid-cols-3 sm:divide-x sm:divide-y-0">
          <TokenTotalCard label={t("accounts.todayUsage")} totals={local.today} emphasize />
          <TokenTotalCard label={t("accounts.weekUsage")} totals={local.week} />
          <TokenTotalCard label={t("accounts.allUsage")} totals={local.allTime} />
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

        <div className="grid gap-4 lg:grid-cols-[1.35fr_1fr]">
          <div className="flex flex-col justify-between rounded-2xl border border-border/80 bg-card/85 p-4 sm:p-5 backdrop-blur-md shadow-xs">
            {/* Header with detailed hover info */}
            <div>
              <div className="flex items-center justify-between gap-3 border-b border-border/50 pb-3">
                <h4 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.12em] text-foreground/90">
                  <BarChart3 className="h-4 w-4 text-primary" />
                  {t("accounts.dailyTrend")}
                </h4>
                <div className="flex items-center gap-3 text-[11px] text-muted-foreground">
                  <span className="flex items-center gap-1.5 font-medium">
                    <span className="h-2 w-2 rounded-full bg-primary" />
                    {t("accounts.last14Days")}
                  </span>
                  <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-[11px] font-semibold text-foreground">
                    峰值 {formatTokens(maxDaily)}
                  </span>
                </div>
              </div>
            </div>

            {/* Daily Stacked Bar Chart */}
            <div className="mt-4 flex h-44 items-end gap-1.5 sm:gap-2">
              {local.daily.map((day, index) => {
                const isToday = index === local.daily.length - 1;
                const total = day.totals.totalTokens;
                const heightPercent = total === 0 ? 3 : Math.max(8, (total / maxDaily) * 100);
                
                const input = day.totals.inputTokens;
                const output = day.totals.outputTokens + day.totals.reasoningTokens;
                const cache = day.totals.cacheReadTokens + day.totals.cacheWriteTokens;
                
                const inputPct = total > 0 ? (input / total) * 100 : 0;
                const outputPct = total > 0 ? (output / total) * 100 : 0;
                const cachePct = total > 0 ? (cache / total) * 100 : 0;

                const dayLabel = `${day.date} · 总量: ${formatTokens(total)} (入: ${formatTokens(input)} / 出: ${formatTokens(output)} / 缓: ${formatTokens(cache)}) · ${t("accounts.agentTurns", { count: day.totals.turnCount })}`;

                return (
                  <div
                    key={day.date}
                    className="group relative flex min-w-0 flex-1 flex-col items-center justify-end gap-1.5 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/50"
                    tabIndex={0}
                    role="img"
                    aria-label={dayLabel}
                  >
                    {/* Floating Tooltip */}
                    <div className="pointer-events-none absolute bottom-[calc(100%+8px)] left-1/2 z-30 hidden w-max max-w-56 -translate-x-1/2 flex-col gap-1 rounded-lg border border-border/80 bg-popover/95 p-2 text-[10px] text-popover-foreground shadow-xl backdrop-blur-md group-hover:flex group-focus:flex">
                      <div className="flex items-center justify-between gap-2 border-b border-border/60 pb-1 font-semibold">
                        <span>{day.date} {isToday && "(今日)"}</span>
                        <span className="font-mono text-primary">{formatTokens(total)}</span>
                      </div>
                      <div className="space-y-0.5 font-mono text-[9px]">
                        <div className="flex justify-between gap-3 text-sky-600 dark:text-sky-400">
                          <span>输入</span>
                          <span>{formatTokens(input)}</span>
                        </div>
                        <div className="flex justify-between gap-3 text-primary">
                          <span>输出+思考</span>
                          <span>{formatTokens(output)}</span>
                        </div>
                        <div className="flex justify-between gap-3 text-emerald-600 dark:text-emerald-400">
                          <span>上下文缓存</span>
                          <span>{formatTokens(cache)}</span>
                        </div>
                        <div className="mt-1 flex justify-between gap-3 border-t border-border/40 pt-1 text-muted-foreground">
                          <span>对话回合</span>
                          <span>{day.totals.turnCount} 轮</span>
                        </div>
                      </div>
                    </div>

                    {/* Stacked Vertical Bar */}
                    <div className="relative flex h-32 w-full items-end justify-center">
                      <div
                        className={cn(
                          "relative flex w-full min-w-1.5 flex-col-reverse overflow-hidden rounded-t-md transition-all duration-200 group-hover:scale-x-105",
                          isToday && "ring-2 ring-primary ring-offset-1 ring-offset-background shadow-[0_0_12px_hsl(var(--primary)/0.35)]",
                          total === 0 && "bg-muted/40",
                        )}
                        style={{ height: `${heightPercent}%` }}
                      >
                        {total > 0 && (
                          <>
                            {/* Layer 1: Input Tokens (Bottom) */}
                            <div
                              className="w-full bg-sky-500/80 transition-colors group-hover:bg-sky-500"
                              style={{ height: `${inputPct}%` }}
                              title={`输入: ${formatTokens(input)}`}
                            />
                            {/* Layer 2: Output & Reasoning Tokens (Middle) */}
                            <div
                              className="w-full bg-primary/85 transition-colors group-hover:bg-primary"
                              style={{ height: `${outputPct}%` }}
                              title={`输出: ${formatTokens(output)}`}
                            />
                            {/* Layer 3: Cache Tokens (Top) */}
                            <div
                              className="w-full bg-emerald-500/80 transition-colors group-hover:bg-emerald-500"
                              style={{ height: `${cachePct}%` }}
                              title={`缓存: ${formatTokens(cache)}`}
                            />
                          </>
                        )}
                      </div>
                    </div>

                    {/* Date Axis Label */}
                    <span
                      className={cn(
                        "text-[10px] font-mono tracking-tight",
                        isToday
                          ? "rounded bg-primary/15 px-1 font-bold text-primary"
                          : "text-muted-foreground",
                      )}
                    >
                      {isToday ? "今" : Number(day.date.slice(-2)) % 2 === 0 ? day.date.slice(5) : ""}
                    </span>
                  </div>
                );
              })}
            </div>

            {/* Legend bar */}
            <div className="mt-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-1.5 border-t border-border/50 pt-3 text-[11px] text-muted-foreground">
              <div className="flex items-center gap-3">
                <span className="flex items-center gap-1.5">
                  <i className="h-2 w-2 rounded-full bg-sky-500" />
                  {t("accounts.inputShort")}
                </span>
                <span className="flex items-center gap-1.5">
                  <i className="h-2 w-2 rounded-full bg-primary" />
                  {t("accounts.outputShort")}
                </span>
                <span className="flex items-center gap-1.5">
                  <i className="h-2 w-2 rounded-full bg-emerald-500" />
                  {t("accounts.cacheShort")}
                </span>
              </div>
              <span className="font-mono text-[10px] text-muted-foreground/80">
                按日聚合统计
              </span>
            </div>
          </div>
          <UsageBreakdown
            title={t("accounts.localModels")}
            items={localModels}
            emptyLabel={t("accounts.localUsageEmpty")}
          />
        </div>

        <div className="rounded-xl border border-dashed border-border/80 bg-muted/15 p-3 sm:p-4">
          <div className="mb-3 flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
              {t("accounts.relayUsage")}
            </h4>
            <span className="text-[10px] leading-4 text-muted-foreground">
              {t("accounts.relaySeparateHint")}
            </span>
          </div>
          <div className="grid gap-3 lg:grid-cols-[1fr_1.2fr]">
            <SourceUsageCard
              name="AgentHarbor Relay"
              badge={t("accounts.relayRequests", { count: relay.allTime.requestCount })}
              totalTokens={relay.allTime.totalTokens}
              count={relay.allTime.reportedCount}
              countLabel={t("accounts.relayReported", { count: relay.allTime.reportedCount })}
              inputTokens={relay.allTime.inputTokens}
              outputTokens={relay.allTime.outputTokens}
              cacheTokens={0}
              share={Math.max(1, relay.allTime.totalTokens)}
              className="bg-background/55"
            />
            <UsageBreakdown
              title={t("accounts.relayModels")}
              items={relayModels}
              emptyLabel={t("accounts.relayEmpty")}
            />
          </div>
        </div>

        {local.recent.length > 0 && (
          <div className="overflow-hidden rounded-xl border border-border/70 bg-background/55">
            <div className="flex items-center justify-between gap-2 border-b border-border/70 px-4 py-3">
              <span className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                {t("accounts.recentAgentTurns")}
              </span>
              <span className="text-[10px] text-muted-foreground">
                {t("accounts.recentAgentTurns")} · {Math.min(8, local.recent.length)}
              </span>
            </div>
            <div className="hidden grid-cols-[120px_minmax(0,1fr)_170px_90px] gap-4 border-b border-border/60 bg-muted/20 px-4 py-2 text-[10px] font-medium text-muted-foreground sm:grid">
              <span>{t("accounts.sourceColumn")}</span>
              <span>{t("accounts.modelColumn")}</span>
              <span>{t("accounts.timeColumn")}</span>
              <span className="text-right">{t("accounts.tokenColumn")}</span>
            </div>
            <div className="divide-y divide-border/60">
              {local.recent.slice(0, 8).map((item, index) => (
                <div
                  key={item.timestamp + "-" + item.sourceId + "-" + index}
                  className="grid grid-cols-[minmax(0,1fr)_auto] gap-4 px-4 py-2.5 text-xs transition-colors hover:bg-muted/25 sm:grid-cols-[120px_minmax(0,1fr)_170px_90px]"
                >
                  <div className="min-w-0 sm:hidden">
                    <p className="font-medium">{item.sourceName}</p>
                    <p className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground" title={item.model}>
                      {item.model}
                    </p>
                    <p className="mt-0.5 text-[10px] text-muted-foreground">
                      {new Date(item.timestamp * 1000).toLocaleString()}
                    </p>
                  </div>
                  <div className="hidden items-center gap-1.5 font-medium sm:flex">
                    <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary/70" />
                    <span className="truncate">{item.sourceName}</span>
                  </div>
                  <div className="hidden truncate font-mono text-[11px] text-muted-foreground sm:block" title={item.model}>
                    {item.model}
                  </div>
                  <div className="hidden truncate text-muted-foreground sm:block">
                    {new Date(item.timestamp * 1000).toLocaleString()}
                  </div>
                  <div className="text-right font-mono tabular-nums text-foreground">
                    {formatTokens(item.totalTokens)}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

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

function planLabel(plan: string, unknown: string) {
  const labels: Record<string, string> = {
    free: "Free",
    plus: "Plus",
    pro5x: "Pro 5x",
    pro20x: "Pro 20x",
    team: "Team",
    business: "Business",
    enterprise: "Enterprise",
    edu: "Edu",
  };
  return labels[plan.toLowerCase()] ?? (plan && plan !== "unknown" ? plan : unknown);
}

function statusStyle(status: CodexAccountSummary["usageStatus"]) {
  if (status === "healthy") return "bg-emerald-500";
  if (status === "expired") return "bg-amber-500";
  if (status === "error") return "bg-destructive";
  return "bg-muted-foreground/40";
}

function QuotaRing({
  label,
  window,
}: {
  label: string;
  window: AccountRateLimitWindow | null;
}) {
  const value = window?.remainingPercent ?? null;
  const radius = 30;
  const circumference = Math.PI * 2 * radius;
  const offset = value == null ? circumference : circumference * (1 - value / 100);
  const tone = value == null
    ? "stroke-muted-foreground/20 text-muted-foreground"
    : value <= 15
      ? "stroke-rose-500 text-rose-500"
      : value <= 35
        ? "stroke-amber-500 text-amber-500"
        : "stroke-emerald-500 text-emerald-500";

  // Parse reset time
  let resetTimeFormatted = "—";
  if (window?.resetsAt) {
    const d = new Date(window.resetsAt * 1000);
    const dateStr = `${d.getMonth() + 1}/${d.getDate()}`;
    const timeStr = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
    resetTimeFormatted = `${dateStr} ${timeStr} 重置`;
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col items-center justify-center p-2">
      <div className="relative h-[82px] w-[82px]">
        <svg viewBox="0 0 72 72" className="h-full w-full -rotate-90" aria-hidden>
          <circle cx="36" cy="36" r={radius} fill="none" strokeWidth="5.5" className="stroke-muted/50 dark:stroke-muted/30" />
          <circle
            cx="36"
            cy="36"
            r={radius}
            fill="none"
            strokeWidth="5.5"
            strokeLinecap="round"
            strokeDasharray={circumference}
            strokeDashoffset={offset}
            className={cn("transition-all duration-500", tone)}
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center">
          <span className={cn("text-base font-bold font-mono tracking-tight", tone)}>
            {value == null ? "—" : `${value}%`}
          </span>
        </div>
      </div>
      <div className="mt-2 text-center">
        <p className="text-xs font-semibold text-foreground/90 font-mono tracking-tight">
          <span className="font-bold">{label}</span>
          <span className="mx-1 text-muted-foreground/50">·</span>
          <span className="text-muted-foreground">{resetTimeFormatted}</span>
        </p>
      </div>
    </div>
  );
}

function AccountCard({
  account,
  switching,
  refreshing,
  onSwitch,
  onDelete,
  onRefresh,
}: {
  account: CodexAccountSummary;
  switching: boolean;
  refreshing: boolean;
  onSwitch: () => void;
  onDelete: () => void;
  onRefresh: () => void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const statusText = t(`accounts.${account.usageStatus}`);
  const title = account.email || account.displayName;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(account.email || account.displayName);
      setCopied(true);
      toast({ title: "已复制账号", description: account.email || account.displayName });
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast({ title: "复制失败", variant: "destructive" });
    }
  };

  const isPlus = /plus/i.test(account.plan);
  const isTeamOrPro = /pro|team|business|enterprise/i.test(account.plan);

  return (
    <article
      className={cn(
        "group relative flex flex-col justify-between overflow-hidden rounded-3xl border bg-card/90 backdrop-blur-md transition-all duration-300",
        account.isActive
          ? "border-emerald-500/40 ring-1 ring-emerald-500/20 shadow-[0_12px_32px_-12px_rgba(16,185,129,0.18)]"
          : "border-border/80 hover:border-primary/40 hover:shadow-lg hover:-translate-y-0.5",
      )}
    >
      {/* Top Header: Email Title + Status Badge */}
      <div className="p-4 sm:p-5 pb-3">
        <div className="flex items-center justify-between gap-2.5">
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <span
              className={cn(
                "h-2.5 w-2.5 shrink-0 rounded-full",
                statusStyle(account.usageStatus),
                account.isActive && "shadow-[0_0_8px_hsl(142,76%,36%)]"
              )}
            />
            <h3
              className="truncate text-[15px] font-bold tracking-tight text-foreground/95"
              title={title}
            >
              {title}
            </h3>
          </div>
        </div>

        {/* Badges line: Plan Badge + Active Badge */}
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          <Badge
            variant="outline"
            className={cn(
              "gap-1 rounded-full px-2.5 py-0.5 text-xs font-semibold tracking-wide",
              isPlus && "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
              isTeamOrPro && "border-primary/30 bg-primary/10 text-primary",
              !isPlus && !isTeamOrPro && "border-border bg-muted/60 text-muted-foreground",
            )}
          >
            <Sparkles className="h-3 w-3" />
            {planLabel(account.plan, t("accounts.planUnknown"))}
          </Badge>

          {account.isActive && (
            <Badge className="gap-1 rounded-full border border-emerald-500/25 bg-emerald-500/15 px-2.5 py-0.5 text-xs font-medium text-emerald-700 shadow-none dark:text-emerald-300">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              {t("accounts.current")}
            </Badge>
          )}
        </div>
      </div>

      {/* Middle: 2x2 Info Grid with clean icons */}
      <div className="grid grid-cols-2 gap-3 border-y border-border/50 bg-black/[0.015] dark:bg-white/[0.015] px-4 sm:px-5 py-3.5 text-xs">
        {/* User Nickname */}
        <div className="flex min-w-0 items-start gap-2.5">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground">
            <User className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">用户昵称</p>
            <p className="truncate font-semibold text-foreground/90" title={account.displayName}>
              {account.displayName || "—"}
            </p>
          </div>
        </div>

        {/* Auth Mode */}
        <div className="flex min-w-0 items-start gap-2.5">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground">
            <Shield className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">认证模式</p>
            <p className="truncate font-semibold text-foreground/90">
              {t("accounts.chatgptOauth")}
            </p>
          </div>
        </div>

        {/* Subscription Status */}
        <div className="flex min-w-0 items-start gap-2.5">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground">
            <ShieldCheck className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">订阅状态</p>
            <p className="truncate font-semibold text-foreground/90">
              {account.usageStatus === "healthy" ? "有效订阅" : statusText}
            </p>
          </div>
        </div>

        {/* Expiry or Last checked */}
        <div className="flex min-w-0 items-start gap-2.5">
          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground">
            <Calendar className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">最近同步</p>
            <p className="truncate font-semibold text-foreground/90" title={account.lastUsageAt ? new Date(account.lastUsageAt * 1000).toLocaleString() : undefined}>
              {account.lastUsageAt
                ? formatRelative(account.lastUsageAt)
                : t("accounts.unknown")}
            </p>
          </div>
        </div>
      </div>

      {/* Quota Ring Dual Displays */}
      <div className="flex items-center justify-around divide-x divide-border/50 px-2 py-4">
        <QuotaRing label="5小时" window={account.primaryWindow} />
        <QuotaRing label="1周" window={account.secondaryWindow} />
      </div>

      {account.usageError && (
        <p className="mx-4 mb-3 rounded-xl bg-destructive/10 px-3 py-2 text-[11px] leading-relaxed text-destructive border border-destructive/20">
          {account.usageError}
        </p>
      )}

      {/* Footer Actions */}
      <div className="flex items-center justify-between border-t border-border/50 bg-black/[0.015] dark:bg-white/[0.015] px-4 py-3">
        {/* Left: Active indicator or Set Active Button */}
        <div>
          {account.isActive ? (
            <div className="flex items-center gap-1.5 font-semibold text-emerald-600 dark:text-emerald-400 text-xs">
              <Check className="h-4 w-4" />
              <span>当前账号</span>
            </div>
          ) : (
            <Button
              variant="outline"
              size="xs"
              className="h-8 gap-1.5 rounded-xl border-border/80 px-3 text-xs font-semibold hover:border-primary/40 hover:bg-primary/5 hover:text-primary transition-all shadow-2xs"
              onClick={onSwitch}
              disabled={switching}
            >
              {switching ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <LogIn className="h-3.5 w-3.5" />}
              <span>{switching ? t("accounts.switching") : t("accounts.setCurrent")}</span>
            </Button>
          )}
        </div>

        {/* Right: Refresh Quota Button, Copy Button, Delete Button in soft pill capsule */}
        <div className="flex items-center gap-1 rounded-2xl bg-muted/50 p-1 border border-border/50">
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 rounded-xl text-muted-foreground hover:bg-background hover:text-foreground hover:shadow-xs transition-all"
            onClick={onRefresh}
            disabled={refreshing}
            title="刷新该账号额度"
          >
            <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin text-primary")} />
          </Button>

          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 rounded-xl text-muted-foreground hover:bg-background hover:text-foreground hover:shadow-xs transition-all"
            onClick={handleCopy}
            title="复制账号名称"
          >
            {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          </Button>

          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 rounded-xl text-muted-foreground hover:bg-destructive/15 hover:text-destructive hover:shadow-xs transition-all"
            onClick={onDelete}
            disabled={account.isActive}
            title={account.isActive ? t("accounts.activeDeleteBlocked") : t("accounts.remove")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </article>
  );
}

export function AccountsPage({ refreshInterval }: { refreshInterval: RefreshInterval }) {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [plan, setPlan] = useState("all");
  const [deleteTarget, setDeleteTarget] = useState<CodexAccountSummary | null>(null);

  const accountsQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: api.accounts.refresh,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    retry: false,
  });
  const tokenUsageQuery = useQuery({
    queryKey: TOKEN_USAGE_QUERY_KEY,
    queryFn: api.usage.overview,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    retry: false,
  });

  const setData = (data: Awaited<ReturnType<typeof api.accounts.list>>) => {
    queryClient.setQueryData(QUERY_KEY, data);
  };

  const loginMutation = useMutation({
    mutationFn: api.accounts.login,
    onSuccess: (data) => {
      setData(data);
      toast({ title: t("accounts.loginSuccess"), variant: "success" });
      void accountsQuery.refetch();
    },
    onError: (error) => {
      toast({ title: t("common.error"), description: String(error), variant: "destructive" });
    },
  });

  const switchMutation = useMutation({
    mutationFn: async (accountKey: string) => {
      const data = await api.accounts.switch(accountKey);
      try {
        await api.targets.restart("codex");
      } catch {
        // The auth switch is already complete; a later Codex launch will use it.
      }
      return data;
    },
    onSuccess: (data) => {
      setData(data);
      toast({ title: t("accounts.switchSuccess"), variant: "success" });
      void accountsQuery.refetch();
    },
    onError: (error) => {
      toast({ title: t("common.error"), description: String(error), variant: "destructive" });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: api.accounts.delete,
    onSuccess: (data) => {
      setData(data);
      setDeleteTarget(null);
      toast({ title: t("accounts.deleteSuccess"), variant: "success" });
    },
    onError: (error) => {
      toast({ title: t("common.error"), description: String(error), variant: "destructive" });
    },
  });

  const refreshQuotaMutation = useMutation({
    mutationFn: async (_key: string) => {
      return await api.accounts.refresh();
    },
    onSuccess: (data) => {
      setData(data);
      toast({ title: "账号额度已刷新", variant: "success" });
    },
    onError: (error) => {
      toast({ title: "刷新失败", description: String(error), variant: "destructive" });
    },
  });

  const refresh = async () => {
    const [accountsResult] = await Promise.all([accountsQuery.refetch(), tokenUsageQuery.refetch()]);
    if (accountsResult.data) toast({ title: t("accounts.refreshSuccess"), variant: "success" });
  };

  const accounts = accountsQuery.data?.items ?? [];
  const plans = useMemo(
    () => Array.from(new Set(accounts.map((account) => account.plan).filter(Boolean))).sort(),
    [accounts],
  );
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return accounts.filter((account) => {
      if (plan !== "all" && account.plan !== plan) return false;
      if (!needle) return true;
      return `${account.email} ${account.displayName}`.toLowerCase().includes(needle);
    });
  }, [accounts, plan, search]);

  return (
    <div className="mx-auto max-w-7xl space-y-5 pb-8">
        <section className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div>
            <div className="flex items-center gap-2">
              <h2 className="text-xl font-semibold tracking-tight">{t("accounts.title")}</h2>
              <Badge variant="outline" className="border-primary/25 bg-primary/5 text-[10px] text-primary">
                {t("accounts.official")}
              </Badge>
              <Badge variant="outline" className="text-[10px] text-muted-foreground">
                {t("accounts.direct")}
              </Badge>
            </div>
            <p className="mt-1.5 text-sm text-muted-foreground">{t("accounts.description")}</p>
            <p className="mt-1 text-xs text-muted-foreground/80">{t("accounts.loginHint")}</p>
          </div>
          <div className="flex shrink-0 gap-2">
            <Button
              variant="outline"
              onClick={() => void refresh()}
              disabled={accountsQuery.isFetching || tokenUsageQuery.isFetching}
            >
              <RefreshCw className={cn((accountsQuery.isFetching || tokenUsageQuery.isFetching) && "animate-spin")} />
              {t("common.refresh")}
            </Button>
            <Button onClick={() => loginMutation.mutate()} disabled={loginMutation.isPending}>
              {loginMutation.isPending ? <Loader2 className="animate-spin" /> : <LogIn />}
              {loginMutation.isPending ? t("accounts.adding") : t("accounts.add")}
            </Button>
          </div>
        </section>

        <TokenUsagePanel
          data={tokenUsageQuery.data}
          loading={tokenUsageQuery.isPending}
          error={tokenUsageQuery.error}
          onRetry={() => void tokenUsageQuery.refetch()}
        />

        <section className="flex flex-col gap-3 rounded-2xl border border-border/70 bg-card/70 p-3 shadow-sm sm:flex-row sm:items-center">
          <div className="relative flex-1">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              placeholder={t("accounts.search")}
              className="h-10 pl-9"
            />
          </div>
          <Select value={plan} onValueChange={setPlan}>
            <SelectTrigger className="h-10 w-full sm:w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">{t("accounts.allPlans")}</SelectItem>
              {plans.map((value) => (
                <SelectItem key={value} value={value}>
                  {planLabel(value, t("accounts.planUnknown"))}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div className="flex items-center gap-2 px-2 text-xs text-muted-foreground">
            <span className="relative flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-50" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-emerald-500" />
            </span>
            {t("accounts.official")}
          </div>
        </section>

        {accountsQuery.isPending ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 3 }).map((_, index) => (
              <Skeleton key={index} className="h-[390px] rounded-2xl" />
            ))}
          </div>
        ) : accountsQuery.isError ? (
          <div className="rounded-2xl border border-destructive/30 bg-destructive/5 p-6 text-sm text-destructive">
            {String(accountsQuery.error)}
          </div>
        ) : accounts.length === 0 ? (
          <div className="flex min-h-80 flex-col items-center justify-center rounded-2xl border border-dashed border-border bg-card/50 px-6 text-center">
            <div className="mb-4 rounded-2xl bg-primary/10 p-4 text-primary">
              <UsersRound className="h-7 w-7" />
            </div>
            <h3 className="font-semibold">{t("accounts.emptyTitle")}</h3>
            <p className="mt-1 max-w-md text-sm text-muted-foreground">{t("accounts.emptyDescription")}</p>
            <Button className="mt-5" onClick={() => loginMutation.mutate()} disabled={loginMutation.isPending}>
              <LogIn />
              {t("accounts.add")}
            </Button>
          </div>
        ) : filtered.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-border p-10 text-center text-sm text-muted-foreground">
            {t("accounts.noMatch")}
          </div>
        ) : (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {filtered.map((account) => (
              <AccountCard
                key={account.accountKey}
                account={account}
                switching={switchMutation.isPending && switchMutation.variables === account.accountKey}
                refreshing={refreshQuotaMutation.isPending && refreshQuotaMutation.variables === account.accountKey}
                onSwitch={() => switchMutation.mutate(account.accountKey)}
                onDelete={() => setDeleteTarget(account)}
                onRefresh={() => refreshQuotaMutation.mutate(account.accountKey)}
              />
            ))}
          </div>
        )}

        <AlertDialog open={Boolean(deleteTarget)} onOpenChange={(open) => !open && setDeleteTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("accounts.removeTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("accounts.removeDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={(event) => {
                event.preventDefault();
                if (deleteTarget) deleteMutation.mutate(deleteTarget.accountKey);
              }}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending && <Loader2 className="animate-spin" />}
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
