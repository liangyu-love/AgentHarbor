import { useContext, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  Activity,
  Calendar,
  Check,
  Clock3,
  Copy,
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

import { DailyTrendPanel } from "@/components/accounts/daily-trend-panel";
import { UsageBreakdown } from "@/components/accounts/model-usage-breakdown";
import { RecentTurnsPanel } from "@/components/accounts/recent-turns-panel";
import { PageActiveContext } from "@/components/layout/page-stage";

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

        <div className="grid gap-4 xl:grid-cols-[1.55fr_1fr]">
          <DailyTrendPanel daily={local.daily} models={localModels} />
          <UsageBreakdown
            title={t("accounts.localModels")}
            items={localModels}
            emptyLabel={t("accounts.localUsageEmpty")}
          />
        </div>

        <section className="overflow-hidden rounded-2xl border border-border/75 bg-card/75 shadow-xs">
          <div className="flex flex-col gap-2 border-b border-border/60 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-foreground/90">
              {t("accounts.relayUsage")}
            </h4>
            <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
              <span className="rounded-md bg-muted/70 px-2 py-1">
                {t("accounts.relayRequests", { count: relay.allTime.requestCount })}
              </span>
              <span className="rounded-md bg-muted/70 px-2 py-1">
                {t("accounts.relayReported", { count: relay.allTime.reportedCount })}
              </span>
            </div>
          </div>

          <div className="grid divide-y divide-border/60 sm:grid-cols-[1.35fr_1fr_1fr] sm:divide-x sm:divide-y-0">
            <div className="p-4 sm:p-5">
              <p className="text-[11px] font-medium text-muted-foreground">{t("accounts.allUsage")}</p>
              <div className="mt-1 flex items-baseline gap-1.5">
                <strong className="font-mono text-2xl font-semibold tracking-tight tabular-nums">
                  {formatTokens(relay.allTime.totalTokens)}
                </strong>
                <span className="text-[10px] text-muted-foreground">Token</span>
              </div>

              <CompositionBar
                className="mt-4"
                input={relay.allTime.inputTokens}
                output={relay.allTime.outputTokens}
                cache={0}
              />

            </div>
            <div className="flex flex-col justify-center p-4 sm:p-5">
              <p className="text-[11px] font-medium text-muted-foreground">{t("accounts.inputShort")}</p>
              <strong className="mt-1 font-mono text-xl font-semibold tracking-tight tabular-nums">{formatTokens(relay.allTime.inputTokens)}</strong>
              <span className="mt-1 text-[10px] text-muted-foreground">Token</span>
            </div>
            <div className="flex flex-col justify-center p-4 sm:p-5">
              <p className="text-[11px] font-medium text-muted-foreground">{t("accounts.outputShort")}</p>
              <strong className="mt-1 font-mono text-xl font-semibold tracking-tight tabular-nums">{formatTokens(relay.allTime.outputTokens)}</strong>
              <span className="mt-1 text-[10px] text-muted-foreground">Token</span>
            </div>
          </div>
          <div className="border-t border-border/60 p-4 sm:p-5">
            <UsageBreakdown
              title={t("accounts.relayModels")}
              items={relayModels}
              emptyLabel={t("accounts.relayEmpty")}
              mergeDuplicateModels
              embedded
            />
          </div>
        </section>

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
    <div className="flex min-w-0 flex-1 flex-col items-center justify-center p-1">
      <div className="relative h-[70px] w-[70px]">
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
          <span className={cn("font-mono text-sm font-bold tracking-tight", tone)}>
            {value == null ? "—" : `${value}%`}
          </span>
        </div>
      </div>
      <div className="mt-1.5 text-center">
        <p className="font-mono text-[11px] font-semibold tracking-tight text-foreground/90">
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
        "group relative flex flex-col justify-between overflow-hidden rounded-2xl border bg-card/90 backdrop-blur-md transition-all duration-300",
        account.isActive
          ? "border-emerald-500/40 ring-1 ring-emerald-500/20 shadow-[0_12px_32px_-12px_rgba(16,185,129,0.18)]"
          : "border-border/80 hover:border-primary/40 hover:shadow-lg hover:-translate-y-0.5",
      )}
    >
      {/* Top Header: Email Title + Status Badge */}
      <div className="px-4 pb-2.5 pt-3.5">
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
              className="truncate text-sm font-bold tracking-tight text-foreground/95"
              title={title}
            >
              {title}
            </h3>
          </div>
        </div>

        {/* Badges line: Plan Badge + Active Badge */}
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <Badge
            variant="outline"
            className={cn(
              "gap-1 rounded-full px-2 py-0.5 text-[11px] font-semibold tracking-wide",
              isPlus && "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
              isTeamOrPro && "border-primary/30 bg-primary/10 text-primary",
              !isPlus && !isTeamOrPro && "border-border bg-muted/60 text-muted-foreground",
            )}
          >
            <Sparkles className="h-3 w-3" />
            {planLabel(account.plan, t("accounts.planUnknown"))}
          </Badge>

          {account.isActive && (
            <Badge className="gap-1 rounded-full border border-emerald-500/25 bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-700 shadow-none dark:text-emerald-300">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              {t("accounts.current")}
            </Badge>
          )}
        </div>
      </div>

      {/* Middle: 2x2 Info Grid with clean icons */}
      <div className="grid grid-cols-2 gap-2.5 border-y border-border/50 bg-black/[0.015] px-4 py-2.5 text-xs dark:bg-white/[0.015]">
        {/* User Nickname */}
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
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
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
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
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
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
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
            <Calendar className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">最近同步</p>
            <p className="truncate font-semibold text-foreground/90" title={account.lastUsageAt ? new Date(account.lastUsageAt * 1000).toLocaleString() : undefined}>
              {account.lastUsageAt
                ? new Date(account.lastUsageAt * 1000).toLocaleString()
                : t("accounts.unknown")}
            </p>
          </div>
        </div>
      </div>

      {/* Quota Ring Dual Displays */}
      <div className="flex items-center justify-around divide-x divide-border/50 px-2 py-2.5">
        <QuotaRing label="5小时" window={account.primaryWindow} />
        <QuotaRing label="1周" window={account.secondaryWindow} />
      </div>

      {account.usageError && (
        <p className="mx-4 mb-3 rounded-xl bg-destructive/10 px-3 py-2 text-[11px] leading-relaxed text-destructive border border-destructive/20">
          {account.usageError}
        </p>
      )}

      {/* Footer Actions */}
      <div className="flex items-center justify-between border-t border-border/50 bg-black/[0.015] px-4 py-2.5 dark:bg-white/[0.015]">
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
              className="h-7 gap-1.5 rounded-lg border-border/80 px-2.5 text-[11px] font-semibold shadow-2xs transition-all hover:border-primary/40 hover:bg-primary/5 hover:text-primary"
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
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
  const tokenUsageQuery = useQuery({
    queryKey: TOKEN_USAGE_QUERY_KEY,
    queryFn: api.usage.overview,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
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
        <section className="flex flex-col gap-3 rounded-2xl border border-border/70 bg-card/70 px-4 py-3 shadow-sm sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="text-xl font-semibold tracking-tight">{t("accounts.title")}</h2>
              <Badge variant="outline" className="border-primary/25 bg-primary/5 text-[10px] text-primary">
                {t("accounts.official")}
              </Badge>
            </div>
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
              {loginMutation.isPending ? t("accounts.adding") : t("accounts.oauthLogin")}
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
            <Skeleton key={index} className="h-[330px] rounded-2xl" />
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
