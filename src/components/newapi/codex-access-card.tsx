import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { aggregationQueryKey } from "@/types/aggregation";
import { ArrowDown, ArrowRight, Cpu, Loader2, Network, Plus, RefreshCw, UserX, Waypoints } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { toast } from "@/hooks/use-toast";
import { providerModelProtocol } from "@/lib/provider-models";
import { protocolBadgeKey, softBadge } from "@/components/newapi/relay-groups";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { CodexAccessMode, NewApiProvider, NewApiStateChange } from "@/types/newapi";
import { cn } from "@/lib/utils";
import { ClientGlyph } from "./configuration-shortcuts";
import { CodexSubagentSettingsFields } from "./codex-subagent-settings";
import { ModelSourceSwitches } from "./model-source-switches";
import type { ModelSources } from "@/types/gateway";
import { useClientLaunch } from "./client-launch";
import { ConsoleHeader } from "./client-console";

function Panel({ title, children, className }: { title: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn("harbor-well min-w-0 p-4", className)}>
      <h3 className="mb-3 text-[11px] font-semibold uppercase tracking-[0.08em] text-muted-foreground">{title}</h3>
      {children}
    </div>
  );
}

// Codex routing switch, access mode picker and accountless toggle.
export function CodexAccessCard({
  store,
  providers,
  selectedProvider,
  nativeCompatible,
  onRestart,
  onRestartRequired,
  onOpenRelays,
}: {
  store: ProviderState;
  providers: NewApiProvider[];
  selectedProvider: NewApiProvider | undefined;
  nativeCompatible: boolean;
  onRestart: () => void;
  /** Codex is running with its previous connection; offer a restart. */
  onRestartRequired: () => void;
  onOpenRelays?: () => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { busy, mutationLocked, run, applyChange, appliedNote } = store;
  const router = store.state.data?.router;
  const routingEnabled = !!router?.enabled;
  const accessMode = router?.accessMode ?? "router";
  const nativeAccess = accessMode === "native";
  const configDrifted = !!router?.configDrifted;
  const restartPending = !!router?.restartPending;
  const codexLaunch = useClientLaunch("codex");
  const codexRunning = codexLaunch.data?.kind !== "app" || codexLaunch.data.running;
  const listenerUnavailable = routingEnabled && !nativeAccess && !store.state.data?.proxyRunning;
  const routingReady = routingEnabled && !configDrifted && !listenerUnavailable;
  // The desktop app lists only the first page (100) of Codex's model catalog.
  const menu = useQuery({
    queryKey: ["codex-menu-overflow", store.state.dataUpdatedAt],
    queryFn: api.codexMenuOverflow,
    enabled: routingEnabled && !nativeAccess,
    retry: false,
  });
  const menuCut = menu.data && menu.data.cut.length > 0 ? menu.data : null;
  const aggregates = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const aggregate = aggregates.data?.models.find((model) => model.id === router?.aggregateModelId);
  const aggregateSelected = router?.aggregateModelId != null;
  const aggregateAvailable = !!aggregate?.available;
  // Builds before the switches: relays alone, or aggregates alone once one was chosen.
  const sources: ModelSources = router?.codexSources
    ?? { relay: !aggregateSelected, account: !aggregateSelected, aggregate: aggregateSelected };
  const availableAggregates = (aggregates.data?.models ?? []).filter(model => model.available);
  const sourceCounts = {
    relay: providers.filter(provider => !provider.accountSource).length,
    account: providers.filter(provider => !!provider.accountSource).length,
    aggregate: availableAggregates.length,
  };
  const relayOffered = (sources.relay && sourceCounts.relay > 0) || (sources.account && sourceCounts.account > 0);
  const routerReachable = aggregateSelected ? aggregateAvailable
    : relayOffered || (sources.aggregate && availableAggregates.length > 0);

  // Threads keep their provider, so a connection change only rewrites
  // config.toml. A running Codex picks it up when the user restarts it.
  const connectionChanged = (change: NewApiStateChange, title: string) => {
    applyChange(change);
    void queryClient.invalidateQueries({ queryKey: ["threads"] });
    if (change.restartRequired) {
      onRestartRequired();
      return;
    }
    toast({ title, description: t("newapi.codexRestartHint"), variant: "success" });
  };

  const toggleRouting = (value: boolean) =>
    run(
      "routing",
      async () => {
        const change = await api.routing.setEnabled(value);
        connectionChanged(change, t(value ? "newapi.routingApplied" : "newapi.routingRemoved"));
      },
      t("newapi.routingFailed"),
    );

  const changeSources = (next: ModelSources) =>
    run(
      "codex-sources",
      async () => {
        const change = await api.aggregation.setCodexSources(next);
        connectionChanged(change, t("newapi.codexSourcesApplied"));
      },
      t("newapi.routingFailed"),
    );

  const changeAccessMode = (mode: CodexAccessMode) =>
    run(
      "access-mode",
      async () => {
        const change = await api.routing.setAccessMode(mode);
        connectionChanged(change, t("newapi.accessModeApplied"));
      },
      t("newapi.routingFailed"),
    );

  // The router target represents a catalog. A conversation's selected model
  // can change independently of the app's pinned default provider.
  const offered = (["relay", "account", "aggregate"] as const).filter(kind => sources[kind]);
  const targetName = nativeAccess
    ? selectedProvider?.name ?? t("clients.notConfigured")
    : offered.length ? offered.map(kind => t(`gateway.kind.${kind}`)).join(" + ") : t("newapi.codexSourcesNone");
  const targetModel = selectedProvider?.model ?? "—";
  const targetProtocol = selectedProvider
    ? t(protocolBadgeKey[providerModelProtocol(selectedProvider)])
    : "—";
  const TargetIcon = !nativeAccess && sources.aggregate ? Network : Cpu;

  const step = "flex min-w-0 items-center gap-3 rounded-xl border bg-card/90 p-2.5 lg:flex-1";
  const stepIcon = (lit: boolean) => cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ring-1 transition-colors",
    lit ? "bg-primary/10 text-primary ring-primary/25" : "bg-muted text-muted-foreground ring-border/70");
  // Stacked on narrow screens, a left-to-right chain on wide ones.
  const connector = (
    <div aria-hidden className="flex shrink-0 justify-center py-1 lg:items-center lg:px-1 lg:py-0">
      <ArrowDown className={cn("h-3.5 w-3.5 transition-colors lg:-rotate-90", routingReady ? "text-primary" : "text-muted-foreground/40")} />
    </div>
  );
  // Spans only: the node may be rendered inside a button.
  const targetNode = (
    <>
      <span className={stepIcon(routingReady)}>
        <TargetIcon className="h-4 w-4" />
      </span>
      <span className="block min-w-0 flex-1">
        <span className="block truncate font-medium text-foreground" title={targetName}>
          {targetName}
        </span>
        {nativeAccess && (
          <span className="block truncate font-mono text-[11px] text-muted-foreground" title={targetModel}>
            {targetModel}
          </span>
        )}
        {nativeAccess && selectedProvider && (
          <span className="block truncate font-mono text-[11px] text-muted-foreground" title={selectedProvider.baseUrl}>
            {selectedProvider.baseUrl}
          </span>
        )}
      </span>
      {nativeAccess && (
        <Badge variant="outline" className="ml-auto shrink-0 px-1.5 py-0 text-[10px] font-normal">
          {targetProtocol}
        </Badge>
      )}
    </>
  );

  const status = configDrifted ? (
    <Badge className={cn(softBadge("amber"), "gap-1 font-medium")}>
      <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
      {t("newapi.driftedBadge")}
    </Badge>
  ) : listenerUnavailable ? (
    <Badge className={cn(softBadge("amber"), "gap-1 font-medium")}>
      <span className="h-1.5 w-1.5 rounded-full bg-amber-500" />
      {t("newapi.listenerUnavailableBadge")}
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
  );

  return (
    <div className="relative">
      <section aria-labelledby="newapi-routing-title" className="space-y-5">
        <ConsoleHeader
          titleId="newapi-routing-title"
          title={t("newapi.codexAccess")}
          leading={
            <div className="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-background ring-1 ring-border">
              <ClientGlyph client="codex" className="h-5 w-5" />
              {routingReady && (
                <span aria-hidden className="absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-emerald-500 ring-2 ring-card" />
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
              {status}
            </>
          }
          actions={
            <>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className={cn("h-8 gap-1.5 rounded-xl px-3 text-xs font-medium",
                  restartPending && "border-amber-500/50 text-amber-700 hover:bg-amber-500/10 dark:text-amber-300")}
                disabled={!!busy}
                onClick={onRestart}
              >
                {busy === "restart-codex" ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <RefreshCw className="h-3.5 w-3.5" />
                )}
                <span>{t(busy === "restart-codex" ? (codexRunning ? "newapi.restartingCodex" : "newapi.startingCodex")
                  : codexRunning ? "newapi.restartCodex" : "newapi.startCodex")}</span>
              </Button>
              <Switch
                aria-label={t("newapi.codexAccess")}
                checked={routingEnabled}
                disabled={
                  mutationLocked ||
                  (!routingEnabled &&
                    (nativeAccess ? providers.length === 0 || !nativeCompatible : !routerReachable))
                }
                onCheckedChange={(value) => void toggleRouting(value)}
              />
            </>
          }
        />

        {restartPending && (
          <div role="status" className="flex flex-wrap items-center gap-3 rounded-2xl border border-amber-500/30 bg-amber-500/[0.07] px-3.5 py-2.5 text-[13px]">
            <span aria-hidden className="route-pending-dot h-2 w-2 shrink-0 rounded-full bg-amber-500" />
            <span className="min-w-0 flex-1 text-amber-800 dark:text-amber-200">{t("newapi.restartPendingHint")}</span>
            <Button type="button" size="sm" variant="outline" className="h-7 rounded-lg border-amber-500/40 px-2.5 text-xs"
              disabled={!!busy} onClick={onRestart}>
              {t("newapi.restartNow")}
            </Button>
          </div>
        )}

        {menuCut && (
          <p role="status" data-codex-menu-overflow className="rounded-2xl border border-amber-500/30 bg-amber-500/[0.07] px-3.5 py-2.5 text-[13px] leading-relaxed text-amber-800 dark:text-amber-200">
            {t("newapi.codexMenuOverflow", {
              limit: menuCut.limit, total: menuCut.total, count: menuCut.cut.length,
              names: menuCut.cut.length > 6
                ? t("newapi.codexMenuOverflowMore", { names: menuCut.cut.slice(0, 6).join("、") })
                : menuCut.cut.join("、"),
            })}
          </p>
        )}

        <div id="newapi-routing-body" className="space-y-4">
          {/* Settings side by side, the resulting route underneath. */}
          <div className="grid items-stretch gap-4 lg:grid-cols-2">
            <Panel title={t("newapi.connectionPanel")}>
              <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1 xl:grid-cols-2">
                <div className="min-w-0 space-y-1.5">
                  <Label htmlFor="codex-access-mode" className="text-xs font-medium text-muted-foreground">
                    {t("newapi.accessMode")}
                  </Label>
                  <Select
                    value={accessMode}
                    disabled={mutationLocked}
                    onValueChange={(value) => void changeAccessMode(value as CodexAccessMode)}
                  >
                    <SelectTrigger id="codex-access-mode" className="bg-card">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="router">{t("newapi.accessRouter")}</SelectItem>
                      <SelectItem value="native" disabled={!nativeCompatible || !!router?.aggregateModelId}>
                        {t("newapi.accessNative")}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                  {nativeAccess && selectedProvider && (
                    <p className="truncate font-mono text-[11px] text-muted-foreground">
                      {t("newapi.accessSelectedModel", {
                        provider: selectedProvider.name,
                        model: selectedProvider.model,
                        protocol: t(protocolBadgeKey[providerModelProtocol(selectedProvider)]),
                      })}
                    </p>
                  )}
                </div>

                {!nativeAccess && sources.aggregate && (
                  <div className="min-w-0 space-y-1.5">
                    <Label htmlFor="codex-aggregate-model" className="text-xs font-medium text-muted-foreground">
                      {t("aggregation.codexDefault")}
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
                      <SelectTrigger id="codex-aggregate-model" className="bg-card">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__single__" disabled={!relayOffered}>{t("aggregation.followRelay")}</SelectItem>
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
                <div className="mt-3 space-y-1.5" data-codex-sources>
                  <p className="text-xs font-medium text-muted-foreground">{t("newapi.codexSources")}</p>
                  <ModelSourceSwitches value={sources} counts={sourceCounts} disabled={mutationLocked}
                    label={t("newapi.codexSources")} onChange={next => void changeSources(next)} />
                  <p className="text-[11px] leading-relaxed text-muted-foreground">{t("newapi.codexSourcesHint")}</p>
                </div>
              )}

              {!nativeAccess && (
                <div className="mt-3 flex items-center justify-between gap-4 rounded-xl border border-border/60 bg-card px-3.5 py-2.5">
                  <span className="flex min-w-0 items-center gap-2.5">
                    <UserX aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="min-w-0">
                      <span className="block text-sm font-medium">{t("newapi.accountless")}</span>
                      <span className="block text-[11px] text-muted-foreground">{t("newapi.accountlessHint")}</span>
                    </span>
                  </span>
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

              {nativeAccess && !nativeCompatible && (
                <p className="mt-3 text-xs text-amber-700 dark:text-amber-300">
                  {t(selectedProvider ? "newapi.nativeProtocolHint" : "newapi.nativeProviderHint")}
                </p>
              )}
              {nativeAccess && <p className="mt-3 text-xs text-muted-foreground">{t("newapi.nativeAuthHint")}</p>}
              {providers.length === 0 && onOpenRelays && (
                <Button type="button" variant="outline" size="sm" className="mt-3 h-8 gap-1.5 text-xs" onClick={onOpenRelays}>
                  <Plus className="h-3.5 w-3.5" />
                  {t("newapi.addProvider")}
                </Button>
              )}
            </Panel>

            <CodexSubagentSettingsFields store={store} />
          </div>

          {/* Pipeline: Codex -> gateway -> target */}
          <Panel title={t("newapi.pipelinePanel")} className="text-xs">
            <div data-codex-pipeline className="flex flex-col lg:flex-row lg:items-stretch">
              <div className={cn(step, "border-border/60")}>
                <span className={stepIcon(true)}>
                  <ClientGlyph client="codex" className="h-4 w-4" />
                </span>
                <span className="min-w-0">
                  <span className="block font-medium text-foreground">OpenAI Codex</span>
                  <span className="block truncate font-mono text-[11px] text-muted-foreground">~/.codex/config.toml</span>
                </span>
              </div>
              {!nativeAccess && (
                <>
                  {connector}
                  <div className={cn(step, routingReady ? "border-primary/35" : "border-border/60")}>
                    <span className={stepIcon(routingReady)}>
                      <Waypoints className="h-4 w-4" />
                    </span>
                    <span className="min-w-0">
                      <span className="block font-medium text-foreground">
                        {t("newapi.gatewayAddress", { port: router?.port ?? 43127 })}
                      </span>
                      <span className="block truncate font-mono text-[11px] text-muted-foreground">openai_base_url</span>
                    </span>
                  </div>
                </>
              )}
              {connector}
              {onOpenRelays && (nativeAccess || relayOffered) ? (
                <button
                  type="button"
                  data-relay-link
                  data-codex-target={nativeAccess ? "native" : offered.join("+") || "none"}
                  title={t("newapi.providersSection")}
                  onClick={onOpenRelays}
                  className={cn(step, "group w-full border-border/60 text-left transition-colors hover:border-primary/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring")}
                >
                  {targetNode}
                </button>
              ) : (
                <div data-codex-target={nativeAccess ? "native" : offered.join("+") || "none"} className={cn(step, "border-border/60")}>
                  {targetNode}
                </div>
              )}
            </div>
            <p className="mt-3 flex items-start gap-1.5 text-[11px] leading-relaxed text-muted-foreground">
              <ArrowRight aria-hidden className="mt-0.5 h-3 w-3 shrink-0" />
              {t(nativeAccess ? "newapi.pipelineNativeHint" : "newapi.pipelineRouterHint")}
            </p>
          </Panel>

          {configDrifted && (
            <p
              role="alert"
              className="rounded-xl border border-amber-500/30 bg-amber-500/[0.07] px-3 py-2 text-[13px] leading-relaxed text-amber-700 dark:text-amber-300"
            >
              {t("newapi.configDriftedHint")}
            </p>
          )}

          {appliedNote && (
            <p className="break-all rounded-lg border border-border/40 bg-background/50 px-2 py-1 font-mono text-[11px] text-muted-foreground/80">
              {appliedNote.path}
              {appliedNote.backupPath ? ` · ${t("newapi.backup")}: ${appliedNote.backupPath}` : ""}
            </p>
          )}
        </div>
      </section>
    </div>
  );
}
