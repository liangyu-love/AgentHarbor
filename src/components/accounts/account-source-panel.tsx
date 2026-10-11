import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Check, ExternalLink, Globe, KeyRound, Loader2, LogIn, Plug, RefreshCw, Sparkles, Trash2, UsersRound, Waypoints } from "lucide-react";

import { maskEmail } from "@/components/accounts/account-card";
import type { useAccountSources } from "@/components/accounts/use-account-sources";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { SignalPath, linkPath, useAnchors } from "@/components/ui/signal-lines";
import { formatInvokeError } from "@/lib/invoke-error";
import { reducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";
import type { AccountSource, AccountSourceAccount, AccountSourceKind } from "@/types/account-sources";

type Sources = ReturnType<typeof useAccountSources>;

function formatCount(value: number) {
  return new Intl.NumberFormat(undefined, { notation: value >= 10_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(value);
}

function CreditsMeter({ account }: { account: AccountSourceAccount }) {
  const { t } = useTranslation();
  const quota = account.quota;
  if (!quota) return <p className="text-xs text-muted-foreground">{t("accountSources.quotaUnread")}</p>;
  if (quota.error) return <p className="text-xs text-destructive">{quota.error}</p>;
  if (quota.limit <= 0) return <p className="text-xs text-muted-foreground">{t("accountSources.quotaNone")}</p>;
  const share = Math.min(100, Math.max(0, (quota.used / quota.limit) * 100));
  const left = 100 - share;
  return (
    <div className="space-y-1.5" data-source-quota>
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="text-muted-foreground">{t("accountSources.credits")}</span>
        <span className="font-mono tabular-nums text-foreground/90">{formatCount(quota.used)} / {formatCount(quota.limit)}</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-muted" role="meter" aria-valuemin={0} aria-valuemax={100}
        aria-valuenow={Math.round(left)} aria-label={t("accountSources.creditsLeft", { value: Math.round(left) })}>
        {/* Grows from empty to what this cycle has used; the numbers above are exact at once. */}
        <div className={cn("source-credits-fill h-full rounded-full", left <= 15 ? "bg-rose-500" : left <= 35 ? "bg-amber-500" : "bg-emerald-500")}
          style={{ "--fill": share / 100 } as CSSProperties} />
      </div>
    </div>
  );
}

/**
 * Where a source's models go: its account → the AgentHarbor router → model
 * inject. Shared and signed in, the lines draw in and carry traffic like the
 * routing topology; not shared, they fall back to dashed hairlines.
 */
function SourceChannel({ source }: { source: AccountSource }) {
  const { t } = useTranslation();
  const board = useRef<HTMLDivElement>(null);
  const { anchors, register } = useAnchors(board);
  const usable = source.accounts.some(account => account.active && !account.lapsed);
  const hubLive = source.exposed && usable;
  const state = `${source.exposed}:${source.accounts.find(account => account.active)?.id ?? ""}`;
  const [ignition, setIgnition] = useState(0);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    setIgnition(value => value + 1);
  }, [state]);
  const node = (id: string, icon: ReactNode, label: string, live: boolean) => (
    <div className="flex min-w-0 flex-col items-center gap-1.5">
      <span ref={register(id)} data-channel-node={id} data-live={live || undefined}
        className={cn("relative flex h-10 w-10 items-center justify-center rounded-2xl border bg-card shadow-sm transition-[border-color,opacity] duration-300",
          live ? "border-primary/50 text-primary" : "border-border text-muted-foreground opacity-80")}>
        {icon}
      </span>
      <span className="max-w-[7rem] truncate text-[11px] text-muted-foreground">{label}</span>
    </div>
  );
  return (
    <div ref={board} data-source-channel={hubLive ? "live" : source.exposed ? "waiting" : "idle"}
      className="relative flex w-full max-w-[24rem] items-start justify-between gap-3">
      <svg aria-hidden className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
        <SignalPath d={linkPath(anchors, "source", "hub")} live={hubLive} ignition={ignition} />
        <SignalPath d={linkPath(anchors, "hub", "inject")} live={source.exposed} ignition={ignition} speed={1.2} />
      </svg>
      {node("source", <KeyRound className="h-4 w-4" />, source.name, usable)}
      <div className="flex min-w-0 flex-col items-center gap-1.5">
        <span ref={register("hub")} className="relative flex h-10 w-10 items-center justify-center">
          <span aria-hidden className={cn("absolute -inset-1 rounded-full", hubLive ? "route-hub-ring" : "border border-dashed border-border")} />
          <span key={ignition} className={cn("relative flex h-9 w-9 items-center justify-center rounded-full border bg-card shadow-sm",
            hubLive ? "border-primary/30 text-primary" : "border-border text-muted-foreground", ignition > 0 && "route-hub-ignite")}>
            <Waypoints className="h-4 w-4" />
          </span>
        </span>
        <span className="text-[11px] text-muted-foreground">AgentHarbor</span>
      </div>
      {node("inject", <Plug className="h-4 w-4" />, t("newapi.title"), source.exposed)}
    </div>
  );
}

/** Waiting for the browser: a packet shuttles between AgentHarbor and the sign-in page. */
function SignInHandshake() {
  const reduced = reducedMotion();
  const path = "M4,10 H84";
  return (
    <span aria-hidden className="flex shrink-0 items-center gap-1.5" data-sign-in-handshake>
      <span className="flex h-7 w-7 items-center justify-center rounded-xl border border-primary/40 bg-card text-primary"><Waypoints className="h-3.5 w-3.5" /></span>
      <svg viewBox="0 0 88 20" className="h-5 w-[5.5rem] overflow-visible">
        <path d={path} fill="none" className="stroke-border" strokeWidth={1.25} strokeDasharray="3 5" />
        <path d={path} fill="none" stroke="hsl(var(--primary))" strokeOpacity={0.55} strokeWidth={1.25} strokeDasharray="3 5" className="route-flow-slow" />
        {!reduced && <circle r={2.6} fill="hsl(var(--primary))">
          <animateMotion dur="2.2s" repeatCount="indefinite" path={path} keyPoints="0;1;0" keyTimes="0;0.5;1" calcMode="spline"
            keySplines="0.45 0 0.55 1;0.45 0 0.55 1" />
        </circle>}
      </svg>
      <span className="flex h-7 w-7 items-center justify-center rounded-xl border border-border bg-card text-muted-foreground"><Globe className="h-3.5 w-3.5" /></span>
    </span>
  );
}

function SourceAccountCard({ source, account, sources, order, onRemove }: {
  source: AccountSource;
  account: AccountSourceAccount;
  sources: Sources;
  order: number;
  onRemove: () => void;
}) {
  const { t } = useTranslation();
  const [revealed, setRevealed] = useState(false);
  const isEmail = account.name.includes("@");
  const title = isEmail && !revealed ? maskEmail(account.name) : account.name;
  const activating = sources.activate.isPending && sources.activate.variables === account.id;
  const refreshing = sources.refreshQuota.isPending && sources.refreshQuota.variables === account.id;
  const workbuddy = source.kind !== "chatgpt-api";
  return (
    <article data-source-account={account.id} data-active={account.active || undefined}
      style={{ animationDelay: `${Math.min(order, 4) * 50}ms` }}
      className={cn("source-account-card route-rise flex flex-col overflow-hidden rounded-2xl border bg-card/90 backdrop-blur-md transition-colors",
        account.active ? "border-emerald-500/40 ring-1 ring-emerald-500/20" : "border-border/80 hover:border-primary/40")}>
      <div className="space-y-2 px-4 pb-3 pt-3.5">
        <div className="flex items-center gap-2">
          <span className={cn("h-2.5 w-2.5 shrink-0 rounded-full", account.lapsed ? "bg-amber-500" : "bg-emerald-500")} />
          <h4 className="min-w-0 flex-1 truncate text-sm font-bold tracking-tight" title={isEmail && !revealed ? undefined : account.name}>{title}</h4>
          {isEmail && (
            <Button type="button" variant="ghost" size="xs" className="h-6 px-2 text-[11px] text-muted-foreground"
              aria-pressed={revealed} onClick={() => setRevealed(value => !value)}>
              {t(revealed ? "accounts.hideEmail" : "accounts.showEmail")}
            </Button>
          )}
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          {account.plan && (
            <Badge variant="outline" className="gap-1 rounded-full px-2 py-0.5 text-[11px] font-semibold">
              <Sparkles className="h-3 w-3" />{account.plan}
            </Badge>
          )}
          {account.active && (
            <Badge className="gap-1 rounded-full border border-emerald-500/25 bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-700 shadow-none dark:text-emerald-300">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />{t("accounts.current")}
            </Badge>
          )}
        </div>
      </div>
      <div className="flex-1 border-y border-border/50 bg-black/[0.015] px-4 py-3 dark:bg-white/[0.015]">
        {account.lapsed ? (
          <p role="status" className="flex items-start gap-2 text-xs text-amber-700 dark:text-amber-300">
            <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />{t("accountSources.lapsed")}
          </p>
        ) : workbuddy ? <CreditsMeter account={account} /> : (
          <p className="text-xs text-muted-foreground">
            {t("accountSources.chatgptUsage")}{" "}
            {source.usageUrl && (
              <a className="inline-flex items-center gap-0.5 font-medium text-primary hover:underline" href={source.usageUrl} target="_blank" rel="noreferrer">
                chatgpt.com<ExternalLink className="h-3 w-3" />
              </a>
            )}
          </p>
        )}
      </div>
      <div className="flex items-center justify-between gap-2 px-4 py-2.5">
        {account.active ? (
          <span className="flex items-center gap-1.5 text-xs font-semibold text-emerald-600 dark:text-emerald-400"><Check className="h-4 w-4" />{t("accounts.current")}</span>
        ) : (
          <Button variant="outline" size="xs" className="h-7 gap-1.5 rounded-lg px-2.5 text-[11px] font-semibold"
            disabled={activating} onClick={() => sources.activate.mutate(account.id)}>
            {activating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <LogIn className="h-3.5 w-3.5" />}
            {t("accounts.setCurrent")}
          </Button>
        )}
        <div className="flex items-center gap-1">
          {workbuddy && (
            <Button variant="ghost" size="icon-sm" className="h-7 w-7 text-muted-foreground" disabled={refreshing}
              aria-label={t("accountSources.refreshQuota")} title={t("accountSources.refreshQuota")}
              onClick={() => sources.refreshQuota.mutate(account.id)}>
              <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin text-primary")} />
            </Button>
          )}
          <Button variant="ghost" size="icon-sm" className="h-7 w-7 text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
            aria-label={t("accounts.remove")} title={t("accounts.remove")} onClick={onRemove}>
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </article>
  );
}

/** One account source: its share with model inject, sign-in, accounts and models. */
export function AccountSourcePanel({ kind, sources }: { kind: AccountSourceKind; sources: Sources }) {
  const { t } = useTranslation();
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const source = sources.sources.find(item => item.kind === kind);
  const titleId = `account-source-${kind}-title`;
  const signingIn = sources.pending?.kind === kind;
  if (sources.query.isPending) {
    return <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">{[0, 1].map(index => <Skeleton key={index} className="h-52 rounded-2xl" />)}</div>;
  }
  if (sources.query.isError || !source) {
    return (
      <div role="alert" className="rounded-2xl border border-destructive/30 bg-destructive/5 p-5 text-sm text-destructive">
        <strong>{t("accountSources.loadFailed")}</strong>
        <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{formatInvokeError(sources.query.error)}</p>
      </div>
    );
  }
  const exposing = sources.setExposed.isPending && sources.setExposed.variables?.kind === kind;
  const modelsRefreshing = sources.refreshModels.isPending && sources.refreshModels.variables === kind;
  const includedModels = source.models.length;
  return (
    <section aria-labelledby={titleId} data-account-source={kind} className="space-y-4">
      <div className="harbor-well grid gap-5 p-4 md:grid-cols-[minmax(0,1fr)_minmax(16rem,24rem)] md:items-center">
        <div className="min-w-0 space-y-1.5">
          <h3 id={titleId} className="text-sm font-semibold">{source.name}</h3>
          <p className="max-w-2xl text-sm text-muted-foreground">{t(`accountSources.desc.${kind}`)}</p>
          <p className="max-w-2xl text-xs text-muted-foreground/90" data-source-note>{t(`accountSources.note.${kind}`)}</p>
        </div>
        <div className="flex min-w-0 flex-col items-stretch gap-3 md:items-end">
          <SourceChannel source={source} />
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2 md:justify-end">
            <label className="flex items-center gap-2 text-sm font-medium">
              <Switch checked={source.exposed} disabled={exposing || (!source.exposed && includedModels === 0)}
                aria-describedby={`${titleId}-share`}
                onCheckedChange={exposed => sources.setExposed.mutate({ kind, exposed })} />
              {t("accountSources.expose")}
            </label>
            {kind === "chatgpt-api" && (
              <Button variant="outline" size="xs" className="h-7 gap-1.5" disabled={modelsRefreshing || source.accounts.length === 0}
                onClick={() => sources.refreshModels.mutate(kind)}>
                <RefreshCw className={cn("h-3.5 w-3.5", modelsRefreshing && "animate-spin")} />{t("accountSources.refreshModels")}
              </Button>
            )}
          </div>
          <p id={`${titleId}-share`} className="text-xs text-muted-foreground md:text-right">
            {source.exposed
              ? t("accountSources.exposedModels", { count: includedModels })
              : includedModels === 0 ? t("accountSources.noModelsYet") : t("accountSources.notExposed")}
          </p>
        </div>
      </div>

      {signingIn && (
        <section aria-label={t("accountSources.signIn", { name: source.name })}
          className="flex flex-col gap-3 rounded-xl border border-border bg-card px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
          <div role="status" aria-live="polite" className="flex min-w-0 items-center gap-4">
            <SignInHandshake />
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-medium">{t(sources.cancelling ? "accounts.cancellingLogin" : "accounts.adding")}</p>
              <p className="text-sm text-muted-foreground">{t(sources.cancelling ? "accounts.cancellingHint" : "accountSources.signInHint")}</p>
            </div>
          </div>
          <Button variant="outline" onClick={sources.cancelSignIn} disabled={sources.cancelling} className="w-full shrink-0 sm:w-auto">
            {t(sources.cancelling ? "accounts.cancellingLogin" : "accounts.cancelLogin")}
          </Button>
        </section>
      )}

      {source.accounts.length === 0 ? (
        <div className="flex min-h-56 flex-col items-center justify-center rounded-2xl border border-dashed border-border bg-card/50 px-6 text-center">
          <div className="mb-4 rounded-2xl bg-primary/10 p-4 text-primary"><UsersRound className="h-7 w-7" /></div>
          <h4 className="font-semibold">{t("accountSources.emptyTitle", { name: source.name })}</h4>
          <p className="mt-1 max-w-md text-sm text-muted-foreground">{t("accountSources.emptyDescription")}</p>
          <Button className="mt-5" onClick={() => sources.signIn(kind)} disabled={sources.pending !== null}>
            <LogIn />{t("accountSources.signIn", { name: source.name })}
          </Button>
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
          {source.accounts.map((account, index) => (
            <SourceAccountCard key={account.id} source={source} account={account} sources={sources} order={index}
              onRemove={() => setConfirm({
                title: t("accountSources.removeTitle"),
                description: t("accountSources.removeDescription", { name: account.name }),
                confirmLabel: t("common.delete"),
                destructive: true,
                onConfirm: () => { setConfirm(null); sources.remove.mutate(account.id); },
              })} />
          ))}
        </div>
      )}

      {source.models.length > 0 && (
        <details className="group rounded-xl border border-border/60 bg-card/60 px-4 py-3" data-source-models>
          <summary className="cursor-pointer text-sm font-medium">
            {t("accountSources.modelList", { count: source.models.length })}
          </summary>
          <ul className="mt-3 flex flex-wrap gap-1.5">
            {source.models.map(model => (
              <li key={model.id} className="rounded-full border border-border/70 bg-muted/40 px-2.5 py-0.5 font-mono text-[11px]" title={model.id}>
                {model.name || model.id}
              </li>
            ))}
          </ul>
        </details>
      )}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </section>
  );
}
