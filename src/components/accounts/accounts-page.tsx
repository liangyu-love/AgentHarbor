import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  Activity,
  BarChart3,
  Check,
  Clock3,
  Database,
  Loader2,
  LogIn,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
  UserRound,
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
import { formatRelative, formatResetLabel } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import type {
  AccountRateLimitWindow,
  CodexAccountSummary,
  RelayUsagePayload,
  RelayUsageTotals,
} from "@/types";

const QUERY_KEY = ["codex-accounts"] as const;
const RELAY_USAGE_QUERY_KEY = ["relay-usage"] as const;

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

function UsageTotalCard({ label, totals }: { label: string; totals: RelayUsageTotals }) {
  const { t } = useTranslation();
  return (
    <div className="rounded-xl border border-border/70 bg-background/65 p-4">
      <p className="text-xs font-medium text-muted-foreground">{label}</p>
      <p className="mt-1 text-2xl font-semibold tracking-tight">{formatTokens(totals.totalTokens)}</p>
      <div className="mt-3 flex items-center justify-between gap-3 text-[11px] text-muted-foreground">
        <span>{t("accounts.relayRequests", { count: totals.requestCount })}</span>
        <span>
          {t("accounts.relayReported", {
            count: totals.reportedCount,
          })}
        </span>
      </div>
    </div>
  );
}

function UsageBreakdown({
  title,
  items,
}: {
  title: string;
  items: Array<{ key: string; title: string; subtitle?: string; totals: RelayUsageTotals }>;
}) {
  const { t } = useTranslation();
  const max = Math.max(1, ...items.map((item) => item.totals.totalTokens));
  return (
    <div className="rounded-xl border border-border/70 bg-background/55 p-4">
      <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">{title}</h4>
      {items.length === 0 ? (
        <p className="py-8 text-center text-xs text-muted-foreground">{t("accounts.relayEmpty")}</p>
      ) : (
        <div className="mt-3 space-y-3">
          {items.slice(0, 5).map((item) => (
            <div key={item.key}>
              <div className="flex items-center justify-between gap-3 text-xs">
                <div className="min-w-0">
                  <p className="truncate font-medium" title={item.title}>{item.title}</p>
                  {item.subtitle && <p className="truncate text-[10px] text-muted-foreground">{item.subtitle}</p>}
                </div>
                <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                  {formatTokens(item.totals.totalTokens)}
                </span>
              </div>
              <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full rounded-full bg-primary/70"
                  style={{ width: `${Math.max(3, (item.totals.totalTokens / max) * 100)}%` }}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function RelayUsagePanel({ data, loading, error }: {
  data: RelayUsagePayload | undefined;
  loading: boolean;
  error: unknown;
}) {
  const { t } = useTranslation();
  if (loading) return <Skeleton className="h-[430px] rounded-2xl" />;
  if (error) {
    return (
      <section className="rounded-2xl border border-destructive/30 bg-destructive/5 p-5 text-sm text-destructive">
        {String(error)}
      </section>
    );
  }
  if (!data) return null;
  const maxDaily = Math.max(1, ...data.daily.map((day) => day.totals.totalTokens));
  const providerItems = data.byProvider.map((item) => ({
    key: item.providerId,
    title: item.providerName,
    totals: item.totals,
  }));
  const modelItems = data.byModel.map((item) => ({
    key: `${item.providerId}::${item.model}`,
    title: item.model,
    subtitle: item.providerName,
    totals: item.totals,
  }));

  return (
    <section className="overflow-hidden rounded-2xl border border-border/75 bg-card/75 shadow-sm">
      <div className="flex flex-col gap-3 border-b border-border/70 px-5 py-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-start gap-3">
          <div className="rounded-xl bg-primary/10 p-2.5 text-primary"><Activity className="h-5 w-5" /></div>
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="font-semibold">{t("accounts.relayMonitor")}</h3>
              <Badge variant="outline" className="border-primary/20 bg-primary/5 text-[10px] text-primary">
                {t("accounts.relayOnly")}
              </Badge>
            </div>
            <p className="mt-1 text-xs text-muted-foreground">{t("accounts.relayMonitorDesc")}</p>
          </div>
        </div>
        <div className="flex items-center gap-4 text-[11px] text-muted-foreground">
          <span className="flex items-center gap-1.5"><Database className="h-3.5 w-3.5" />{t("accounts.localHistory")}</span>
          <span>{t("accounts.inputShort")} {formatTokens(data.allTime.inputTokens)}</span>
          <span>{t("accounts.outputShort")} {formatTokens(data.allTime.outputTokens)}</span>
          <span>{t("accounts.reasoningShort")} {formatTokens(data.allTime.reasoningTokens)}</span>
        </div>
      </div>

      <div className="space-y-4 p-4 sm:p-5">
        <div className="grid gap-3 sm:grid-cols-3">
          <UsageTotalCard label={t("accounts.todayUsage")} totals={data.today} />
          <UsageTotalCard label={t("accounts.weekUsage")} totals={data.week} />
          <UsageTotalCard label={t("accounts.allUsage")} totals={data.allTime} />
        </div>

        <div className="grid gap-4 lg:grid-cols-[1.25fr_1fr_1fr]">
          <div className="rounded-xl border border-border/70 bg-background/55 p-4">
            <div className="flex items-center justify-between gap-3">
              <h4 className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                <BarChart3 className="h-3.5 w-3.5" />{t("accounts.dailyTrend")}
              </h4>
              <span className="text-[10px] text-muted-foreground">{t("accounts.last14Days")}</span>
            </div>
            <div className="mt-4 flex h-36 items-end gap-1.5">
              {data.daily.map((day) => (
                <div key={day.date} className="group flex min-w-0 flex-1 flex-col items-center justify-end gap-1.5">
                  <div className="relative flex h-28 w-full items-end">
                    <div
                      className="w-full min-w-1 rounded-t bg-primary/65 transition-colors group-hover:bg-primary"
                      style={{ height: `${day.totals.totalTokens === 0 ? 2 : Math.max(6, day.totals.totalTokens / maxDaily * 100)}%` }}
                      title={`${day.date} · ${formatTokens(day.totals.totalTokens)} Token · ${day.totals.requestCount} requests`}
                    />
                  </div>
                  <span className="text-[9px] text-muted-foreground">
                    {Number(day.date.slice(-2)) % 2 === 0 ? day.date.slice(5) : ""}
                  </span>
                </div>
              ))}
            </div>
          </div>
          <UsageBreakdown title={t("accounts.byProvider")} items={providerItems} />
          <UsageBreakdown title={t("accounts.byModel")} items={modelItems} />
        </div>

        {data.recent.length > 0 && (
          <div className="overflow-hidden rounded-xl border border-border/70 bg-background/55">
            <div className="border-b border-border/70 px-4 py-3 text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
              {t("accounts.recentRelayRequests")}
            </div>
            <div className="divide-y divide-border/60">
              {data.recent.slice(0, 5).map((item, index) => (
                <div key={`${item.timestamp}-${item.providerId}-${index}`} className="grid grid-cols-[minmax(0,1fr)_auto] gap-4 px-4 py-3 text-xs sm:grid-cols-[160px_minmax(0,1fr)_90px_100px]">
                  <div className="truncate font-medium">{item.providerName}</div>
                  <div className="truncate font-mono text-[11px] text-muted-foreground" title={item.model}>{item.model}</div>
                  <div className={cn("hidden sm:block", item.success ? "text-emerald-600 dark:text-emerald-400" : "text-destructive")}>
                    HTTP {item.statusCode}
                  </div>
                  <div className="text-right font-mono text-muted-foreground">
                    {item.usageReported ? formatTokens(item.totalTokens) : t("accounts.unreported")}
                  </div>
                </div>
              ))}
            </div>
          </div>
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
  const radius = 31;
  const circumference = Math.PI * 2 * radius;
  const offset = value == null ? circumference : circumference * (1 - value / 100);
  const tone = value == null
    ? "stroke-muted-foreground/15"
    : value <= 15
      ? "stroke-destructive"
      : value <= 35
        ? "stroke-amber-500"
        : "stroke-emerald-500";

  return (
    <div className="flex min-w-0 flex-1 flex-col items-center gap-2">
      <div className="relative h-[78px] w-[78px]">
        <svg viewBox="0 0 72 72" className="h-full w-full -rotate-90" aria-hidden>
          <circle cx="36" cy="36" r={radius} fill="none" strokeWidth="6" className="stroke-muted/80" />
          <circle
            cx="36"
            cy="36"
            r={radius}
            fill="none"
            strokeWidth="6"
            strokeLinecap="round"
            strokeDasharray={circumference}
            strokeDashoffset={offset}
            className={cn("transition-all duration-500", tone)}
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center text-sm font-semibold">
          {value == null ? "—" : `${value}%`}
        </div>
      </div>
      <div className="text-center">
        <p className="text-xs font-semibold text-foreground">{label}</p>
        <p className="mt-0.5 min-h-8 text-[10px] leading-4 text-muted-foreground">
          {window?.resetsAt ? formatResetLabel(window.resetsAt) : "—"}
        </p>
      </div>
    </div>
  );
}

function AccountCard({
  account,
  switching,
  onSwitch,
  onDelete,
}: {
  account: CodexAccountSummary;
  switching: boolean;
  onSwitch: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const statusText = t(`accounts.${account.usageStatus}`);
  const title = account.email || account.displayName;

  return (
    <article
      className={cn(
        "overflow-hidden rounded-2xl border bg-card/92 shadow-sm transition-all",
        account.isActive
          ? "border-emerald-500/40 shadow-[0_16px_36px_rgba(16,185,129,0.10)]"
          : "border-border/80 hover:border-primary/30 hover:shadow-md",
      )}
    >
      <div className="border-b border-border/70 px-5 py-4">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className={cn("h-2.5 w-2.5 shrink-0 rounded-full", statusStyle(account.usageStatus))} />
              <h3 className="truncate text-[15px] font-semibold" title={title}>{title}</h3>
            </div>
            {account.displayName !== title && (
              <p className="mt-1 truncate pl-[18px] text-xs text-muted-foreground">{account.displayName}</p>
            )}
          </div>
          <div className="flex shrink-0 flex-wrap justify-end gap-1.5">
            <Badge variant="secondary" className="border border-border/60 bg-muted/70 text-[10px]">
              {planLabel(account.plan, t("accounts.planUnknown"))}
            </Badge>
            {account.isActive && (
              <Badge className="border border-emerald-500/20 bg-emerald-500/10 text-[10px] text-emerald-700 shadow-none hover:bg-emerald-500/10 dark:text-emerald-300">
                <Check className="mr-1 h-3 w-3" />
                {t("accounts.current")}
              </Badge>
            )}
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4 border-b border-border/70 px-5 py-4 text-xs">
        <div className="flex min-w-0 items-start gap-2">
          <UserRound className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <p className="text-[10px] text-muted-foreground">{t("accounts.authMode")}</p>
            <p className="truncate font-medium">{t("accounts.chatgptOauth")}</p>
          </div>
        </div>
        <div className="flex min-w-0 items-start gap-2">
          <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0">
            <p className="text-[10px] text-muted-foreground">{statusText}</p>
            <p className="truncate font-medium" title={account.usageError ?? undefined}>
              {account.lastUsageAt
                ? t("accounts.lastChecked", { time: formatRelative(account.lastUsageAt) })
                : t("accounts.unknown")}
            </p>
          </div>
        </div>
      </div>

      <div className="flex items-start divide-x divide-border/70 px-4 py-5">
        <QuotaRing label={t("accounts.fiveHour")} window={account.primaryWindow} />
        <QuotaRing label={t("accounts.weekly")} window={account.secondaryWindow} />
      </div>

      {account.usageError && (
        <p className="mx-4 mb-3 rounded-lg bg-destructive/8 px-3 py-2 text-[11px] leading-4 text-destructive">
          {account.usageError}
        </p>
      )}

      <div className="flex items-center justify-between border-t border-border/70 px-4 py-3">
        <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
          <Clock3 className="h-3.5 w-3.5" />
          {account.lastUsedAt ? formatRelative(account.lastUsedAt) : "—"}
        </div>
        <div className="flex items-center gap-1.5">
          {!account.isActive && (
            <Button variant="soft" size="xs" onClick={onSwitch} disabled={switching}>
              {switching ? <Loader2 className="animate-spin" /> : <LogIn />}
              {switching ? t("accounts.switching") : t("accounts.setCurrent")}
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 text-muted-foreground hover:text-destructive"
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
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [plan, setPlan] = useState("all");
  const [deleteTarget, setDeleteTarget] = useState<CodexAccountSummary | null>(null);

  const accountsQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: api.accounts.refresh,
    refetchInterval: REFRESH_INTERVAL_MS[refreshInterval],
    refetchIntervalInBackground: false,
    retry: false,
  });
  const relayUsageQuery = useQuery({
    queryKey: RELAY_USAGE_QUERY_KEY,
    queryFn: api.usage.relay,
    refetchInterval: REFRESH_INTERVAL_MS[refreshInterval],
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

  const refresh = async () => {
    const [accountsResult] = await Promise.all([accountsQuery.refetch(), relayUsageQuery.refetch()]);
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
    <div className="h-full overflow-y-auto p-4 sm:p-6">
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
              disabled={accountsQuery.isFetching}
            >
              <RefreshCw className={cn(accountsQuery.isFetching && "animate-spin")} />
              {t("common.refresh")}
            </Button>
            <Button onClick={() => loginMutation.mutate()} disabled={loginMutation.isPending}>
              {loginMutation.isPending ? <Loader2 className="animate-spin" /> : <LogIn />}
              {loginMutation.isPending ? t("accounts.adding") : t("accounts.add")}
            </Button>
          </div>
        </section>

        <RelayUsagePanel
          data={relayUsageQuery.data}
          loading={relayUsageQuery.isPending}
          error={relayUsageQuery.error}
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
                onSwitch={() => switchMutation.mutate(account.accountKey)}
                onDelete={() => setDeleteTarget(account)}
              />
            ))}
          </div>
        )}
      </div>

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
