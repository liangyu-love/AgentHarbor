import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown, ArrowRight, Check, ChevronDown, Copy, FileCode2, Info, Layers3, Loader2, RefreshCw, ShieldAlert, ShieldCheck, ShieldOff,
  SquareTerminal, TerminalSquare, Waypoints,
} from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/hooks/use-toast";
import { formatInvokeError } from "@/lib/invoke-error";
import { cn } from "@/lib/utils";
import type { ModelSources } from "@/types/gateway";
import type { CursorCommitLocale, CursorStatus } from "@/types/cursor";
import { softBadge } from "./relay-groups";
import { ClientGlyph } from "./configuration-shortcuts";
import { ConsoleHeader } from "./client-console";
import { ModelSourceSwitches } from "./model-source-switches";
import { revealWebSearchSettings } from "./web-search-settings";

const OFFICIAL = "__cursor__";

function Panel({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("harbor-well min-w-0 p-4", className)}>
      <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{title}</h3>
      {children}
    </div>
  );
}

/** One switchable way into AgentHarbor: its name, what it does and its switch. */
function AccessRow({ icon, title, hint, state, control, children }: {
  icon: ReactNode;
  title: string;
  hint: string;
  state?: ReactNode;
  control: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="rounded-xl border border-border/60 bg-card px-3.5 py-3">
      <div className="flex items-center justify-between gap-4">
        <span className="flex min-w-0 items-start gap-2.5">
          <span aria-hidden className="mt-0.5 shrink-0 text-muted-foreground">{icon}</span>
          <span className="min-w-0">
            <span className="flex flex-wrap items-center gap-2 text-sm font-medium">{title}{state}</span>
            <span className="block text-[11px] leading-relaxed text-muted-foreground">{hint}</span>
          </span>
        </span>
        {control}
      </div>
      {children}
    </div>
  );
}

function CopyLine({ text, label, className, children }: { text: string; label: string; className?: string; children?: ReactNode }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <div className={cn("flex min-w-0 items-center gap-1 rounded-lg border border-border/50 bg-background/60 py-1 pl-2.5 pr-1", className)}>
      <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground" title={text}>{text}</code>
      <Button type="button" size="icon" variant="ghost" className="h-6 w-6 shrink-0" aria-label={label} title={label}
        onClick={() => {
          void navigator.clipboard?.writeText(text).then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          }).catch(() => toast({ title: t("cursor.copyFailed"), variant: "destructive" }));
        }}>
        {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
      </Button>
      {children}
    </div>
  );
}

/** A short explanation inside the folded notes. */
function Note({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="min-w-0 space-y-1">
      <h4 className="text-[11px] font-semibold text-foreground">{title}</h4>
      <div className="space-y-1.5 text-[11px] leading-relaxed text-muted-foreground">{children}</div>
    </div>
  );
}

// Cursor IDE and cursor-agent: how each connects (the IDE with its local
// certificate), the models, Tab and commit messages offered, and the longer
// explanations folded underneath.
export function CursorAccessCard({ onOpenRelays }: { onOpenRelays?: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const status = useQuery({ queryKey: queryKeys.cursorStatus, queryFn: api.cursor.status });
  const [busy, setBusy] = useState<string | null>(null);
  const data = status.data;

  const run = async (key: string, work: () => Promise<CursorStatus | void>, success?: string) => {
    setBusy(key);
    try {
      const next = await work();
      if (next) queryClient.setQueryData(queryKeys.cursorStatus, next);
      else void status.refetch();
      if (success) toast({ title: success, variant: "success" });
    } catch (error) {
      toast({ title: t("cursor.failed"), description: formatInvokeError(error), variant: "destructive" });
      void status.refetch();
    } finally {
      setBusy(null);
    }
  };

  const ide = !!data?.ide;
  const cli = !!data?.cli;
  const routed = !!data?.running && (data.ideApplied || data.cliApplied);
  const trusted = data?.certificate === "trusted";
  const counts = {
    relay: data?.models.filter(model => model.kind === "relay").length ?? 0,
    account: data?.models.filter(model => model.kind === "account").length ?? 0,
    aggregate: data?.models.filter(model => model.kind === "aggregate").length ?? 0,
  };
  const locked = !!busy || !data;
  const models = data?.models ?? [];
  const modelGroups = new Map<string, typeof models>();
  for (const model of models) {
    const key = `${model.kind}/${model.sourceName}`;
    const group = modelGroups.get(key);
    if (group) group.push(model);
    else modelGroups.set(key, [model]);
  }
  const sourceNames = [...modelGroups.values()].map(group => group[0].sourceName).join(" · ");
  const modelLabel = (id: string) => {
    const model = models.find(item => item.id === id);
    return model ? `${model.label} · ${model.sourceName}` : id;
  };
  const offered = (["relay", "account", "aggregate"] as const).filter(kind => data?.sources[kind]);
  const searchMissing = !!data && !data.webSearchReady;

  const step = "flex min-w-0 items-center gap-3 rounded-xl border bg-card/90 p-2.5 lg:flex-1";
  const stepIcon = (lit: boolean) => cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ring-1 transition-colors",
    lit ? "bg-primary/10 text-primary ring-primary/25" : "bg-muted text-muted-foreground ring-border/70");
  const connector = (
    <div aria-hidden className="flex shrink-0 justify-center py-1 lg:items-center lg:px-1 lg:py-0">
      <ArrowDown className={cn("h-3.5 w-3.5 transition-colors lg:-rotate-90", routed ? "text-primary" : "text-muted-foreground/40")} />
    </div>
  );
  const appliedBadge = (applied: boolean) => applied
    ? <Badge className={cn(softBadge("green"), "px-1.5 py-0 text-[10px] font-medium")}>{t("cursor.applied")}</Badge>
    : <Badge className={cn(softBadge("amber"), "px-1.5 py-0 text-[10px] font-medium")}>{t("cursor.notApplied")}</Badge>;

  const headerStatus = !data ? null : routed ? (
    <Badge className={cn(softBadge("green"), "gap-1 font-medium")}>
      <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
      {t("cursor.status.routed")}
    </Badge>
  ) : ide || cli ? (
    <Badge className={cn(softBadge("amber"), "gap-1 font-medium")}>
      <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
      {t("cursor.status.pending")}
    </Badge>
  ) : (
    <Badge variant="outline" className="text-xs font-normal text-muted-foreground">{t("newapi.connectionDisabled")}</Badge>
  );

  const certificateBadge = data && (
    data.certificate === "trusted"
      ? <Badge className={cn(softBadge("green"), "gap-1 font-medium")}><ShieldCheck className="h-3 w-3" />{t("cursor.certificate.trusted")}</Badge>
      : data.certificate === "untrusted"
        ? <Badge className={cn(softBadge("amber"), "gap-1 font-medium")}><ShieldAlert className="h-3 w-3" />{t("cursor.certificate.untrusted")}</Badge>
        : <Badge variant="outline" className="gap-1 text-xs font-normal"><ShieldAlert className="h-3 w-3" />{t("cursor.certificate.manual")}</Badge>
  );

  return (
    <section aria-labelledby="cursor-access-title" className="space-y-4" data-cursor-access>
      <ConsoleHeader
        titleId="cursor-access-title"
        title={t("cursor.title")}
        leading={
          <div className="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-background ring-1 ring-border">
            <ClientGlyph client="cursor" className="h-5 w-5" />
            {routed && <span aria-hidden className="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-emerald-500 ring-2 ring-card" />}
          </div>
        }
        meta={
          <>
            {headerStatus}
            {data?.port != null && (
              <Badge variant="outline" className="font-mono text-[11px] font-normal">127.0.0.1:{data.port}</Badge>
            )}
          </>
        }
        actions={
          <Button type="button" variant="outline" size="sm" className="h-8 gap-1.5 rounded-xl px-3 text-xs font-medium"
            data-config-action="restart-cursor" disabled={locked || !data?.ideInstalled}
            onClick={() => void run("restart", async () => { await api.cursor.restartIde(); }, t("cursor.restarted"))}>
            {busy === "restart" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
            {t("cursor.restart")}
          </Button>
        }
      />

      {status.isError && (
        <p role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2 text-[13px] text-destructive">
          {formatInvokeError(status.error)}
        </p>
      )}

      <div className="grid items-stretch gap-4 lg:grid-cols-2">
        <Panel title={t("cursor.accessPanel")}>
          <div className="space-y-2.5">
            <AccessRow
              icon={<FileCode2 className="h-4 w-4" />}
              title={t("cursor.ide")}
              hint={t(data && !data.ideInstalled ? "cursor.ideMissing" : "cursor.ideHint")}
              state={ide && data && appliedBadge(data.ideApplied)}
              control={
                <Switch aria-label={t("cursor.ide")} data-cursor-switch="ide" checked={ide}
                  disabled={locked || (!ide && (!data?.ideInstalled || data.certificate === "untrusted"))}
                  onCheckedChange={value => void run("ide", () => api.cursor.setIde(value),
                    t(value ? "cursor.ideOn" : "cursor.ideOff"))} />
              }
            >
              {/* The certificate only matters to the IDE, so it lives on its row. */}
              <div className="mt-2.5 flex flex-wrap items-center gap-2 border-t border-border/50 pt-2.5" data-cursor-certificate-row>
                <span className="text-[11px] font-medium text-muted-foreground">{t("cursor.certificatePanel")}</span>
                {certificateBadge}
                {data && data.certificate !== "manual" && (trusted ? (
                  <Button type="button" variant="ghost" size="sm" className="ml-auto h-7 gap-1.5 rounded-lg px-2.5 text-xs text-muted-foreground"
                    data-cursor-certificate="remove" disabled={locked}
                    onClick={() => void run("distrust", api.cursor.distrustCertificate, t("cursor.certificate.removed"))}>
                    {busy === "distrust" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldOff className="h-3.5 w-3.5" />}
                    {t("cursor.certificate.remove")}
                  </Button>
                ) : (
                  <Button type="button" size="sm" className="ml-auto h-7 gap-1.5 rounded-lg px-2.5 text-xs"
                    data-cursor-certificate="trust" disabled={locked}
                    onClick={() => void run("trust", api.cursor.trustCertificate, t("cursor.certificate.installed"))}>
                    {busy === "trust" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldCheck className="h-3.5 w-3.5" />}
                    {t("cursor.certificate.install")}
                  </Button>
                ))}
              </div>
              {!ide && data?.ideInstalled && data.certificate === "untrusted" && (
                <p className="mt-2 text-[11px] leading-relaxed text-amber-700 dark:text-amber-300">{t("cursor.trustFirst")}</p>
              )}
              {data?.certificate === "manual" && data.trustCommand && (
                <CopyLine className="mt-2" text={data.trustCommand} label={t("cursor.copyCommand")} />
              )}
            </AccessRow>

            <AccessRow
              icon={<TerminalSquare className="h-4 w-4" />}
              title={t("cursor.cli")}
              hint={t(!data?.cliSupported ? "cursor.cliManual" : data.cliInstalled ? "cursor.cliHint" : "cursor.cliMissing")}
              state={cli && data && appliedBadge(data.cliApplied)}
              control={
                <Switch aria-label={t("cursor.cli")} data-cursor-switch="cli" checked={cli}
                  disabled={locked || (!cli && !data?.cliSupported)}
                  onCheckedChange={value => void run("cli", () => api.cursor.setCli(value),
                    t(value ? "cursor.cliOn" : "cursor.cliOff"))} />
              }
            >
              {data?.cliCommand && (
                <CopyLine className="mt-2.5" text={data.cliCommand} label={t("cursor.copyCommand")}>
                  {data.running && (
                    <Button type="button" size="icon" variant="ghost" className="h-6 w-6 shrink-0"
                      aria-label={t("cursor.openTerminal")} title={t("cursor.openTerminal")} disabled={locked}
                      onClick={() => void run("terminal", async () => { await api.cursor.openTerminal(); })}>
                      <SquareTerminal className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </CopyLine>
              )}
            </AccessRow>
          </div>
        </Panel>

        <Panel title={t("cursor.modelsPanel")}>
          <div className="space-y-1.5" data-cursor-sources>
            <p className="text-xs font-medium text-muted-foreground">{t("cursor.sources")}</p>
            <ModelSourceSwitches value={data?.sources ?? { relay: true, account: true, aggregate: true }} counts={counts}
              disabled={locked} label={t("cursor.sources")}
              onChange={(next: ModelSources) => void run("sources", () => api.cursor.setSources(next), t("cursor.sourcesApplied"))} />
          </div>
          {models.length > 0 ? (
            <details className="group/models mt-3 border-t border-border/60 pt-3" data-cursor-models>
              <summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded-md text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 [&::-webkit-details-marker]:hidden">
                <span className="min-w-0">
                  <span className="font-medium">{t("cursor.modelCount", { count: models.length })}</span>
                  <span className="mt-0.5 block truncate text-[11px] text-muted-foreground" title={sourceNames}>{sourceNames}</span>
                </span>
                <span className="flex shrink-0 items-center gap-1.5 text-muted-foreground">
                  {t("cursor.modelDetails")}
                  <ChevronDown aria-hidden className="h-3.5 w-3.5 transition-transform group-open/models:rotate-180 motion-reduce:transition-none" />
                </span>
              </summary>
              <div role="region" aria-label={t("cursor.modelDetails")} tabIndex={0}
                className="mt-3 max-h-52 space-y-3 overflow-y-auto overscroll-contain rounded-md pr-2 outline-none focus-visible:ring-2 focus-visible:ring-ring"
                data-cursor-model-list>
                {[...modelGroups].map(([key, group]) => (
                  <div key={key}>
                    <div className="mb-1 flex min-w-0 items-center justify-between gap-3 text-[11px] text-muted-foreground">
                      <span className="truncate font-medium" title={group[0].sourceName}>{group[0].sourceName}</span>
                      <span className="shrink-0 tabular-nums">{group.length}</span>
                    </div>
                    <ul className="space-y-0.5">
                      {group.map(model => (
                        <li key={model.id} title={`${model.label} · ${model.id} → ${model.cursorId}`}
                          className="truncate py-1 text-xs" data-cursor-model={model.id}>
                          {model.label}
                        </li>
                      ))}
                    </ul>
                  </div>
                ))}
              </div>
            </details>
          ) : data && (
            <button type="button" onClick={onOpenRelays} className="mt-3 text-[11px] text-primary underline-offset-2 hover:underline" data-cursor-models>
              {t("cursor.noModels")}
            </button>
          )}

          <div className="mt-3 grid gap-3 border-t border-border/60 pt-3 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="cursor-tab-model" className="text-xs font-medium text-muted-foreground">{t("cursor.tab")}</Label>
              <Select value={data?.tabModel ?? OFFICIAL} disabled={locked}
                onValueChange={value => void run("tab", () => api.cursor.setTabModel(value === OFFICIAL ? null : value), t("cursor.tabApplied"))}>
                <SelectTrigger id="cursor-tab-model" className="min-w-0 bg-card [&>span]:truncate"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={OFFICIAL}>{t("cursor.official")}</SelectItem>
                  {data?.tabModel && !models.some(model => model.id === data.tabModel) && (
                    <SelectItem value={data.tabModel} disabled>{t("cursor.modelGone", { model: data.tabModel })}</SelectItem>
                  )}
                  {models.map(model => <SelectItem key={model.id} value={model.id}>{modelLabel(model.id)}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="cursor-commit-model" className="text-xs font-medium text-muted-foreground">{t("cursor.commit")}</Label>
              <div className="flex min-w-0 gap-1.5">
                <Select value={data?.commitModel ?? OFFICIAL} disabled={locked}
                  onValueChange={value => void run("commit", () => api.cursor.setCommitModel(value === OFFICIAL ? null : value,
                    data?.commitLocale ?? "english"), t("cursor.commitApplied"))}>
                  <SelectTrigger id="cursor-commit-model" className="min-w-0 flex-1 bg-card [&>span]:truncate"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={OFFICIAL}>{t("cursor.official")}</SelectItem>
                    {data?.commitModel && !models.some(model => model.id === data.commitModel) && (
                      <SelectItem value={data.commitModel} disabled>{t("cursor.modelGone", { model: data.commitModel })}</SelectItem>
                    )}
                    {models.map(model => <SelectItem key={model.id} value={model.id}>{modelLabel(model.id)}</SelectItem>)}
                  </SelectContent>
                </Select>
                {data?.commitModel && (
                  <Select value={data.commitLocale} disabled={locked}
                    onValueChange={value => void run("commit", () => api.cursor.setCommitModel(data.commitModel,
                      value as CursorCommitLocale), t("cursor.commitApplied"))}>
                    <SelectTrigger aria-label={t("cursor.commitLocale")} className="w-24 shrink-0 bg-card text-xs"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="english">{t("cursor.locale.english")}</SelectItem>
                      <SelectItem value="chinese">{t("cursor.locale.chinese")}</SelectItem>
                    </SelectContent>
                  </Select>
                )}
              </div>
            </div>
          </div>

          {/* WebSearch runs on the search service shared under the client console. */}
          <div className="mt-3 flex items-center justify-between gap-3" data-cursor-web-search>
            <p className={cn("min-w-0 text-[11px] leading-relaxed", searchMissing ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground")}>
              {t(searchMissing ? "cursor.webSearchMissing" : "cursor.webSearchReady")}
            </p>
            <button type="button" onClick={revealWebSearchSettings}
              className="shrink-0 rounded-md text-[11px] font-medium text-primary underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
              {t("cursor.webSearchSet")}
            </button>
          </div>
        </Panel>
      </div>

      {data && !data.routerRunning && (ide || cli) && (
        <p role="status" className="rounded-xl border border-amber-500/30 bg-amber-500/[0.07] px-3 py-2 text-[11px] text-amber-700 dark:text-amber-300">
          {t("cursor.routerDown")}
        </p>
      )}

      {/* How it works, kept one click away so the panel stays short. */}
      <details className="group/notes harbor-well min-w-0 text-xs" data-cursor-notes>
        <summary className="flex cursor-pointer list-none items-center gap-2.5 rounded-[inherit] px-4 py-3 outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          <Info aria-hidden className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          <span className="shrink-0 font-medium text-foreground">{t("cursor.notesPanel")}</span>
          <span className="min-w-0 flex-1 truncate text-[11px] text-muted-foreground">{t("cursor.notesSummary")}</span>
          <ChevronDown aria-hidden className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-open/notes:rotate-180 motion-reduce:transition-none" />
        </summary>
        <div className="space-y-4 border-t border-border/60 px-4 pb-4 pt-3">
          <div data-cursor-pipeline className="flex flex-col lg:flex-row lg:items-stretch">
            <div className={cn(step, "border-border/60")}>
              <span className={stepIcon(routed)}><ClientGlyph client="cursor" className="h-4 w-4" /></span>
              <span className="min-w-0">
                <span className="block font-medium text-foreground">{t("cursor.pipelineClients")}</span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground">
                  {[ide && "http.proxy", cli && "CURSOR_API_ENDPOINT"].filter(Boolean).join(" · ") || "—"}
                </span>
              </span>
            </div>
            {connector}
            <div className={cn(step, routed ? "border-primary/35" : "border-border/60")}>
              <span className={stepIcon(routed)}><Waypoints className="h-4 w-4" /></span>
              <span className="min-w-0">
                <span className="block font-medium text-foreground">{t("cursor.pipelinePort", { port: data?.port ?? "—" })}</span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground">{t("cursor.pipelinePortHint")}</span>
              </span>
            </div>
            {connector}
            <button type="button" onClick={onOpenRelays} disabled={!onOpenRelays}
              className={cn(step, "group w-full border-border/60 text-left transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:hover:border-border/60")}>
              <span className={stepIcon(routed && offered.length > 0)}><Layers3 className="h-4 w-4" /></span>
              <span className="min-w-0">
                <span className="block truncate font-medium text-foreground">
                  {offered.length ? offered.map(kind => t(`gateway.kind.${kind}`)).join(" + ") : t("cursor.sourcesNone")}
                </span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground">{t("cursor.modelCount", { count: models.length })}</span>
              </span>
            </button>
          </div>

          <div className="grid gap-x-6 gap-y-4 lg:grid-cols-2">
            <Note title={t("cursor.notes.routing")}>
              <p className="flex items-start gap-1.5">
                <ArrowRight aria-hidden className="mt-0.5 h-3 w-3 shrink-0" />
                {t("cursor.pipelineHint")}
              </p>
              <p>{t("cursor.notes.ide")}</p>
            </Note>
            <Note title={t("cursor.notes.account")}>
              <p>{t("cursor.accountNote")}</p>
            </Note>
            <Note title={t("cursor.certificatePanel")}>
              <p>{t("cursor.certificate.hint")}</p>
              {data?.trustCommand && data.certificate !== "manual" && (
                <CopyLine text={data.trustCommand} label={t("cursor.copyCommand")} />
              )}
              {data?.certificatePath && (
                <p className="truncate font-mono text-[10.5px] text-muted-foreground/80" title={data.certificatePath}>{data.certificatePath}</p>
              )}
            </Note>
            <Note title={t("cursor.notes.models")}>
              <p>{t("cursor.sourcesHint")}</p>
              <p>{t("cursor.tabHint")}</p>
              {data?.settingsPath && (
                <p className="truncate font-mono text-[10.5px] text-muted-foreground/80" title={data.settingsPath}>{data.settingsPath}</p>
              )}
            </Note>
          </div>
        </div>
      </details>
    </section>
  );
}
