import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Boxes, Check, Copy, Eye, EyeOff, KeyRound, Layers3, Plus, RefreshCw, Search, Trash2, UsersRound, Waypoints } from "lucide-react";

import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { SignalPath, linkPath, useAnchors } from "@/components/ui/signal-lines";
import { useFold } from "@/hooks/use-fold";
import { toast } from "@/hooks/use-toast";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { useDecodedText } from "@/lib/motion";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { GatewayKey, GatewayModel, GatewayProfile, GatewayState } from "@/types/gateway";
import type { ModelProtocol } from "@/types/newapi";

function CopyButton({ text, label, onCopy }: { text?: string; label: string; onCopy?: () => Promise<string> }) {
  const { t } = useTranslation();
  const [done, setDone] = useState(false);
  return (
    <Button type="button" variant="ghost" size="icon-sm" className="h-7 w-7 shrink-0 text-muted-foreground" aria-label={label} title={label}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(onCopy ? await onCopy() : text ?? "");
          setDone(true);
          setTimeout(() => setDone(false), 1500);
        } catch (error) {
          toast({ title: t("gateway.copyFailed"), description: formatInvokeError(error), variant: "destructive" });
        }
      }}>
      {done ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
    </Button>
  );
}

const KIND_ORDER: GatewayModel["kind"][] = ["relay", "account", "aggregate"];

/** The shared prefix of aggregates' gateway ids; an aggregate may name its own. */
function AggregatePrefixForm({ value, busy, onSave }: { value: string; busy: boolean; onSave: (prefix: string) => void }) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const next = draft.trim();
  const dirty = next !== value;
  return (
    <form className="flex flex-wrap items-center gap-x-2 gap-y-1.5" data-gateway-prefix
      onSubmit={event => { event.preventDefault(); if (dirty && next) onSave(next); }}>
      <Label htmlFor="gateway-aggregate-prefix" className="text-xs font-medium">{t("gateway.aggregatePrefix")}</Label>
      <span className="inline-flex items-center gap-1 font-mono text-xs">
        <Input id="gateway-aggregate-prefix" value={draft} onChange={event => setDraft(event.target.value.toLowerCase())}
          className="h-8 w-28 font-mono text-xs" maxLength={40} spellCheck={false} disabled={busy}
          aria-describedby="gateway-aggregate-prefix-hint" />
        <span aria-hidden className="text-muted-foreground">/gpt-5.5</span>
      </span>
      <Button type="submit" size="sm" variant="outline" className="h-8" disabled={busy || !dirty || !next}>{t("common.save")}</Button>
      <p id="gateway-aggregate-prefix-hint" className="w-full text-[11px] text-muted-foreground">{t("gateway.aggregatePrefixHint")}</p>
    </form>
  );
}
const KIND_ICON = { relay: Boxes, account: UsersRound, aggregate: Layers3 } as const;
const SHOWN_KEYS = 4;

/** A key's secret that decodes into place when it is revealed or replaced. */
function KeySecret({ text }: { text: string }) {
  const shown = useDecodedText(text);
  return <>{shown}</>;
}

/**
 * Agents (their keys) → the gateway port → the three kinds of model it serves.
 * Running, the lines carry traffic as on the routing topology; a key or kind
 * with nothing behind it stays a dashed hairline. Turning the gateway on or
 * adding a key re-lights the lines that changed.
 */
function GatewaySchematic({ state }: { state: GatewayState | undefined }) {
  const { t } = useTranslation();
  const board = useRef<HTMLDivElement>(null);
  const { anchors, register, measure } = useAnchors(board);
  const keys = state?.keys ?? [];
  const shown = keys.slice(0, SHOWN_KEYS);
  const running = state?.running ?? false;
  const counts = Object.fromEntries(KIND_ORDER.map(kind => [kind, (state?.models ?? []).filter(model => model.kind === kind).length]));
  const [ignition, setIgnition] = useState(0);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    setIgnition(value => value + 1);
  }, [running]);
  // Keys come and go: measure again once their chips are laid out.
  const keySignature = shown.map(key => key.id).join("|");
  useEffect(() => { measure(); }, [keySignature, measure]);
  const keyNode = (key: GatewayKey) => (
    <span key={key.id} ref={register(`key:${key.id}`)} data-schematic-key={key.id}
      className="route-rise flex max-w-[11rem] items-center gap-1.5 rounded-xl border border-border bg-card px-2.5 py-1.5 text-xs font-medium shadow-sm">
      <KeyRound aria-hidden className={cn("h-3.5 w-3.5 shrink-0", running ? "text-primary" : "text-muted-foreground")} />
      <span className="truncate">{key.name}</span>
    </span>
  );
  return (
    <div ref={board} data-gateway-schematic={running ? "live" : "idle"}
      className="route-surface relative grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-x-8 gap-y-3 rounded-xl px-3 py-4 sm:gap-x-14 sm:px-5">
      <svg aria-hidden className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
        {shown.map(key => (
          <SignalPath key={key.id} d={linkPath(anchors, `key:${key.id}`, "hub")} live={running} ignition={ignition} speed={0.9} />
        ))}
        {shown.length === 0 && <SignalPath d={linkPath(anchors, "key:none", "hub")} live={false} />}
        {KIND_ORDER.map(kind => (
          <SignalPath key={kind} d={linkPath(anchors, "hub", `kind:${kind}`)} live={running && counts[kind] > 0} ignition={ignition} speed={1.15} />
        ))}
      </svg>
      <div className="relative flex min-w-0 flex-col items-start gap-2">
        {shown.length === 0 && (
          <span ref={register("key:none")} className="rounded-xl border border-dashed border-border px-2.5 py-1.5 text-xs text-muted-foreground">
            {t("gateway.schematicNoKey")}
          </span>
        )}
        {shown.map(keyNode)}
        {keys.length > SHOWN_KEYS && <span className="pl-1 text-[11px] text-muted-foreground">+{keys.length - SHOWN_KEYS}</span>}
      </div>
      <div className="relative flex flex-col items-center gap-1.5">
        <span ref={register("hub")} className="relative flex h-[4.5rem] w-[4.5rem] items-center justify-center">
          {running && <span aria-hidden className="route-hub-halo absolute inset-0 rounded-full border border-primary/40" />}
          <span aria-hidden className={cn("absolute inset-0 rounded-full", running ? "route-hub-ring" : "border border-dashed border-border")} />
          <span key={ignition} className={cn("relative flex h-[3.75rem] w-[3.75rem] flex-col items-center justify-center rounded-full border bg-card shadow-md",
            running ? "border-primary/30" : "border-border", ignition > 0 && "route-hub-ignite")}>
            <Waypoints aria-hidden className={cn("h-4 w-4", running ? "text-primary" : "text-muted-foreground")} />
            <span className="font-mono text-[10px] text-muted-foreground">:{state?.port ?? 43127}</span>
          </span>
        </span>
        <span className="font-mono text-[10px] text-muted-foreground">/gateway</span>
      </div>
      <div className="relative flex min-w-0 flex-col items-end gap-2">
        {KIND_ORDER.map(kind => {
          const Icon = KIND_ICON[kind];
          const live = running && counts[kind] > 0;
          return (
            <span key={kind} ref={register(`kind:${kind}`)} data-schematic-kind={kind} data-live={live || undefined}
              className={cn("flex items-center gap-2 rounded-xl border bg-card px-2.5 py-1.5 text-xs shadow-sm transition-[border-color,opacity] duration-300",
                live ? "border-primary/40" : "border-border", counts[kind] === 0 && "opacity-60")}>
              <Icon aria-hidden className={cn("h-3.5 w-3.5", live ? "text-primary" : "text-muted-foreground")} />
              <span className="truncate">{t(`gateway.kind.${kind}`)}</span>
              <span className="font-mono text-[11px] tabular-nums text-muted-foreground">{counts[kind]}</span>
            </span>
          );
        })}
      </div>
    </div>
  );
}

/**
 * The general gateway: one local address and named keys through which any
 * agent reaches every enabled relay model, shared account source and
 * aggregate, each under its own id, with protocol translation.
 */
export function GatewayPanel({ providersRevision }: { providersRevision: number }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: queryKeys.gateway, queryFn: api.gateway.state, refetchOnWindowFocus: false });
  const state = query.data;
  // Provider edits change which models the gateway serves.
  const { refetch } = query;
  useEffect(() => { if (providersRevision) void refetch(); }, [providersRevision, refetch]);
  const modelsFold = useFold("gateway-models", false);
  const exportFold = useFold("gateway-export", false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [revealed, setRevealed] = useState<Record<string, string>>({});
  const [filter, setFilter] = useState("");
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const [keyId, setKeyId] = useState("");
  const [modelId, setModelId] = useState("");
  const [ingress, setIngress] = useState<ModelProtocol>("responses");
  const [profile, setProfile] = useState<GatewayProfile>("generic");
  const [configText, setConfigText] = useState<string | null>(null);

  const port = state?.port ?? 43127;
  const openAiBase = `http://127.0.0.1:${port}/gateway/v1`;
  const anthropicBase = `http://127.0.0.1:${port}/gateway`;
  const keys = state?.keys ?? [];
  const models = state?.models ?? [];
  const exportKey = keys.find(key => key.id === keyId) ?? keys[0];
  const exportModel = modelId ? models.find(model => model.id === modelId) : models[0];

  const run = async (action: () => Promise<GatewayState | void>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const next = await action();
      if (next) {
        queryClient.setQueryData(queryKeys.gateway, next);
        setConfigText(null);
      }
    } catch (cause) {
      setError(formatInvokeError(cause));
    } finally {
      setBusy(false);
    }
  };
  const reveal = async (id: string) => revealed[id] ?? await api.gateway.revealKey(id);
  const grouped = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    const visible = models.filter(model => !needle || `${model.id} ${model.label} ${model.sourceName}`.toLowerCase().includes(needle));
    return KIND_ORDER.map(kind => ({ kind, models: visible.filter(model => model.kind === kind) })).filter(group => group.models.length);
  }, [models, filter]);
  const exportConnection = (revealCredential: boolean) => api.gateway.exportConnection({
    ingressProtocol: ingress, selectedModelId: exportModel?.id ?? null, consumerProfile: profile,
    keyId: exportKey?.id ?? "", revealCredential,
  });

  return (
    <section aria-labelledby="gateway-title" data-gateway className="space-y-4 rounded-2xl border border-border/70 bg-card/80 p-4 shadow-sm sm:p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <div className="flex items-center gap-2">
            <h3 id="gateway-title" className="text-sm font-semibold">{t("gateway.title")}</h3>
            <span data-gateway-status className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium",
              state?.running ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300" : "bg-muted text-muted-foreground")}>
              <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", state?.running ? "bg-emerald-500" : "bg-muted-foreground/40")} />
              {t(state?.running ? "gateway.running" : "gateway.stopped")}
            </span>
          </div>
          <p className="max-w-3xl text-xs text-muted-foreground">{t("gateway.description")}</p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Label htmlFor="gateway-keep-running" className="cursor-pointer text-xs font-medium">{t("gateway.keepRunning")}</Label>
          <Switch id="gateway-keep-running" checked={state?.keepRunning ?? false} disabled={busy || !query.isSuccess}
            onCheckedChange={value => void run(() => api.gateway.setKeepRunning(value))} />
        </div>
      </div>

      {(error || query.isError) && (
        <p role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          {error ?? formatInvokeError(query.error)}
        </p>
      )}

      <GatewaySchematic state={state} />

      <div className="grid gap-3 md:grid-cols-2">
        {([["gateway.openAiBase", openAiBase], ["gateway.anthropicBase", anthropicBase]] as const).map(([label, url]) => (
          <div key={label} className="harbor-well min-w-0 space-y-1 px-3 py-2.5" data-gateway-endpoint>
            <p className="text-[11px] text-muted-foreground">{t(label)}</p>
            <div className="flex items-center gap-2">
              <code className="min-w-0 flex-1 truncate font-mono text-xs">{url}</code>
              <CopyButton text={url} label={t("gateway.copyAddress")} />
            </div>
          </div>
        ))}
      </div>

      <div className="space-y-2" data-gateway-keys>
        <div className="flex items-center justify-between gap-2">
          <p className="flex items-center gap-1.5 text-xs font-semibold"><KeyRound className="h-3.5 w-3.5" />{t("gateway.keys")}</p>
          <span className="text-[11px] text-muted-foreground">{t("gateway.keysHint")}</span>
        </div>
        {keys.length === 0 ? (
          <p className="rounded-xl border border-dashed border-border px-3 py-3 text-xs text-muted-foreground">{t("gateway.noKeys")}</p>
        ) : (
          <ul className="divide-y divide-border/60 overflow-hidden rounded-xl border border-border/70">
            {keys.map(key => (
              <li key={key.id} data-gateway-key={key.id} className="flex flex-wrap items-center gap-2 bg-background/40 px-3 py-2">
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{key.name}</span>
                <code className="max-w-full truncate font-mono text-xs text-muted-foreground sm:max-w-[22rem]" data-gateway-secret>
                  <KeySecret text={revealed[key.id] ?? key.masked} />
                </code>
                <div className="flex items-center">
                  <Button type="button" variant="ghost" size="icon-sm" className="h-7 w-7 text-muted-foreground"
                    aria-label={t(revealed[key.id] ? "gateway.hideKey" : "gateway.showKey")} aria-pressed={!!revealed[key.id]}
                    onClick={() => {
                      if (revealed[key.id]) setRevealed(({ [key.id]: _, ...rest }) => rest);
                      else void run(async () => { const secret = await api.gateway.revealKey(key.id); setRevealed(current => ({ ...current, [key.id]: secret })); });
                    }}>
                    {revealed[key.id] ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
                  </Button>
                  <CopyButton label={t("gateway.copyKey")} onCopy={() => reveal(key.id)} />
                  <Button type="button" variant="ghost" size="icon-sm" className="h-7 w-7 text-muted-foreground" aria-label={t("gateway.regenerate")} title={t("gateway.regenerate")}
                    onClick={() => setConfirm({ title: t("gateway.regenerateTitle"), description: t("gateway.regenerateDescription", { name: key.name }),
                      confirmLabel: t("gateway.regenerate"), destructive: true,
                      onConfirm: () => { setConfirm(null); setRevealed(({ [key.id]: _, ...rest }) => rest); void run(() => api.gateway.regenerateKey(key.id)); } })}>
                    <RefreshCw className="h-3.5 w-3.5" />
                  </Button>
                  <Button type="button" variant="ghost" size="icon-sm" className="h-7 w-7 text-muted-foreground hover:bg-destructive/15 hover:text-destructive"
                    aria-label={t("common.delete")} title={t("common.delete")}
                    onClick={() => setConfirm({ title: t("gateway.deleteTitle"), description: t("gateway.deleteDescription", { name: key.name }),
                      confirmLabel: t("common.delete"), destructive: true,
                      onConfirm: () => { setConfirm(null); void run(() => api.gateway.deleteKey(key.id)); } })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
        <form className="flex gap-2" onSubmit={event => {
          event.preventDefault();
          const name = newName.trim();
          if (name) void run(async () => { const next = await api.gateway.createKey(name); setNewName(""); return next; });
        }}>
          <Input value={newName} onChange={event => setNewName(event.target.value)} maxLength={40} className="h-9 text-sm"
            placeholder={t("gateway.keyNamePlaceholder")} aria-label={t("gateway.keyName")} disabled={busy} />
          <Button type="submit" size="sm" className="h-9 shrink-0 gap-1.5" disabled={busy || !newName.trim()}>
            <Plus className="h-3.5 w-3.5" />{t("gateway.createKey")}
          </Button>
        </form>
      </div>

      <div className="rounded-xl border border-border/60 px-3 py-2.5">
        <FoldHeader open={modelsFold.open} onToggle={modelsFold.toggle} titleId="gateway-models-title" contentId="gateway-models-body"
          title={t("gateway.models", { count: models.length })} titleClassName="text-xs" />
        <FoldBody open={modelsFold.open} id="gateway-models-body" className="mt-3 space-y-3">
          <AggregatePrefixForm value={state?.aggregatePrefix ?? "pool"} busy={busy || !query.isSuccess}
            onSave={prefix => void run(() => api.gateway.setAggregatePrefix(prefix))} />
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input value={filter} onChange={event => setFilter(event.target.value)} className="h-8 pl-8 text-xs"
              placeholder={t("gateway.filterModels")} aria-label={t("gateway.filterModels")} />
          </div>
          {models.length === 0 && <p className="text-xs text-muted-foreground">{t("gateway.noModels")}</p>}
          {grouped.map(group => (
            <div key={group.kind} className="space-y-1" data-gateway-group={group.kind}>
              <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">{t(`gateway.kind.${group.kind}`)}</p>
              <ul className="space-y-0.5">
                {group.models.map(model => (
                  <li key={model.id} data-gateway-model={model.id} className="flex items-center gap-2 rounded-lg px-2 py-1 hover:bg-muted/50">
                    <code className="min-w-0 flex-1 truncate font-mono text-xs" title={model.id}>{model.id}</code>
                    <span className="hidden min-w-0 max-w-[14rem] truncate text-[11px] text-muted-foreground sm:block">{model.sourceName}</span>
                    {model.contextWindow && <span className="shrink-0 font-mono text-[10px] tabular-nums text-muted-foreground">{Math.round(model.contextWindow / 1000)}K</span>}
                    <CopyButton text={model.id} label={t("gateway.copyModel")} />
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </FoldBody>
      </div>

      <div className="rounded-xl border border-border/60 px-3 py-2.5">
        <FoldHeader open={exportFold.open} onToggle={exportFold.toggle} titleId="gateway-export-title" contentId="gateway-export-body"
          title={t("gateway.connection")} titleClassName="text-xs" />
        <FoldBody open={exportFold.open} id="gateway-export-body" className="mt-3 space-y-3">
          <div className="grid grid-cols-1 items-end gap-3 md:grid-cols-[repeat(4,minmax(0,1fr))_auto]">
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="gateway-export-key" className="text-xs font-medium text-muted-foreground">{t("gateway.keyName")}</Label>
              <Select value={exportKey?.id ?? ""} onValueChange={value => { setKeyId(value); setConfigText(null); }}>
                <SelectTrigger id="gateway-export-key" className="bg-card text-xs"><SelectValue placeholder={t("gateway.noKeys")} /></SelectTrigger>
                <SelectContent>{keys.map(key => <SelectItem key={key.id} value={key.id}>{key.name}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="gateway-export-model" className="text-xs font-medium text-muted-foreground">{t("gateway.defaultModel")}</Label>
              <Select value={exportModel?.id ?? modelId} onValueChange={value => { setModelId(value); setConfigText(null); }}>
                <SelectTrigger id="gateway-export-model" className="bg-card text-xs"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {modelId && !exportModel && <SelectItem value={modelId} disabled>{t("gateway.unavailable")}</SelectItem>}
                  {models.map(model => <SelectItem key={model.id} value={model.id}>{model.id}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="gateway-export-protocol" className="text-xs font-medium text-muted-foreground">{t("gateway.ingress")}</Label>
              <Select value={ingress} onValueChange={value => { setIngress(value as ModelProtocol); setConfigText(null); }}>
                <SelectTrigger id="gateway-export-protocol" className="bg-card text-xs"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="responses" disabled={profile === "anthropicSdk"}>Responses</SelectItem>
                  <SelectItem value="chatCompletions" disabled={profile === "anthropicSdk"}>Chat</SelectItem>
                  <SelectItem value="anthropicMessages">Messages</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="gateway-export-profile" className="text-xs font-medium text-muted-foreground">{t("gateway.template")}</Label>
              <Select value={profile} onValueChange={value => {
                setProfile(value as GatewayProfile);
                if (value === "anthropicSdk") setIngress("anthropicMessages");
                setConfigText(null);
              }}>
                <SelectTrigger id="gateway-export-profile" className="bg-card text-xs"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="generic">{t("gateway.generic")}</SelectItem>
                  <SelectItem value="anthropicSdk">Anthropic SDK</SelectItem>
                  <SelectItem value="opencode">OpenCode</SelectItem>
                </SelectContent>
              </Select>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button variant="outline" size="sm" className="h-9 text-xs" disabled={busy || !exportKey || !exportModel}
                onClick={() => void run(async () => setConfigText((await exportConnection(false)).configText))}>
                {t("gateway.preview")}
              </Button>
              <Button size="sm" className="h-9 gap-1.5 text-xs" disabled={busy || !exportKey || !exportModel}
                onClick={() => void run(async () => {
                  const result = await exportConnection(true);
                  await navigator.clipboard.writeText(result.configText);
                  toast({ title: t("gateway.copied"), variant: "success" });
                })}>
                <Copy className="h-3.5 w-3.5" />{t("gateway.copyConnection")}
              </Button>
            </div>
          </div>
          {configText !== null && (
            <pre className="gateway-preview-reveal max-h-72 overflow-auto rounded-xl border border-border/60 bg-zinc-950 p-4 font-mono text-xs leading-relaxed text-zinc-100"
              data-gateway-preview key={configText}>{configText}</pre>
          )}
        </FoldBody>
      </div>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </section>
  );
}
