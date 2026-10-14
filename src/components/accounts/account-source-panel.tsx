import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Check, Copy, ExternalLink, Globe, KeyRound, Loader2, LogIn, Plug, RefreshCw, Sparkles, Trash2, UsersRound, Waypoints } from "lucide-react";

import { maskEmail } from "@/components/accounts/account-card";
import type { useAccountSources } from "@/components/accounts/use-account-sources";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { SignalPath, linkPath, useAnchors } from "@/components/ui/signal-lines";
import { useFold } from "@/hooks/use-fold";
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
  if (quota.error) return <p className="text-xs text-destructive">{formatInvokeError(quota.error, quota.error)}</p>;
  if (quota.limit <= 0) return <p className="text-xs text-muted-foreground">{t("accountSources.quotaNone")}</p>;
  const share = Math.min(100, Math.max(0, (quota.used / quota.limit) * 100));
  const left = 100 - share;
  const label = quota.unit === "premium" ? "accountSources.premiumRequests" : quota.unit === "percent" ? "accountSources.tightestModel" : "accountSources.credits";
  return (
    <div className="space-y-1.5" data-source-quota>
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span className="text-muted-foreground">{t(label)}</span>
        <span className="font-mono tabular-nums text-foreground/90">
          {quota.unit === "percent" ? t("accountSources.usedPercent", { value: Math.round(share) }) : `${formatCount(quota.used)} / ${formatCount(quota.limit)}`}
        </span>
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
  const usageHost = source.usageUrl ? new URL(source.usageUrl).host : null;
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
        ) : source.hasQuota ? <>
          <CreditsMeter account={account} />
          {source.kind === "gemini-cli" && <ProjectField account={account} sources={sources} />}
        </> : (
          <p className="text-xs text-muted-foreground">
            {t(`accountSources.usage.${source.kind}`, { defaultValue: t("accountSources.vendorUsage") })}{" "}
            {source.usageUrl && (
              <a className="inline-flex items-center gap-0.5 font-medium text-primary hover:underline" href={source.usageUrl} target="_blank" rel="noreferrer">
                {usageHost}<ExternalLink className="h-3 w-3" />
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
          {source.hasQuota && (
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

/** The Google Cloud project a Gemini CLI account's requests are billed to. */
function ProjectField({ account, sources }: { account: AccountSourceAccount; sources: Sources }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(account.project ?? "");
  useEffect(() => setDraft(account.project ?? ""), [account.project]);
  const saving = sources.setProject.isPending && sources.setProject.variables?.id === account.id;
  const next = draft.trim();
  return (
    <form className="mt-2 flex items-center gap-1.5" data-source-project
      onSubmit={event => { event.preventDefault(); if (next && next !== account.project) sources.setProject.mutate({ id: account.id, project: next }); }}>
      <Input value={draft} onChange={event => setDraft(event.target.value)} placeholder={t("accountSources.projectPlaceholder")}
        aria-label={t("accountSources.project")} className="h-7 min-w-0 flex-1 font-mono text-[11px]" spellCheck={false} maxLength={64} disabled={saving} />
      <Button type="submit" variant="outline" size="xs" className="h-7 shrink-0" disabled={saving || !next || next === account.project}>
        {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t("common.save")}
      </Button>
    </form>
  );
}

/** The code a device sign-in asks the user to type at the vendor's page. */
function DeviceCode({ code, url }: { code: string; url: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex min-w-0 flex-1 flex-wrap items-center gap-3 sm:justify-center" data-device-code>
      <span className="sr-only">{t("accountSources.deviceCode")}</span>
      <code className="source-device-code" aria-live="polite">
        {[...code].map((char, index) => (
          <span key={index} style={{ animationDelay: `${index * 45}ms` }} data-dash={char === "-" || undefined}>{char}</span>
        ))}
      </code>
      <div className="flex items-center gap-1.5">
        <Button type="button" variant="outline" size="xs" className="h-7 gap-1.5"
          onClick={() => void navigator.clipboard?.writeText(code).then(() => setCopied(true), () => setCopied(false))}>
          {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}{t(copied ? "accountSources.codeCopied" : "accountSources.copyCode")}
        </Button>
        <a href={url} target="_blank" rel="noreferrer"
          className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-xs font-medium text-primary hover:bg-primary/10">
          {new URL(url).host}<ExternalLink className="h-3 w-3" />
        </a>
      </div>
    </div>
  );
}

/** One account source: its share with model inject, sign-in, accounts and models. */
export function AccountSourcePanel({ kind, sources }: { kind: AccountSourceKind; sources: Sources }) {
  const { t } = useTranslation();
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const modelsFold = useFold(`account-source-models-${kind}`, false);
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
  const hidden = new Set(source.hiddenModels ?? []);
  const choosing = sources.setHiddenModels.isPending && sources.setHiddenModels.variables?.kind === kind;
  const chooseModels = (next: string[]) => sources.setHiddenModels.mutate({ kind, hidden: next });
  const includedModels = source.models.length - source.models.filter(model => hidden.has(model.id)).length;
  return (
    <section aria-labelledby={titleId} data-account-source={kind} className="space-y-4">
      <div className="harbor-well grid gap-5 p-4 md:grid-cols-[minmax(0,1fr)_minmax(16rem,24rem)] md:items-center">
        <div className="min-w-0 space-y-1.5">
          <h3 id={titleId} className="text-sm font-semibold">{source.name}</h3>
          <p className="max-w-2xl text-sm text-muted-foreground">{t(`accountSources.desc.${kind}`)}</p>
          <p className="max-w-2xl text-xs text-muted-foreground/90" data-source-note>{t(`accountSources.note.${kind}`)}</p>
          {source.risk && (
            <p className="flex max-w-2xl items-start gap-1.5 text-xs text-amber-700 dark:text-amber-300" data-source-risk>
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />{t(`accountSources.risk.${kind}`)}
            </p>
          )}
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
            {source.fetchesModels && (
              <Button variant="outline" size="xs" className="h-7 gap-1.5" disabled={modelsRefreshing || source.accounts.length === 0}
                onClick={() => sources.refreshModels.mutate(kind)}>
                <RefreshCw className={cn("h-3.5 w-3.5", modelsRefreshing && "animate-spin")} />{t("accountSources.refreshModels")}
              </Button>
            )}
          </div>
          <p id={`${titleId}-share`} className="text-xs text-muted-foreground md:text-right">
            {source.exposed
              ? t("accountSources.exposedModels", { count: includedModels })
              : source.models.length === 0 ? t("accountSources.noModelsYet")
                : includedModels === 0 ? t("accountSources.noModelsChosen") : t("accountSources.notExposed")}
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
              <p className="text-sm text-muted-foreground">
                {t(sources.cancelling ? "accounts.cancellingHint" : sources.prompt?.code ? "accountSources.deviceCodeHint" : "accountSources.signInHint")}
              </p>
            </div>
          </div>
          {sources.prompt?.code && <DeviceCode code={sources.prompt.code} url={sources.prompt.url} />}
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
        <div className="rounded-xl border border-border/60 bg-card/60 px-4 py-3" data-source-models>
          <FoldHeader open={modelsFold.open} onToggle={modelsFold.toggle} titleId={`${titleId}-models`} contentId={`${titleId}-models-body`}
            title={t("accountSources.modelListShared", { shared: source.models.length - hidden.size, count: source.models.length })} titleClassName="text-sm font-medium" />
          <FoldBody open={modelsFold.open} id={`${titleId}-models-body`} className="mt-3 space-y-2">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              <span className="min-w-0 flex-1">{t("accountSources.modelFilterHint")}</span>
              <button type="button" className="font-medium text-primary hover:underline disabled:opacity-40" data-models-all
                disabled={choosing || hidden.size === 0} onClick={() => chooseModels([])}>{t("accountSources.modelsAll")}</button>
              <button type="button" className="font-medium text-primary hover:underline disabled:opacity-40" data-models-none
                disabled={choosing || includedModels === 0}
                title={source.exposed ? t("accountSources.modelsNoneShared") : undefined}
                onClick={() => chooseModels(source.models.map(model => model.id))}>{t("accountSources.modelsNone")}</button>
            </div>
            <ul className="flex flex-wrap gap-1.5">
              {source.models.map((model, index) => {
                const shared = !hidden.has(model.id);
                const last = shared && source.exposed && includedModels === 1;
                return (
                  <li key={model.id} className="route-rise" style={{ animationDelay: `${Math.min(index, 24) * 18}ms` }}>
                    <button type="button" aria-pressed={shared} data-source-model={model.id} disabled={choosing}
                      title={last ? t("accountSources.modelsNoneShared") : model.id}
                      onClick={() => chooseModels(shared ? [...hidden, model.id] : [...hidden].filter(id => id !== model.id))}
                      className={cn("rounded-full border px-2.5 py-0.5 font-mono text-[11px] transition-colors hover:border-primary/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                        shared ? "border-primary/40 bg-primary/10 text-foreground" : "border-border/60 bg-transparent text-muted-foreground line-through decoration-muted-foreground/60")}>
                      {model.name || model.id}
                    </button>
                  </li>
                );
              })}
            </ul>
          </FoldBody>
        </div>
      )}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
      <ConfirmDialog onClose={sources.dismissRisk} request={sources.riskFor === kind ? {
        title: t("accountSources.riskTitle", { name: source.name }),
        description: t(`accountSources.risk.${kind}`),
        confirmLabel: t("accountSources.riskAccept"),
        destructive: true,
        onConfirm: sources.acceptRisk,
      } : null} />
    </section>
  );
}
