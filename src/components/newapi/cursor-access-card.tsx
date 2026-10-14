import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowDown, ArrowRight, Check, Copy, FileCode2, Layers3, Loader2, RefreshCw, ShieldAlert, ShieldCheck, ShieldOff, SquareTerminal,
  TerminalSquare, Waypoints,
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

function CopyLine({ text, label }: { text: string; label: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  return (
    <div className="mt-2.5 flex min-w-0 items-center gap-2 rounded-lg border border-border/50 bg-background/60 py-1 pl-2.5 pr-1">
      <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground" title={text}>{text}</code>
      <Button type="button" size="icon" variant="ghost" className="h-6 w-6 shrink-0" aria-label={label}
        onClick={() => {
          void navigator.clipboard?.writeText(text).then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          }).catch(() => toast({ title: t("cursor.copyFailed"), variant: "destructive" }));
        }}>
        {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
      </Button>
    </div>
  );
}

// Cursor IDE and cursor-agent: the local port they reach AgentHarbor through,
// the certificate the IDE needs, the models offered, and Tab / commit messages.
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
  const modelLabel = (id: string) => {
    const model = models.find(item => item.id === id);
    return model ? `${model.label} · ${model.sourceName}` : id;
  };
  const offered = (["relay", "account", "aggregate"] as const).filter(kind => data?.sources[kind]);

  const step = "flex min-w-0 items-center gap-3 rounded-xl border bg-card/90 p-2.5 lg:flex-1";
  const stepIcon = (lit: boolean) => cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ring-1 transition-colors",
    lit ? "bg-primary/10 text-primary ring-primary/25" : "bg-muted text-muted-foreground ring-border/70");
  const connector = (
    <div aria-hidden className="flex shrink-0 justify-center py-1 lg:items-center lg:px-1 lg:py-0">
      <ArrowDown className={cn("h-3.5 w-3.5 transition-colors lg:-rotate-90", routed ? "text-primary" : "text-muted-foreground/40")} />
    </div>
  );

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
    <section aria-labelledby="cursor-access-title" className="space-y-5" data-cursor-access>
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
              state={ide && data && (data.ideApplied
                ? <Badge className={cn(softBadge("green"), "px-1.5 py-0 text-[10px] font-medium")}>{t("cursor.applied")}</Badge>
                : <Badge className={cn(softBadge("amber"), "px-1.5 py-0 text-[10px] font-medium")}>{t("cursor.notApplied")}</Badge>)}
              control={
                <Switch aria-label={t("cursor.ide")} data-cursor-switch="ide" checked={ide}
                  disabled={locked || (!ide && (!data?.ideInstalled || data.certificate === "untrusted"))}
                  onCheckedChange={value => void run("ide", () => api.cursor.setIde(value),
                    t(value ? "cursor.ideOn" : "cursor.ideOff"))} />
              }
            >
              {!ide && data?.ideInstalled && data.certificate === "untrusted" && (
                <p className="mt-2 text-[11px] leading-relaxed text-amber-700 dark:text-amber-300">{t("cursor.trustFirst")}</p>
              )}
              {data?.settingsPath && (
                <p className="mt-1.5 truncate font-mono text-[10.5px] text-muted-foreground/80" title={data.settingsPath}>{data.settingsPath}</p>
              )}
            </AccessRow>

            <AccessRow
              icon={<TerminalSquare className="h-4 w-4" />}
              title={t("cursor.cli")}
              hint={t(!data?.cliSupported ? "cursor.cliManual" : data.cliInstalled ? "cursor.cliHint" : "cursor.cliMissing")}
              state={cli && data && (data.cliApplied
                ? <Badge className={cn(softBadge("green"), "px-1.5 py-0 text-[10px] font-medium")}>{t("cursor.applied")}</Badge>
                : <Badge className={cn(softBadge("amber"), "px-1.5 py-0 text-[10px] font-medium")}>{t("cursor.notApplied")}</Badge>)}
              control={
                <Switch aria-label={t("cursor.cli")} data-cursor-switch="cli" checked={cli}
                  disabled={locked || (!cli && !data?.cliSupported)}
                  onCheckedChange={value => void run("cli", () => api.cursor.setCli(value),
                    t(value ? "cursor.cliOn" : "cursor.cliOff"))} />
              }
            >
              {data?.cliCommand && <CopyLine text={data.cliCommand} label={t("cursor.copyCommand")} />}
              {data?.running && (
                <Button type="button" variant="outline" size="sm" className="mt-2.5 h-7 gap-1.5 rounded-lg px-2.5 text-xs"
                  disabled={locked} onClick={() => void run("terminal", async () => { await api.cursor.openTerminal(); })}>
                  <SquareTerminal className="h-3.5 w-3.5" />
                  {t("cursor.openTerminal")}
                </Button>
              )}
            </AccessRow>
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">{t("cursor.accountNote")}</p>
        </Panel>

        <Panel title={t("cursor.certificatePanel")}>
          <div className="flex flex-wrap items-center gap-2">
            {certificateBadge}
            {data?.certificate !== "manual" && (trusted ? (
              <Button type="button" variant="outline" size="sm" className="h-7 gap-1.5 rounded-lg px-2.5 text-xs"
                data-cursor-certificate="remove" disabled={locked}
                onClick={() => void run("distrust", api.cursor.distrustCertificate, t("cursor.certificate.removed"))}>
                {busy === "distrust" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldOff className="h-3.5 w-3.5" />}
                {t("cursor.certificate.remove")}
              </Button>
            ) : (
              <Button type="button" size="sm" className="h-7 gap-1.5 rounded-lg px-2.5 text-xs"
                data-cursor-certificate="trust" disabled={locked}
                onClick={() => void run("trust", api.cursor.trustCertificate, t("cursor.certificate.installed"))}>
                {busy === "trust" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ShieldCheck className="h-3.5 w-3.5" />}
                {t("cursor.certificate.install")}
              </Button>
            ))}
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">{t("cursor.certificate.hint")}</p>
          {data?.trustCommand && <CopyLine text={data.trustCommand} label={t("cursor.copyCommand")} />}
          {data?.certificatePath && (
            <p className="mt-2 truncate font-mono text-[10.5px] text-muted-foreground/80" title={data.certificatePath}>{data.certificatePath}</p>
          )}
        </Panel>

        <Panel title={t("cursor.modelsPanel")}>
          <div className="space-y-1.5" data-cursor-sources>
            <p className="text-xs font-medium text-muted-foreground">{t("cursor.sources")}</p>
            <ModelSourceSwitches value={data?.sources ?? { relay: true, account: true, aggregate: true }} counts={counts}
              disabled={locked} label={t("cursor.sources")}
              onChange={(next: ModelSources) => void run("sources", () => api.cursor.setSources(next), t("cursor.sourcesApplied"))} />
            <p className="text-[11px] leading-relaxed text-muted-foreground">{t("cursor.sourcesHint")}</p>
          </div>
          <div className="mt-3 flex flex-wrap gap-1.5" data-cursor-models>
            {models.slice(0, 12).map(model => (
              <span key={model.id} title={`${model.id} → ${model.cursorId}`}
                className="max-w-[14rem] truncate rounded-full border border-border/60 bg-card px-2 py-0.5 text-[11px]">
                {model.label}<span className="text-muted-foreground"> · {model.sourceName}</span>
              </span>
            ))}
            {models.length > 12 && (
              <span className="rounded-full px-2 py-0.5 text-[11px] text-muted-foreground">{t("cursor.moreModels", { count: models.length - 12 })}</span>
            )}
            {data && models.length === 0 && (
              <button type="button" onClick={onOpenRelays} className="text-[11px] text-primary underline-offset-2 hover:underline">
                {t("cursor.noModels")}
              </button>
            )}
          </div>
        </Panel>

        <Panel title={t("cursor.extrasPanel")}>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="cursor-tab-model" className="text-xs font-medium text-muted-foreground">{t("cursor.tab")}</Label>
              <Select value={data?.tabModel ?? OFFICIAL} disabled={locked}
                onValueChange={value => void run("tab", () => api.cursor.setTabModel(value === OFFICIAL ? null : value), t("cursor.tabApplied"))}>
                <SelectTrigger id="cursor-tab-model" className="bg-card"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={OFFICIAL}>{t("cursor.official")}</SelectItem>
                  {data?.tabModel && !models.some(model => model.id === data.tabModel) && (
                    <SelectItem value={data.tabModel} disabled>{t("cursor.modelGone", { model: data.tabModel })}</SelectItem>
                  )}
                  {models.map(model => <SelectItem key={model.id} value={model.id}>{modelLabel(model.id)}</SelectItem>)}
                </SelectContent>
              </Select>
              <p className="text-[11px] leading-relaxed text-muted-foreground">{t("cursor.tabHint")}</p>
            </div>
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="cursor-commit-model" className="text-xs font-medium text-muted-foreground">{t("cursor.commit")}</Label>
              <Select value={data?.commitModel ?? OFFICIAL} disabled={locked}
                onValueChange={value => void run("commit", () => api.cursor.setCommitModel(value === OFFICIAL ? null : value,
                  data?.commitLocale ?? "english"), t("cursor.commitApplied"))}>
                <SelectTrigger id="cursor-commit-model" className="bg-card"><SelectValue /></SelectTrigger>
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
                  <SelectTrigger aria-label={t("cursor.commitLocale")} className="h-8 bg-card text-xs"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="english">{t("cursor.locale.english")}</SelectItem>
                    <SelectItem value="chinese">{t("cursor.locale.chinese")}</SelectItem>
                  </SelectContent>
                </Select>
              )}
            </div>
          </div>
          <p className={cn("mt-3 text-[11px] leading-relaxed", data && !data.webSearchReady ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground")}>
            {t(data && !data.webSearchReady ? "cursor.webSearchMissing" : "cursor.webSearchReady")}
          </p>
        </Panel>
      </div>

      <Panel title={t("newapi.pipelinePanel")} className="text-xs">
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
        <p className="mt-3 flex items-start gap-1.5 text-[11px] leading-relaxed text-muted-foreground">
          <ArrowRight aria-hidden className="mt-0.5 h-3 w-3 shrink-0" />
          {t("cursor.pipelineHint")}
        </p>
        {data && !data.routerRunning && (ide || cli) && (
          <p role="status" className="mt-2 text-[11px] text-amber-700 dark:text-amber-300">{t("cursor.routerDown")}</p>
        )}
      </Panel>
    </section>
  );
}
