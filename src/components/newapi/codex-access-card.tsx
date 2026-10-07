import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { aggregationQueryKey } from "@/types/aggregation";
import { ArrowRight, Cpu, Loader2, Network, Plus, RefreshCw, Sparkles } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { AuroraCard } from "@/components/ui/aurora-card";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useFold } from "@/hooks/use-fold";
import { toast } from "@/hooks/use-toast";
import { providerModelProtocol } from "@/lib/provider-models";
import { protocolBadgeKey, softBadge } from "@/components/newapi/relay-groups";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { CodexAccessMode, NewApiProvider } from "@/types/newapi";
import { cn } from "@/lib/utils";
import { CodexSubagentSettingsFields } from "./codex-subagent-settings";

// Codex routing switch, access mode picker and accountless toggle.
export function CodexAccessCard({
  store,
  providers,
  selectedProvider,
  nativeCompatible,
  onRestart,
  onOpenRelays,
}: {
  store: ProviderState;
  providers: NewApiProvider[];
  selectedProvider: NewApiProvider | undefined;
  nativeCompatible: boolean;
  onRestart: () => void;
  onOpenRelays?: () => void;
}) {
  const { t } = useTranslation();
  const { busy, mutationLocked, run, applyChange, appliedNote } = store;
  const fold = useFold("codex-access");
  const router = store.state.data?.router;
  const routingEnabled = !!router?.enabled;
  const accessMode = router?.accessMode ?? "router";
  const nativeAccess = accessMode === "native";
  const configDrifted = !!router?.configDrifted;
  const aggregates = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const aggregate = aggregates.data?.models.find((model) => model.id === router?.aggregateModelId);
  const aggregateSelected = router?.aggregateModelId != null;
  const aggregateAvailable = !!aggregate?.available;

  // Resolved target info for visualization
  const targetName = aggregateSelected
    ? aggregate?.displayName ?? t("aggregation.unavailable")
    : selectedProvider?.name ?? t("clients.notConfigured");
  const targetModel = aggregateSelected
    ? aggregate?.publicModelId ?? "—"
    : selectedProvider?.model ?? "—";
  const targetProtocol = aggregateSelected
    ? t("aggregation.target")
    : selectedProvider
      ? t(protocolBadgeKey[providerModelProtocol(selectedProvider)])
      : "—";

  const targetNodeClass =
    "flex min-w-0 flex-1 items-center gap-2.5 rounded-lg border border-border/50 bg-background/70 p-2";
  // Spans only: the node may be rendered inside a button.
  const targetNode = (
    <>
      <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-purple-500/15 text-purple-600 dark:text-purple-400">
        <Cpu className="h-3.5 w-3.5" />
      </span>
      <span className="block min-w-0 flex-1">
        <span className="block truncate font-medium text-foreground" title={targetName}>
          {targetName}
        </span>
        <span className="block truncate font-mono text-[11px] text-muted-foreground" title={targetModel}>
          {targetModel}
        </span>
        {nativeAccess && selectedProvider && (
          <span className="block truncate font-mono text-[11px] text-muted-foreground" title={selectedProvider.baseUrl}>
            {selectedProvider.baseUrl}
          </span>
        )}
      </span>
      <Badge variant="outline" className="ml-auto shrink-0 px-1.5 py-0 text-[10px] font-normal">
        {targetProtocol}
      </Badge>
    </>
  );

  return (
    <AuroraCard color="blue" shine={routingEnabled} className="overflow-hidden shadow-sm">
      <section aria-labelledby="newapi-routing-title" className="space-y-4 p-4 sm:p-5">
        <FoldHeader
          open={fold.open}
          onToggle={fold.toggle}
          titleId="newapi-routing-title"
          contentId="newapi-routing-body"
          title={t("newapi.codexAccess")}
          leading={
            <div className="relative flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary ring-1 ring-primary/20">
              <Sparkles className="h-4 w-4" />
              {routingEnabled && (
                <span aria-hidden className="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-emerald-500 ring-2 ring-background" />
              )}
            </div>
          }
          meta={
            <>
              {nativeAccess && (
                <Badge variant="outline" className="text-xs font-normal">
                  {t("newapi.nativeNoRouter")}
                </Badge>
              )}
              {configDrifted ? (
              <Badge className={cn(softBadge("amber"), "gap-1 font-medium")}>
                <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
                {t("newapi.driftedBadge")}
              </Badge>
            ) : routingEnabled ? (
              <Badge className={cn(softBadge("green"), "gap-1 font-medium")}>
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
                {t("newapi.enabledBadge")}
              </Badge>
            ) : (
              <Badge variant="outline" className="text-xs font-normal text-muted-foreground">
                {t("newapi.connectionDisabled")}
              </Badge>
            )}
            </>
          }
          actions={
            <>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-8 gap-1.5 rounded-lg px-3 text-xs font-medium"
                disabled={!!busy}
                onClick={onRestart}
              >
                {busy === "restart-codex" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="h-3.5 w-3.5" />
                )}
                <span>{t(busy === "restart-codex" ? "newapi.restartingCodex" : "newapi.restartCodex")}</span>
              </Button>
              <Switch
                aria-label={t("newapi.codexAccess")}
                checked={routingEnabled}
                disabled={
                  mutationLocked ||
                  (!routingEnabled &&
                    (aggregateSelected
                      ? !aggregateAvailable
                      : providers.length === 0 || (nativeAccess && !nativeCompatible)))
                }
                onCheckedChange={(value) =>
                  void run(
                    "routing",
                    async () => {
                      const change = await api.routing.setEnabled(value);
                      applyChange(change);
                      toast({
                        title: t(value ? "newapi.routingApplied" : "newapi.routingRemoved"),
                        description: change.applied?.backupPath ?? undefined,
                        variant: "success",
                      });
                    },
                    t("newapi.routingFailed"),
                  )
                }
              />
            </>
          }
        />

        <FoldBody open={fold.open} id="newapi-routing-body" className="space-y-4">
          {/* Pipeline: Codex -> gateway -> target */}
          <div className="flex flex-col items-stretch gap-2 rounded-xl border border-border/60 bg-muted/20 p-2.5 text-xs xl:flex-row xl:items-center">
            <div className="flex min-w-0 items-center gap-2.5 rounded-lg border border-border/50 bg-background/70 p-2 xl:w-auto">
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-zinc-800 font-mono text-[10px] font-bold text-white dark:bg-zinc-200 dark:text-zinc-900">
                CLI
              </div>
              <div className="min-w-0">
                <div className="font-medium text-foreground">OpenAI Codex</div>
                <div className="text-[11px] text-muted-foreground">~/.codex/config.toml</div>
              </div>
            </div>

            <ArrowRight
              aria-hidden
              className={cn("hidden h-3.5 w-3.5 shrink-0 xl:block", routingEnabled ? "text-primary" : "text-muted-foreground/40")}
            />

            {!nativeAccess && (
              <>
                <div className="flex min-w-0 items-center gap-2.5 rounded-lg border border-border/50 bg-background/70 p-2">
                  <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary">
                    <Network className="h-3.5 w-3.5" />
                  </div>
                  <div className="min-w-0">
                    <div className="font-medium text-foreground">
                      {t("newapi.gatewayAddress", { port: router?.port ?? 43127 })}
                    </div>
                    <div className="font-mono text-[11px] text-muted-foreground">/codex/v1/responses</div>
                  </div>
                </div>
                <ArrowRight
                  aria-hidden
                  className={cn("hidden h-3.5 w-3.5 shrink-0 xl:block", routingEnabled ? "text-primary" : "text-muted-foreground/40")}
                />
              </>
            )}

            {onOpenRelays ? (
              <button
                type="button"
                data-relay-link
                title={t("newapi.providersSection")}
                onClick={onOpenRelays}
                className={cn(
                  targetNodeClass,
                  "text-left transition-colors hover:border-primary/40 hover:bg-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                )}
              >
                {targetNode}
              </button>
            ) : (
              <div className={targetNodeClass}>{targetNode}</div>
            )}
          </div>

          {/* Settings */}
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="codex-access-mode" className="text-xs font-medium text-muted-foreground">
                {t("newapi.accessMode")}
              </Label>
              <Select
                value={accessMode}
                disabled={mutationLocked}
                onValueChange={(value) =>
                  void run(
                    "access-mode",
                    async () => {
                      applyChange(await api.routing.setAccessMode(value as CodexAccessMode));
                      toast({ title: t("newapi.accessModeApplied"), variant: "success" });
                    },
                    t("newapi.routingFailed"),
                  )
                }
              >
                <SelectTrigger id="codex-access-mode" className="bg-background/80">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="router">{t("newapi.accessRouter")}</SelectItem>
                  <SelectItem value="native" disabled={!nativeCompatible || !!router?.aggregateModelId}>
                    {t("newapi.accessNative")}
                  </SelectItem>
                </SelectContent>
              </Select>
              {!aggregateSelected && selectedProvider && (
                <p className="truncate font-mono text-[11px] text-muted-foreground">
                  {t("newapi.accessSelectedModel", {
                    provider: selectedProvider.name,
                    model: selectedProvider.model,
                    protocol: t(protocolBadgeKey[providerModelProtocol(selectedProvider)]),
                  })}
                </p>
              )}
            </div>

            {!nativeAccess && (
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor="codex-aggregate-model" className="text-xs font-medium text-muted-foreground">
                  {t("aggregation.codexTarget")}
                </Label>
                <Select
                  value={router?.aggregateModelId ?? "__single__"}
                  disabled={mutationLocked || aggregates.isPending}
                  onValueChange={(value) =>
                    void run(
                      "aggregate-target",
                      async () => {
                        applyChange(await api.aggregation.selectCodex(value === "__single__" ? null : value));
                      },
                      t("newapi.routingFailed"),
                    )
                  }
                >
                  <SelectTrigger id="codex-aggregate-model" className="bg-background/80">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="__single__">{t("aggregation.singleChannel")}</SelectItem>
                    {router?.aggregateModelId && !aggregate && (
                      <SelectItem value={router.aggregateModelId} disabled>
                        {t("aggregation.unavailable")}
                      </SelectItem>
                    )}
                    {(aggregates.data?.models ?? []).map((model) => (
                      <SelectItem key={model.id} value={model.id} disabled={!model.available}>
                        {model.displayName} · {model.publicModelId}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}
          </div>

          {!nativeAccess && (
            <div className="flex items-center justify-between gap-4 rounded-xl border border-border/60 bg-background/60 px-3.5 py-2.5">
              <span className="text-sm font-medium">{t("newapi.accountless")}</span>
              <Switch
                aria-label={t("newapi.accountless")}
                checked={!!router?.accountless}
                disabled={mutationLocked}
                onCheckedChange={(value) =>
                  void run(
                    "accountless",
                    async () => {
                      const change = await api.routing.setAccountless(value);
                      applyChange(change);
                      toast({ title: t("newapi.accountlessApplied"), variant: "success" });
                    },
                    t("newapi.routingFailed"),
                  )
                }
              />
            </div>
          )}

          <CodexSubagentSettingsFields store={store} />

          {providers.length === 0 && onOpenRelays && (
            <Button type="button" variant="outline" size="sm" className="h-8 gap-1.5 text-xs" onClick={onOpenRelays}>
              <Plus className="h-3.5 w-3.5" />
              {t("newapi.addProvider")}
            </Button>
          )}

          {!nativeCompatible && (
            <p className="text-xs text-amber-700 dark:text-amber-300">
              {t(selectedProvider ? "newapi.nativeProtocolHint" : "newapi.nativeProviderHint")}
            </p>
          )}
          {nativeAccess && <p className="text-xs text-muted-foreground">{t("newapi.nativeAuthHint")}</p>}

          {configDrifted && (
            <p
              role="alert"
              className="rounded-lg border border-amber-500/30 bg-amber-500/[0.07] px-3 py-2 text-[13px] leading-relaxed text-amber-700 dark:text-amber-300"
            >
              {t("newapi.configDriftedHint")}
            </p>
          )}

          {appliedNote && (
            <p className="break-all rounded border border-border/40 bg-background/50 px-2 py-1 font-mono text-[11px] text-muted-foreground/80">
              {appliedNote.path}
              {appliedNote.backupPath ? ` · ${t("newapi.backup")}: ${appliedNote.backupPath}` : ""}
            </p>
          )}
        </FoldBody>
      </section>
    </AuroraCard>
  );
}
