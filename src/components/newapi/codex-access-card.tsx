import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { aggregationQueryKey } from "@/types/aggregation";
import {
  ArrowRight,
  CircleHelp,
  Cpu,
  Loader2,
  Network,
  RefreshCw,
  Settings2,
  Sparkles,
} from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { AuroraCard } from "@/components/ui/aurora-card";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { toast } from "@/hooks/use-toast";
import { providerModelProtocol } from "@/lib/provider-models";
import { protocolBadgeKey, softBadge } from "@/components/newapi/relay-groups";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { CodexAccessMode, NewApiProvider } from "@/types/newapi";
import { cn } from "@/lib/utils";

// Codex routing switch, access mode picker and accountless toggle.
export function CodexAccessCard({
  store,
  providers,
  selectedProvider,
  nativeCompatible,
  onRestart,
}: {
  store: ProviderState;
  providers: NewApiProvider[];
  selectedProvider: NewApiProvider | undefined;
  nativeCompatible: boolean;
  onRestart: () => void;
}) {
  const { t } = useTranslation();
  const { busy, mutationLocked, run, applyChange, appliedNote } = store;
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

  return (
    <AuroraCard
      color="blue"
      shine={routingEnabled}
      className="overflow-hidden shadow-sm transition-all duration-300 hover:shadow-md"
    >
      <div aria-labelledby="newapi-routing-title" className="p-5 sm:p-6 space-y-5">
        {/* Header row */}
        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div className="flex items-start gap-3.5">
            <div className="relative flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/20 via-primary/10 to-primary/5 text-primary shadow-inner ring-1 ring-primary/25">
              <Sparkles className="h-5 w-5 transition-transform duration-300 motion-safe:group-hover/aurora:scale-110" />
              {routingEnabled && (
                <span
                  aria-hidden
                  className="absolute -right-0.5 -top-0.5 flex h-3 w-3 items-center justify-center"
                >
                  <span className="absolute inline-flex h-full w-full animate-ping motion-reduce:animate-none rounded-full bg-emerald-400 opacity-60" />
                  <span className="relative inline-flex h-2 w-2 rounded-full bg-emerald-500 ring-2 ring-background" />
                </span>
              )}
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2.5">
                <h2 id="newapi-routing-title" className="text-base font-semibold tracking-tight text-foreground">
                  {t("newapi.codexAccess")}
                </h2>
                {routingEnabled && !configDrifted && (
                  <Badge className={cn(softBadge("green"), "gap-1 shadow-sm font-medium")}>
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse motion-reduce:animate-none" />
                    {t("newapi.enabledBadge")}
                  </Badge>
                )}
                {configDrifted && (
                  <Badge className={cn(softBadge("amber"), "gap-1 shadow-sm font-medium")}>
                    <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
                    {t("newapi.driftedBadge")}
                  </Badge>
                )}
                {!routingEnabled && (
                  <Badge variant="outline" className="text-xs text-muted-foreground font-normal">
                    {t("newapi.connectionDisabled")}
                  </Badge>
                )}
              </div>
              <p className="mt-1 text-sm leading-relaxed text-muted-foreground">
                {t(nativeAccess ? "newapi.nativeAccessDesc" : "newapi.smartRoutingDesc")}
              </p>
            </div>
          </div>

          <div className="flex items-center gap-3 self-end sm:self-auto shrink-0">
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-9 gap-2 rounded-xl border-border/80 bg-background/80 px-3.5 text-xs font-semibold shadow-sm backdrop-blur-md transition-all hover:border-primary/40 hover:bg-background hover:shadow-sm motion-safe:active:scale-95"
              disabled={!!busy}
              onClick={onRestart}
            >
              {busy === "restart-codex" ? (
                <Loader2 className="h-4 w-4 animate-spin text-primary" />
              ) : (
                <RefreshCw className="h-4 w-4 text-primary transition-transform group-hover:rotate-45" />
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
          </div>
        </div>

        {/* Visual Pipeline Topology */}
        <div className="rounded-xl border border-border/60 bg-gradient-to-r from-card/80 via-muted/20 to-card/80 p-3.5 backdrop-blur-sm">
          <div className="flex flex-col xl:flex-row items-center justify-between gap-3 text-xs">
            {/* Step 1: Codex Client */}
            <div className="flex items-center gap-2.5 w-full xl:w-auto p-2 rounded-lg bg-background/70 border border-border/50 shadow-sm">
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-zinc-800 text-white dark:bg-zinc-200 dark:text-zinc-900 font-mono text-[10px] font-bold">
                CLI
              </div>
              <div className="min-w-0">
                <div className="font-medium text-foreground">OpenAI Codex</div>
                <div className="text-[11px] text-muted-foreground">~/.codex/config.toml</div>
              </div>
            </div>

            {/* Arrow & Connection */}
            <div className="hidden xl:flex flex-col items-center justify-center px-2">
              <div className="flex items-center gap-1 text-[11px] font-medium text-muted-foreground/80">
                <span className={cn("h-1.5 w-1.5 rounded-full", routingEnabled ? "bg-emerald-500 animate-pulse motion-reduce:animate-none" : "bg-muted-foreground/40")} />
                {t(routingEnabled ? (nativeAccess ? "newapi.directConnection" : "newapi.gatewayConnection") : "newapi.connectionDisabled")}
              </div>
              <ArrowRight className={cn("h-3.5 w-3.5", routingEnabled ? "text-primary" : "text-muted-foreground/40")} />
            </div>

            {/* Step 2: Local gateway */}
            {!nativeAccess && <div className="flex min-w-0 items-center gap-2.5 w-full xl:w-auto p-2 rounded-lg bg-background/70 border border-border/50 shadow-sm">
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-primary/15 text-primary">
                <Network className="h-3.5 w-3.5" />
              </div>
              <div className="min-w-0">
                <div className="font-medium text-foreground">
                  {t("newapi.gatewayAddress", { port: router?.port ?? 43127 })}
                </div>
                <div className="text-[11px] text-muted-foreground font-mono">
                  /codex/v1/responses
                </div>
              </div>
            </div>}

            {/* Arrow */}
            {!nativeAccess && <div className="hidden xl:flex flex-col items-center justify-center px-2">
              <ArrowRight className={cn("h-3.5 w-3.5", routingEnabled ? "text-primary" : "text-muted-foreground/40")} />
            </div>}

            {/* Step 3: Target Model */}
            <div className="flex min-w-0 items-center gap-2.5 w-full xl:w-auto p-2 rounded-lg bg-background/70 border border-border/50 shadow-sm">
              <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-purple-500/15 text-purple-600 dark:text-purple-400 text-[10px] font-bold">
                <Cpu className="h-3.5 w-3.5" />
              </div>
              <div className="min-w-0 flex-1 xl:max-w-[200px]">
                <div className="font-medium text-foreground truncate" title={targetName}>
                  {targetName}
                </div>
                <div className="text-[11px] text-muted-foreground truncate font-mono" title={targetModel}>
                  {targetModel}
                </div>
                {nativeAccess && selectedProvider && (
                  <div className="truncate font-mono text-[11px] text-muted-foreground" title={selectedProvider.baseUrl}>
                    {selectedProvider.baseUrl}
                  </div>
                )}
              </div>
              <Badge variant="outline" className="ml-auto text-[10px] px-1.5 py-0 shrink-0 font-normal">
                {targetProtocol}
              </Badge>
            </div>
          </div>
        </div>

        {/* Configuration settings block */}
        <div className="space-y-4 rounded-xl border border-border/60 bg-muted/20 p-4 backdrop-blur-sm">
          <div className="space-y-2">
            <Label htmlFor="codex-access-mode" className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("newapi.accessMode")}
            </Label>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
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
                <SelectTrigger id="codex-access-mode" className="w-full sm:max-w-xs bg-background/80 shadow-sm">
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
                <div className="min-w-0 text-xs text-muted-foreground truncate font-mono bg-background/50 px-2.5 py-1 rounded-md border border-border/40">
                  {t("newapi.accessSelectedModel", {
                    provider: selectedProvider.name,
                    model: selectedProvider.model,
                    protocol: t(protocolBadgeKey[providerModelProtocol(selectedProvider)]),
                  })}
                </div>
              )}
            </div>
            {!nativeCompatible && (
              <p className="text-xs text-amber-700 dark:text-amber-300">
                {t(selectedProvider ? "newapi.nativeProtocolHint" : "newapi.nativeProviderHint")}
              </p>
            )}
            {nativeAccess && <p className="text-xs leading-relaxed text-muted-foreground">{t("newapi.nativeAuthHint")}</p>}
            <p className="text-xs leading-relaxed text-muted-foreground">
              {t(nativeAccess ? "newapi.nativeNetworkHint" : "newapi.routerNetworkHint")}
            </p>
          </div>

          {!nativeAccess && (
            <div className="pt-2 border-t border-border/40 space-y-1.5">
              <Label htmlFor="codex-aggregate-model" className="text-xs font-medium text-foreground">
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
                <SelectTrigger id="codex-aggregate-model" className="bg-background/80 shadow-sm">
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

          <p className="text-xs leading-relaxed text-muted-foreground">
            {t(routingEnabled ? (nativeAccess ? "newapi.nativeOnHint" : "newapi.routingOnHint") : "newapi.routingOffHint")}
          </p>

          {configDrifted && (
            <p
              role="alert"
              className="rounded-lg border border-amber-500/30 bg-amber-500/[0.07] px-3 py-2 text-[13px] leading-relaxed text-amber-700 dark:text-amber-300"
            >
              {t("newapi.configDriftedHint")}
            </p>
          )}

          {appliedNote && (
            <p className="break-all font-mono text-[11px] text-muted-foreground/80 bg-background/50 px-2 py-1 rounded border border-border/40">
              {appliedNote.path}
              {appliedNote.backupPath ? ` · ${t("newapi.backup")}: ${appliedNote.backupPath}` : ""}
            </p>
          )}

          {!nativeAccess && (
            <div className="flex items-center justify-between gap-4 rounded-xl bg-card/60 p-3.5 ring-1 ring-border/60 transition-colors hover:bg-card/90">
              <div className="min-w-0">
                <div className="flex items-center gap-1.5">
                  <Settings2 className="h-3.5 w-3.5 text-muted-foreground" />
                  <span className="text-sm font-semibold">{t("newapi.accountless")}</span>
                  <span title={t("newapi.accountlessTip")}>
                    <CircleHelp className="h-4 w-4 text-muted-foreground hover:text-foreground cursor-help transition-colors" />
                  </span>
                </div>
                <p className="mt-0.5 text-xs text-muted-foreground">{t("newapi.accountlessHint")}</p>
              </div>
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
        </div>
      </div>
    </AuroraCard>
  );
}
