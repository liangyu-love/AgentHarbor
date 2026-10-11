import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, LogIn, RefreshCw, Search, UsersRound } from "lucide-react";
import { PageHero } from "@/components/layout/page-hero";

import { AccountCard, planLabel } from "@/components/accounts/account-card";
import { AccountSourcePanel } from "@/components/accounts/account-source-panel";
import { useAccountSources } from "@/components/accounts/use-account-sources";
import { useAccounts } from "@/components/accounts/use-accounts";
import { AnimatedSegmentedControl } from "@/components/ui/animated-segmented-control";

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
import type { RefreshInterval } from "@/hooks/use-auto-refresh";
import { cn } from "@/lib/utils";
import { formatInvokeError } from "@/lib/invoke-error";
import type { AccountSourceKind } from "@/types/account-sources";

const TABS = ["codex", "chatgpt-api", "workbuddy", "workbuddy-ai"] as const;
type Tab = (typeof TABS)[number];
const TAB_KEY = "agentharbor.accounts-tab.v1";

function readTab(): Tab {
  try {
    const saved = localStorage.getItem(TAB_KEY);
    return (TABS as readonly string[]).includes(saved ?? "") ? saved as Tab : "codex";
  } catch {
    return "codex";
  }
}

export function AccountsPage({ refreshInterval }: { refreshInterval: RefreshInterval }) {
  const { t } = useTranslation();
  const {
    accountsQuery, accounts, plans, filtered, search, setSearch, plan, setPlan,
    deleteTarget, setDeleteTarget, startLogin, cancelLogin, loginPending, loginCancelling,
    switchMutation, deleteMutation, refreshQuotaMutation, refresh,
  } = useAccounts(refreshInterval);
  const sources = useAccountSources(refreshInterval);
  const [tab, setTabState] = useState<Tab>(readTab);
  // The panel enters from the side of the newly chosen source, so switching
  // reads as moving along the row of sources rather than a page swap.
  const [direction, setDirection] = useState<"forward" | "back" | null>(null);
  const setTab = (next: Tab) => {
    if (next !== tab) setDirection(TABS.indexOf(next) > TABS.indexOf(tab) ? "forward" : "back");
    setTabState(next);
    try { localStorage.setItem(TAB_KEY, next); } catch { /* storage unavailable: the tab is not remembered */ }
  };
  const sourceTab = tab === "codex" ? null : tab as AccountSourceKind;
  const sourceName = (kind: AccountSourceKind) => sources.sources.find(source => source.kind === kind)?.name ?? kind;
  const countOf = (value: Tab) => value === "codex" ? accounts.length
    : sources.sources.find(source => source.kind === value)?.accounts.length ?? 0;
  const tabItems = TABS.map(value => ({
    value,
    label: <span className="flex items-center gap-1.5">
      {value === "codex" ? "Codex" : value === "chatgpt-api" ? "ChatGPT" : value === "workbuddy" ? "WorkBuddy" : "WorkBuddy AI"}
      <span className="rounded-full bg-current/10 px-1.5 font-mono text-[10px] leading-4 tabular-nums opacity-70">{countOf(value)}</span>
    </span>,
  }));
  const fetching = sourceTab ? sources.query.isFetching : accountsQuery.isFetching;

  return (
    <div className="mx-auto max-w-7xl space-y-5 pb-8">
        <PageHero kicker="Accounts" icon={<UsersRound />} titleAs="h2" title={t("accounts.title")}
          description={t("accounts.heroDesc")}
          badge={<Badge variant="outline" className="border-primary/25 bg-primary/5 text-[10px] text-primary">{t("accounts.official")}</Badge>}
          actions={<>
            <Button
              variant="outline"
              onClick={() => void (sourceTab ? sources.query.refetch() : refresh())}
              disabled={fetching}
            >
              <RefreshCw className={cn(fetching && "animate-spin")} />
              {t("common.refresh")}
            </Button>
            {sourceTab
              ? sources.pending === null && <Button onClick={() => sources.signIn(sourceTab)}><LogIn />{t("accountSources.signIn", { name: sourceName(sourceTab) })}</Button>
              : !loginPending && <Button onClick={startLogin}><LogIn />{t("accounts.oauthLogin")}</Button>}
          </>} />

        <nav aria-label={t("accountSources.tabs")} data-accounts-tabs className="max-w-full overflow-x-auto">
          <div className="inline-flex rounded-full bg-muted p-0.5 dark:bg-white/[0.06]">
            <AnimatedSegmentedControl items={tabItems} value={tab} onValueChange={value => setTab(value as Tab)}
              className="gap-0.5"
              indicatorClassName="rounded-full bg-primary shadow-sm"
              itemClassName="rounded-full px-4 py-[5px] text-[13px] font-medium whitespace-nowrap"
              activeItemClassName="text-primary-foreground"
              inactiveItemClassName="text-muted-foreground hover:text-foreground" />
          </div>
        </nav>

        <div key={tab} data-accounts-panel={tab} data-direction={direction ?? undefined}
          className={cn("space-y-5", direction && "accounts-panel-enter")}>
        {sourceTab ? <AccountSourcePanel kind={sourceTab} sources={sources} /> : <>
        {loginPending && (
          <section aria-label={t("accounts.oauthLogin")} className="flex flex-col gap-3 rounded-xl border border-border bg-card px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
            <div role="status" aria-live="polite" className="flex min-w-0 items-start gap-3">
              <Loader2 aria-hidden="true" className="mt-0.5 h-4 w-4 shrink-0 animate-spin text-primary motion-reduce:animate-none" />
              <div className="min-w-0 space-y-1">
                <p className="text-sm font-medium">{t(loginCancelling ? "accounts.cancellingLogin" : "accounts.adding")}</p>
                <p className="text-sm text-muted-foreground">{t(loginCancelling ? "accounts.cancellingHint" : "accounts.loginHint")}</p>
              </div>
            </div>
            <Button variant="outline" onClick={cancelLogin} disabled={loginCancelling} className="w-full shrink-0 sm:w-auto">
              {t(loginCancelling ? "accounts.cancellingLogin" : "accounts.cancelLogin")}
            </Button>
          </section>
        )}

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
        </section>

        {accountsQuery.isPending ? (
          <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
            {Array.from({ length: 3 }).map((_, index) => (
            <Skeleton key={index} className="h-[330px] rounded-2xl" />
            ))}
          </div>
        ) : accountsQuery.isError ? (
          <div role="alert" className="rounded-2xl border border-destructive/30 bg-destructive/5 p-5 text-sm text-destructive">
            <strong>{t("accounts.loadFailed")}</strong>
            <details className="mt-2 text-xs text-muted-foreground">
              <summary className="w-fit cursor-pointer">{t("accounts.technicalDetails")}</summary>
              <p className="mt-2 break-all font-mono">{formatInvokeError(accountsQuery.error)}</p>
            </details>
          </div>
        ) : accounts.length === 0 ? (
          <div className="flex min-h-80 flex-col items-center justify-center rounded-2xl border border-dashed border-border bg-card/50 px-6 text-center">
            <div className="mb-4 rounded-2xl bg-primary/10 p-4 text-primary">
              <UsersRound className="h-7 w-7" />
            </div>
            <h3 className="font-semibold">{t("accounts.emptyTitle")}</h3>
            <p className="mt-1 max-w-md text-sm text-muted-foreground">{t("accounts.emptyDescription")}</p>
            <Button className="mt-5" onClick={startLogin} disabled={loginPending}>
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

        </>}
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
