import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertCircle,
  ArrowRight,
  RefreshCw,
  RotateCcw,
  Settings2,
  Shield,
  Unlink,
} from "lucide-react";
import { api } from "@/lib/api";
import { clientModels, clientProtocols, conflictScopes, exclusionOf, reloadHint, type Exclusion } from "@/lib/client-config";
import { ConfigDialog } from "./client-config-dialog";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { ClientGlyph } from "./configuration-shortcuts";
import { ConsoleHeader } from "./client-console";
import { ClientLaunch } from "./client-launch";
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
import type { ClaudeOfficialPreview, ClientAccessMode, ClientStatus } from "@/types/clients";
import type { ModelProtocol, NewApiProvider } from "@/types/newapi";
import { queryKeys } from "@/lib/query-keys";
import { aggregationQueryKey, type ModelTargetRef } from "@/types/aggregation";
import { errorKey } from "@/lib/invoke-error";
import { cn } from "@/lib/utils";

const CLIENT_KEY = queryKeys.clientConfigs;
const clientErrorKey = (error: unknown) =>
  errorKey(error, ["clients", "newapi", "aggregation", "modelTargets"], "clients.errors.storage");

const summarizeExclusions = (
  providers: NewApiProvider[],
  protocols: ModelProtocol[],
  mode: ClientAccessMode,
  t: (key: string, options?: Record<string, unknown>) => string,
) => {
  const counts: Record<Exclusion, number> = { protocol: 0, disabled: 0, noApiKey: 0, noModel: 0, routerOnly: 0 };
  for (const provider of providers) {
    const reason = exclusionOf(provider, protocols, mode);
    if (reason) counts[reason] += 1;
  }
  const order: Exclusion[] = ["protocol", "disabled", "noApiKey", "noModel", "routerOnly"];
  return order
    .filter((reason) => counts[reason] > 0)
    .map((reason) => t(`clients.excluded.${reason}`, { count: counts[reason] }));
};

// One configurable CLI client's panel in the injection console.
export function ClientConfigs({ providers, onOpenRelays, target }: {
  providers: NewApiProvider[];
  onOpenRelays?: () => void;
  target: "claudeCode" | "opencode" | "pi" | "harness";
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const clients = useQuery({ queryKey: CLIENT_KEY, queryFn: api.targets.configs });
  const aggregates = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const [selected, setSelected] = useState<ClientStatus | null>(null);
  const [restoring, setRestoring] = useState<ClientStatus | null>(null);
  const [releasing, setReleasing] = useState<ClientStatus | null>(null);
  const [reapplying, setReapplying] = useState<{ client: ClientStatus; rebaseline: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [officialOpen, setOfficialOpen] = useState(false);
  const [officialPreview, setOfficialPreview] = useState<ClaudeOfficialPreview | null>(null);
  const [officialPending, setOfficialPending] = useState(false);
  const officialOperation = useRef<"preview" | "apply" | null>(null);
  const officialSequence = useRef(0);
  const actionsLocked = busy || officialPending;

  useEffect(() => () => { officialSequence.current += 1; }, []);

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: CLIENT_KEY }),
      queryClient.invalidateQueries({ queryKey: queryKeys.providers }),
    ]);
  };

  const closeOfficial = () => {
    if (officialOperation.current === "apply") return;
    officialSequence.current += 1;
    officialOperation.current = null;
    setOfficialPending(false);
    setOfficialOpen(false);
    setOfficialPreview(null);
    setError(null);
  };

  const previewOfficial = async () => {
    if (busy || officialOperation.current) return;
    const sequence = ++officialSequence.current;
    officialOperation.current = "preview";
    setOfficialOpen(true);
    setOfficialPending(true);
    setOfficialPreview(null);
    setError(null);
    try {
      const preview = await api.targets.previewClaudeOfficial();
      if (sequence === officialSequence.current) setOfficialPreview(preview);
    } catch (cause) {
      if (sequence === officialSequence.current) setError(clientErrorKey(cause));
    } finally {
      if (sequence === officialSequence.current) {
        officialOperation.current = null;
        setOfficialPending(false);
      }
    }
  };

  const switchOfficial = async () => {
    if (!officialPreview?.hasChanges || busy || officialOperation.current) return;
    const sequence = officialSequence.current;
    officialOperation.current = "apply";
    setBusy(true);
    setError(null);
    try {
      const applied = await api.targets.switchClaudeOfficial(officialPreview.revision);
      // The write remains successful even if the UI was closed or reloading
      // the status fails. Invalidate shared state after every successful write.
      let refreshFailed = false;
      try { await refresh(); } catch { refreshFailed = true; }
      if (sequence !== officialSequence.current) return;
      toast({
        title: t("clients.officialRestored"),
        description: [...(applied.warnings ?? []).map((key) => t(key)), ...(refreshFailed ? [t("clients.officialRefreshFailed")] : [])].join(" ") || t("clients.restartHint"),
        variant: applied.warnings?.length || refreshFailed ? "destructive" : "success",
      });
      setOfficialOpen(false);
      setOfficialPreview(null);
    } catch (cause) {
      if (sequence !== officialSequence.current) return;
      setError(clientErrorKey(cause));
      // Every failed write needs a fresh preview, especially a stale revision.
      setOfficialPreview(null);
    } finally {
      if (sequence === officialSequence.current) {
        officialOperation.current = null;
        setBusy(false);
      }
    }
  };

  const restore = async () => {
    if (!restoring || busy) return;
    setBusy(true);
    setError(null);
    try {
      const restored = await api.targets.restoreConfig(restoring.targetId, restoring.conflict);
      toast({
        title: t("clients.restored"),
        description: restored.warnings?.length ? restored.warnings.map((key) => t(key)).join(" ") : undefined,
        variant: restored.warnings?.length ? "destructive" : "success",
      });
      setRestoring(null);
    } catch (cause) {
      setError(clientErrorKey(cause));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const reapply = async () => {
    if (!reapplying || busy) return;
    setBusy(true);
    setError(null);
    try {
      const applied = await api.targets.reapplyConfig(reapplying.client.targetId, reapplying.rebaseline);
      toast({
        title: t(reapplying.rebaseline ? "clients.reappliedRebaseline" : "clients.reapplied"),
        description: applied.warnings?.length
          ? applied.warnings.map((key) => t(key)).join(" ")
          : t(reloadHint(reapplying.client.targetId)),
        variant: applied.warnings?.length ? "destructive" : "success",
      });
      setReapplying(null);
    } catch (cause) {
      setError(clientErrorKey(cause));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const discardResume = async (target: ClientStatus) => {
    setBusy(true);
    setError(null);
    try {
      await api.targets.discardResume(target.targetId);
      toast({ title: t("clients.resume.discarded"), variant: "success" });
    } catch (cause) {
      setError(clientErrorKey(cause));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const release = async () => {
    if (!releasing || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.targets.releaseConfig(releasing.targetId);
      toast({ title: t("clients.released"), variant: "success" });
      setReleasing(null);
    } catch (cause) {
      setError(clientErrorKey(cause));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const client = clients.data?.find((entry) => entry.targetId === target);
  const mode = client?.request?.accessMode ?? (client?.configured ? "router" : "native");
  const protocols = client ? clientProtocols(client, mode) : [];
  const modelIds = target === "claudeCode" ? client?.request?.claudeModelIds : null;
  const selection = client?.request?.modelSelection;
  const aggregateId = selection?.defaultTarget?.kind === "aggregate" ? selection.defaultTarget.aggregateId : null;
  const aggregate = aggregates.data?.models.find((model) => model.id === aggregateId);
  const provider = providers.find((p) => p.id === client?.providerId);
  const sourceName = aggregateId ? aggregate?.displayName ?? t("aggregation.unavailable") : provider?.name ?? client?.providerId;
  const modelName = aggregateId
    ? aggregate?.displayName ?? client?.model
    : provider?.models.find((model) => model.id === client?.model)?.displayName || client?.model;

  const targetUnavailable = (ref: ModelTargetRef) => {
    if (ref.kind === "aggregate")
      return !aggregates.data?.models.some((model) => model.id === ref.aggregateId && model.available);
    const source = providers.find((item) => item.id === ref.providerId);
    return !source || !!exclusionOf(source, protocols, mode) || !clientModels(source, protocols).includes(ref.modelId);
  };

  const unavailable =
    !!client?.configured &&
    (selection
      ? !selection.defaultTarget || targetUnavailable(selection.defaultTarget) || selection.availableTargets.some(targetUnavailable)
      : !provider ||
        !!exclusionOf(provider, protocols, mode) ||
        !clientModels(provider, protocols).includes(client.model ?? "") ||
        !!client.request?.providerIds.some((id) => !providers.some((p) => p.id === id && !exclusionOf(p, protocols, mode))));

  // Harness starts as the profile it was configured in, else the one listed first.
  const launchProfile = target === "harness" ? client?.request?.harnessProfile ?? client?.harness?.profiles[0]?.name ?? null : null;
  // Harness writes into a profile it initialized itself; none yet means nothing to write.
  const noHarnessProfile = target === "harness" && !client?.configured && !client?.harness?.profiles.length;
  const eligible = providers.filter((p) => !exclusionOf(p, protocols, mode));
  const noEligible = !!client && eligible.length === 0 && !aggregates.data?.models.some((model) => model.available);
  const reasons = noEligible ? summarizeExclusions(providers, protocols, mode, t) : [];
  // What a configured client uses, as three facts instead of one long sentence.
  const facts: { key: string; value: string; mono?: boolean }[] = client?.configured
    ? [
        { key: "access", value: t(mode === "native" ? "clients.accessNative" : "clients.accessRouter") },
        // Which Harness profile the routes went into; the others are untouched.
        ...(target === "harness" ? [{ key: "profile", value: client.request?.harnessProfile ?? "—", mono: true }] : []),
        { key: "source", value: sourceName ?? "—" },
        modelIds?.length
          ? { key: "model", value: t("clients.factModels", { count: new Set(modelIds).size }) }
          : { key: "model", value: modelName ?? "—", mono: true },
      ]
    : [];

  return (
    <section
      id={`client-config-${target}`}
      tabIndex={-1}
      className="space-y-5 focus:outline-none"
      aria-labelledby={`client-config-${target}-title`}
    >
      <ConsoleHeader
        titleId={`client-config-${target}-title`}
        title={client?.name ?? t(`newapi.topology.client.${target}`)}
        leading={
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-background text-foreground ring-1 ring-border">
            <ClientGlyph client={target} className="h-5 w-5" />
          </div>
        }
        meta={client && (
          <>
            {target === "claudeCode" && (
              <span className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] tabular-nums text-muted-foreground">
                {client.clientVersion ? `v${client.clientVersion}` : t("clients.versionShort")}
              </span>
            )}
            <Badge
              variant="outline"
              className={cn(
                "gap-1 text-xs font-medium",
                client.conflict
                  ? "border-amber-500/40 bg-amber-500/10 text-amber-700 dark:text-amber-300"
                  : client.configured
                    ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                    : "text-muted-foreground",
              )}
            >
              {client.configured && !client.conflict && <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse motion-reduce:animate-none" />}
              {client.conflict && <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />}
              {t(client.conflict ? "clients.conflict" : client.configured ? "clients.configured" : "clients.notConfigured")}
            </Badge>
          </>
        )}
        actions={client && (
          <>
            <ClientLaunch target={target} harnessProfile={launchProfile} disabled={actionsLocked}
              name={target === "harness" && launchProfile === "desktop" ? "Harness Desktop" : client.name} />
            {client.conflict && (
              <Button
                size="sm"
                variant="ghost"
                className="h-8 gap-1.5 rounded-lg text-xs text-amber-600 hover:text-amber-700 dark:text-amber-400"
                disabled={actionsLocked}
                onClick={() => setReapplying({ client, rebaseline: true })}
              >
                <RefreshCw className="h-3.5 w-3.5" />
                {t("clients.rebaseline")}
              </Button>
            )}
            {(client.conflict || (client.error && client.error !== "clients.errors.invalidConfig")) && (
              <Button
                size="sm"
                variant="ghost"
                className="h-8 gap-1.5 rounded-lg text-xs text-destructive hover:bg-destructive/10"
                disabled={actionsLocked}
                onClick={() => setReleasing(client)}
              >
                <Unlink className="h-3.5 w-3.5" />
                {t("clients.release")}
              </Button>
            )}
            {client.configured && !client.conflict && !client.error && !unavailable && (
              <Button
                size="sm"
                variant="ghost"
                className="h-8 gap-1.5 rounded-lg text-xs text-muted-foreground hover:text-foreground"
                disabled={actionsLocked}
                onClick={() => setReapplying({ client, rebaseline: false })}
              >
                <RefreshCw className="h-3.5 w-3.5" />
                {t("clients.reapply")}
              </Button>
            )}
            {client.configured && (
              <Button
                size="sm"
                variant="ghost"
                className="h-8 gap-1.5 rounded-lg text-xs text-muted-foreground hover:text-foreground"
                disabled={actionsLocked || !!client.error}
                onClick={() => setRestoring(client)}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                {t("clients.restore")}
              </Button>
            )}
            {target === "claudeCode" && (
              <Button
                size="sm"
                variant="outline"
                className="h-8 gap-1.5 rounded-lg text-xs"
                disabled={actionsLocked}
                onClick={() => void previewOfficial()}
              >
                <Shield className="h-3.5 w-3.5" />
                {t("clients.switchOfficial")}
              </Button>
            )}
            <Button
              size="sm"
              data-config-action
              variant={client.configured ? "outline" : "default"}
              className="h-8 gap-1.5 rounded-lg text-xs font-semibold shadow-sm"
              disabled={actionsLocked || !!client.error || noHarnessProfile}
              onClick={() => setSelected(client)}
            >
              <Settings2 className="h-3.5 w-3.5" />
              {t(client.configured ? "clients.change" : "clients.configure")}
            </Button>
          </>
        )}
      />

      {clients.isPending && (
        <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
          <RefreshCw className="h-4 w-4 animate-spin text-primary" />
          <span>{t("common.loading")}</span>
        </div>
      )}

      {(error || clients.error) && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3.5 text-sm text-destructive"
        >
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>{t(error ?? clientErrorKey(clients.error))}</span>
        </div>
      )}

      {!client && clients.isSuccess && (
        <p role="status" className="harbor-well p-4 text-sm text-muted-foreground">{t("clients.notDetected")}</p>
      )}

      {client && (
        <>
          {facts.length > 0 ? (
            <dl className={cn("grid gap-3", facts.length > 3 ? "sm:grid-cols-2 xl:grid-cols-4" : "sm:grid-cols-3")} data-client-facts>
              {facts.map((fact) => (
                <div key={fact.key} className="harbor-well min-w-0 px-4 py-3">
                  <dt className="poster-kicker text-[10px] text-muted-foreground">{t(`clients.fact.${fact.key}`)}</dt>
                  <dd className={cn("mt-1 truncate text-[15px] font-semibold text-foreground", fact.mono && "font-mono text-[13px]")} title={fact.value}>
                    {fact.value}
                  </dd>
                </div>
              ))}
            </dl>
          ) : (
            <p className="harbor-well p-4 text-sm leading-relaxed text-muted-foreground">{t(`clients.${client.targetId}Hint`)}</p>
          )}

          {!client.configured && client.resumePending && (
            <div role="status" data-client-resume
              className="flex flex-col gap-2 rounded-xl border border-primary/25 bg-primary/[0.05] p-3 text-xs text-foreground sm:flex-row sm:items-center sm:justify-between">
              <p className="flex items-start gap-2"><RefreshCw className="mt-0.5 h-3.5 w-3.5 shrink-0 text-primary" />{t("clients.resume.note")}</p>
              <Button size="sm" variant="ghost" className="h-7 shrink-0 text-xs text-muted-foreground" disabled={actionsLocked}
                onClick={() => void discardResume(client)}>
                {t("clients.resume.discard")}
              </Button>
            </div>
          )}

          {noHarnessProfile && (
            <p role="status" data-harness-no-profile
              className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-amber-800 dark:text-amber-200">
              <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />{t("clients.harnessNoProfiles", { home: client.harness?.home ?? "~/.dsh" })}
            </p>
          )}

          <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
            <span className="poster-kicker shrink-0 text-[10px]">{t("clients.configPath")}</span>
            <span className="min-w-0 truncate font-mono" title={client.path}>{client.path}</span>
          </div>

          {client.configured && client.stale && !client.conflict && !client.error && (
            <div role="status" data-client-stale
              className="flex flex-col gap-2 rounded-xl border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-amber-800 sm:flex-row sm:items-center sm:justify-between dark:text-amber-200">
              <p className="flex items-start gap-2"><AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />{t("clients.stale")}</p>
              <Button size="sm" variant="outline" className="h-7 shrink-0 gap-1.5 text-xs" disabled={actionsLocked}
                onClick={() => setReapplying({ client, rebaseline: false })}>
                <RefreshCw className="h-3.5 w-3.5" />{t("clients.reapply")}
              </Button>
            </div>
          )}

          {(client.conflict || client.error || unavailable) && (
            <div
              role="alert"
              className="space-y-1 rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive"
            >
              <p className="font-medium">
                {t(client.error ?? (client.conflict ? "clients.errors.conflict" : aggregateId ? "aggregation.targetUnavailable" : "clients.errors.provider"))}
              </p>
              {client.conflict && !!client.conflictFields?.length && (
                <p role="status" className="text-[11px] text-muted-foreground">
                  {t("clients.conflictScope", {
                    fields: conflictScopes(client.conflictFields)
                      .map((scope) => t("clients.conflictScopes." + scope))
                      .join(" · "),
                  })}
                </p>
              )}
            </div>
          )}

          {reasons.length > 0 && (
            <p className="rounded-lg bg-muted/30 p-2 text-xs text-muted-foreground">
              {t("clients.noEligibleProviders")} {reasons.join(" · ")}
            </p>
          )}

          {noEligible && onOpenRelays && (
            <button
              type="button"
              data-relay-link
              onClick={onOpenRelays}
              className="inline-flex items-center gap-1 rounded text-xs font-medium text-primary hover:text-primary/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              {t("newapi.addProvider")}
              <ArrowRight className="h-3 w-3" aria-hidden />
            </button>
          )}
        </>
      )}

      {selected && (
        <ConfigDialog
          client={clients.data?.find((client) => client.targetId === selected.targetId) ?? selected}
          providers={providers}
          onClose={() => setSelected(null)}
          onApplied={refresh}
        />
      )}

      <AlertDialog open={officialOpen} onOpenChange={(open) => { if (!open) closeOfficial(); }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("clients.switchOfficialTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("clients.switchOfficialDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {officialPending && (
            <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
              <RefreshCw className="h-4 w-4 animate-spin" />
              {t("common.loading")}
            </p>
          )}
          {officialPreview && (
            <div className="space-y-3">
              <p className="break-all rounded-lg bg-muted/40 px-3 py-2 font-mono text-xs text-muted-foreground">
                {officialPreview.path}
              </p>
              {officialPreview.removedFields.length > 0 && (
                <div className="space-y-1.5">
                  <p className="text-sm font-medium">{t("clients.officialRemovedFields")}</p>
                  <ul className="max-h-48 space-y-1 overflow-y-auto rounded-lg border border-border p-3 font-mono text-xs">
                    {officialPreview.removedFields.map((field) => <li key={field} className="break-all">{field}</li>)}
                  </ul>
                </div>
              )}
              {!officialPreview.hasChanges && (
                <p role="status" className="text-sm text-muted-foreground">{t("clients.officialNoChanges")}</p>
              )}
              {officialPreview.warnings.length > 0 && (
                <ul className="space-y-1 text-xs text-amber-700 dark:text-amber-300">
                  {officialPreview.warnings.map((warning) => <li key={warning} role="status">{t(warning)}</li>)}
                </ul>
              )}
            </div>
          )}
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy || officialPending || !!officialPreview && !officialPreview.hasChanges}
              onClick={(event) => {
                event.preventDefault();
                if (officialPreview) void switchOfficial();
                else void previewOfficial();
              }}
            >
              {t(officialPreview || officialPending ? "clients.switchOfficial" : "clients.preview")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Restore confirmation */}
      <AlertDialog
        open={!!restoring}
        onOpenChange={(open) => {
          if (!open && !busy) setRestoring(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("clients.restoreTitle", { name: restoring?.name })}</AlertDialogTitle>
            <AlertDialogDescription>
              {t(restoring?.conflict ? "clients.restoreConflictDescription" : "clients.restoreDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(event) => {
                event.preventDefault();
                void restore();
              }}
            >
              {t("clients.restore")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Reapply confirmation */}
      <AlertDialog
        open={!!reapplying}
        onOpenChange={(open) => {
          if (!open && !busy) setReapplying(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t(reapplying?.rebaseline ? "clients.rebaselineTitle" : "clients.reapplyTitle", {
                name: reapplying?.client.name,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t(reapplying?.rebaseline ? "clients.rebaselineDescription" : "clients.reapplyDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(event) => {
                event.preventDefault();
                void reapply();
              }}
            >
              {t(reapplying?.rebaseline ? "clients.rebaseline" : "clients.reapply")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Release confirmation */}
      <AlertDialog
        open={!!releasing}
        onOpenChange={(open) => {
          if (!open && !busy) setReleasing(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("clients.releaseTitle", { name: releasing?.name })}</AlertDialogTitle>
            <AlertDialogDescription>{t("clients.releaseDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={(event) => {
                event.preventDefault();
                void release();
              }}
            >
              {t("clients.release")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
