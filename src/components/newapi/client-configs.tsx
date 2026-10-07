import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertCircle,
  ArrowRight,
  Code2,
  RefreshCw,
  RotateCcw,
  Settings2,
  Terminal,
  Unlink,
} from "lucide-react";
import { api } from "@/lib/api";
import { clientModels, clientProtocols, conflictScopes, exclusionOf, type Exclusion } from "@/lib/client-config";
import { ConfigDialog } from "./client-config-dialog";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { useFold } from "@/hooks/use-fold";
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
import type { ClientStatus } from "@/types/clients";
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
  t: (key: string, options?: Record<string, unknown>) => string,
) => {
  const counts: Record<Exclusion, number> = { protocol: 0, disabled: 0, noApiKey: 0, noModel: 0 };
  for (const provider of providers) {
    const reason = exclusionOf(provider, protocols);
    if (reason) counts[reason] += 1;
  }
  const order: Exclusion[] = ["protocol", "disabled", "noApiKey", "noModel"];
  return order
    .filter((reason) => counts[reason] > 0)
    .map((reason) => t(`clients.excluded.${reason}`, { count: counts[reason] }));
};

export function ClientConfigs({ providers, onOpenRelays }: { providers: NewApiProvider[]; onOpenRelays?: () => void }) {
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
  const fold = useFold("client-configs");

  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: CLIENT_KEY }),
      queryClient.invalidateQueries({ queryKey: queryKeys.providers }),
    ]);
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
          : t("clients.restartHint"),
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

  return (
    <section className="space-y-4" aria-labelledby="client-configs-title">
      <FoldHeader
        open={fold.open}
        onToggle={fold.toggle}
        titleId="client-configs-title"
        contentId="client-configs-body"
        title={t("clients.title")}
        leading={
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Terminal className="h-4 w-4" />
          </div>
        }
      />

      <FoldBody open={fold.open} id="client-configs-body" className="space-y-4">
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

      <div className="grid gap-4 lg:grid-cols-2">
        {clients.data?.map((client) => {
          const provider = providers.find((p) => p.id === client.providerId);
          const mode = client.request?.accessMode ?? (client.configured ? "router" : "native");
          const protocols = clientProtocols(client, mode);
          const modelIds = client.targetId === "claudeCode" ? client.request?.claudeModelIds : null;
          const selection = client.request?.modelSelection;
          const aggregateId = selection?.defaultTarget?.kind === "aggregate" ? selection.defaultTarget.aggregateId : null;
          const aggregate = aggregates.data?.models.find((model) => model.id === aggregateId);
          const sourceName = aggregateId ? aggregate?.displayName ?? t("aggregation.unavailable") : provider?.name ?? client.providerId;
          const modelName = aggregateId
            ? aggregate?.displayName ?? client.model
            : provider?.models.find((model) => model.id === client.model)?.displayName || client.model;
          const configuredSummary =
            client.targetId === "claudeCode"
              ? t(modelIds?.length ? "clients.configuredModelsSummary" : "clients.configuredDefaultSummary", {
                  provider: sourceName,
                  count: modelIds ? new Set(modelIds).size : 0,
                  model: modelName,
                })
              : `${sourceName} / ${client.model}`;

          const targetUnavailable = (target: ModelTargetRef) => {
            if (target.kind === "aggregate")
              return !aggregates.data?.models.some((model) => model.id === target.aggregateId && model.available);
            const source = providers.find((item) => item.id === target.providerId);
            return !source || !!exclusionOf(source, protocols) || !clientModels(source, protocols).includes(target.modelId);
          };

          const unavailable =
            client.configured &&
            (selection
              ? !selection.defaultTarget || targetUnavailable(selection.defaultTarget) || selection.availableTargets.some(targetUnavailable)
              : !provider ||
                !!exclusionOf(provider, protocols) ||
                !clientModels(provider, protocols).includes(client.model ?? "") ||
                client.request?.providerIds.some((id) => !providers.some((p) => p.id === id && !exclusionOf(p, protocols))));

          const eligible = providers.filter((p) => !exclusionOf(p, protocols));
          const noEligible =
            eligible.length === 0 && !aggregates.data?.models.some((model) => model.available);
          const reasons = noEligible ? summarizeExclusions(providers, protocols, t) : [];

          const isClaude = client.targetId === "claudeCode";

          return (
            <div
              key={client.targetId}
              className={cn(
                "group relative flex flex-col justify-between overflow-hidden rounded-2xl border bg-card/80 p-5 shadow-sm transition-all duration-300 hover:border-primary/40 hover:shadow-md",
                client.conflict && "border-amber-500/40 bg-amber-500/[0.02]",
                client.configured && !client.conflict && "border-border/80",
              )}
            >
              {/* Subtle top indicator bar */}
              <div
                className={cn(
                  "absolute left-5 top-0 h-0.5 w-10 rounded-b-sm transition-all duration-300 group-hover:w-16",
                  isClaude
                    ? "bg-gradient-to-r from-amber-500 to-orange-500"
                    : "bg-gradient-to-r from-cyan-500 to-blue-500",
                )}
              />

              <div className="space-y-3.5">
                {/* Header */}
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex items-center gap-2.5">
                    <div
                      className={cn(
                        "flex h-9 w-9 shrink-0 items-center justify-center rounded-xl shadow-inner ring-1",
                        isClaude
                          ? "bg-amber-500/10 text-amber-600 ring-amber-500/20 dark:text-amber-400"
                          : "bg-sky-500/10 text-sky-600 ring-sky-500/20 dark:text-sky-400",
                      )}
                    >
                      {isClaude ? <Terminal className="h-4 w-4" /> : <Code2 className="h-4 w-4" />}
                    </div>
                    <div>
                      <div className="flex items-center gap-2">
                        <h3 className="font-semibold text-foreground tracking-tight">{client.name}</h3>
                        {client.targetId === "claudeCode" && (
                          <span className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] tabular-nums text-muted-foreground">
                            {client.clientVersion ? `v${client.clientVersion}` : t("clients.versionShort")}
                          </span>
                        )}
                      </div>
                      {client.configured && (
                        <div className="text-[11px] font-medium text-muted-foreground">
                          {t(mode === "native" ? "clients.accessNative" : "clients.accessRouter")}
                        </div>
                      )}
                    </div>
                  </div>

                  <Badge
                    variant="outline"
                    className={cn(
                      "gap-1 font-medium text-xs shadow-sm",
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
                </div>

                {/* Path indicator */}
                <div className="flex items-center gap-1.5 rounded-lg border border-border/50 bg-muted/30 px-2.5 py-1.5 font-mono text-xs text-muted-foreground">
                  <span className="shrink-0 text-[10px] font-sans font-semibold uppercase tracking-wider text-muted-foreground/70">
                    {t("clients.configPath")}
                  </span>
                  <span className="truncate" title={client.path}>{client.path}</span>
                </div>

                {/* Main description or configured status */}
                <div className="rounded-xl border border-border/40 bg-background/60 p-3 shadow-sm">
                  <p className="break-words text-sm font-medium leading-relaxed text-foreground">
                    {client.configured ? configuredSummary : t(`clients.${client.targetId}Hint`)}
                  </p>
                </div>

                {/* Conflict or Error alert */}
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
                  <p className="text-xs text-muted-foreground bg-muted/30 p-2 rounded-lg">
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
              </div>

              {/* Actions row */}
              <div className="mt-5 flex flex-wrap items-center gap-2 pt-3 border-t border-border/50">
                <Button
                  size="sm"
                  variant={client.configured ? "outline" : "default"}
                  className={cn(
                    "h-8 gap-1.5 rounded-lg text-xs font-semibold shadow-sm transition-all",
                    !client.configured && "bg-primary text-primary-foreground hover:bg-primary/90",
                  )}
                  disabled={busy || !!client.error}
                  onClick={() => setSelected(client)}
                >
                  <Settings2 className="h-3.5 w-3.5" />
                  {t(client.configured ? "clients.change" : "clients.configure")}
                </Button>

                {client.configured && (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 gap-1.5 rounded-lg text-xs text-muted-foreground hover:text-foreground"
                    disabled={busy || !!client.error}
                    onClick={() => setRestoring(client)}
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                    {t("clients.restore")}
                  </Button>
                )}

                {client.configured && !client.conflict && !client.error && !unavailable && (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 gap-1.5 rounded-lg text-xs text-muted-foreground hover:text-foreground"
                    disabled={busy}
                    onClick={() => setReapplying({ client, rebaseline: false })}
                  >
                    <RefreshCw className="h-3.5 w-3.5" />
                    {t("clients.reapply")}
                  </Button>
                )}

                {client.conflict && (
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-8 gap-1.5 rounded-lg text-xs text-amber-600 hover:text-amber-700 dark:text-amber-400"
                    disabled={busy}
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
                    disabled={busy}
                    onClick={() => setReleasing(client)}
                  >
                    <Unlink className="h-3.5 w-3.5" />
                    {t("clients.release")}
                  </Button>
                )}
              </div>
            </div>
          );
        })}
      </div>
      </FoldBody>

      {selected && (
        <ConfigDialog
          client={clients.data?.find((client) => client.targetId === selected.targetId) ?? selected}
          providers={providers}
          onClose={() => setSelected(null)}
          onApplied={refresh}
        />
      )}

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
