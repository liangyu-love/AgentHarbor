import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Layers3, Plug, Share2, X } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { aggregationQueryKey } from "@/types/aggregation";
import { useFold } from "@/hooks/use-fold";
import { OWN_CLICK } from "@/components/ui/fold-header";
import { toast } from "@/hooks/use-toast";
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
import { ClientConfigs } from "@/components/newapi/client-configs";
import { ClaudeDesktopConfigCard } from "@/components/newapi/claude-desktop-config-card";
import { CodexAccessCard } from "@/components/newapi/codex-access-card";
import { CursorAccessCard } from "@/components/newapi/cursor-access-card";
import { RoutingDiagnostics } from "@/components/newapi/routing-diagnostics";
import { AggregationManager } from "@/components/newapi/aggregation-manager";
import { GatewayPanel } from "@/components/newapi/gateway-panel";
import { useProviderState } from "@/components/newapi/use-provider-state";
import { providerModelProtocol } from "@/lib/provider-models";
import { formatInvokeError } from "@/lib/invoke-error";
import { RoutingTopology } from "./routing-topology";
import { ClientConsole, SELECT_CLIENT_EVENT } from "./client-console";
import { CLIENTS, useRoutingLinks } from "./use-routing-links";
import { WebSearchSettings } from "./web-search-settings";
import { PortOwnerNotice } from "./port-owner-notice";
import { useClientLaunch } from "./client-launch";

// A group opens on a poster band: a full-width rule, the title large on the
// left, the English kicker and a live summary on the right. Clicking the band
// folds the group (no chevron: the title is the real button); the summary
// stays readable while it is folded. Folded content stays mounted, so unsaved
// edits survive and shortcuts can still reach it.
function SectionGroup({ id, kicker, icon, title, description, summary, delay, expandOn, children }: {
  id: string;
  kicker: string;
  icon: ReactNode;
  title: string;
  description: string;
  summary?: ReactNode;
  delay: number;
  /** A window event that unfolds the group, for shortcuts that land inside it. */
  expandOn?: string;
  children: ReactNode;
}) {
  const { t } = useTranslation();
  const foldId = `inject-section-${id}`;
  const fold = useFold(foldId);
  const titleId = `${foldId}-title`;
  const bodyId = `${foldId}-body`;
  useEffect(() => {
    if (!expandOn) return;
    const expand = () => window.dispatchEvent(new CustomEvent("agentharbor:expand-section", { detail: foldId }));
    window.addEventListener(expandOn, expand);
    return () => window.removeEventListener(expandOn, expand);
  }, [expandOn, foldId]);
  return (
    <section className="route-rise space-y-4" style={{ animationDelay: `${delay}ms` } as CSSProperties}
      aria-labelledby={titleId} data-inject-section={id}>
      <header className="section-band" data-folded={!fold.open}
        onClick={event => { if (!(event.target as HTMLElement).closest(OWN_CLICK)) fold.toggle(); }}>
        <div aria-hidden className="section-band-rule" />
        <div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-2 px-1 pt-4">
          <div className="wallpaper-plate min-w-0">
            <h2 id={titleId} className="font-display text-[1.6rem] font-semibold leading-tight tracking-tight text-foreground">
              <button type="button" aria-expanded={fold.open} aria-controls={bodyId} onClick={fold.toggle}
                className="section-band-title rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2">
                {title}
              </button>
            </h2>
            <p className="mt-1 text-xs text-muted-foreground">{description}</p>
          </div>
          <div className="wallpaper-plate flex min-w-0 flex-col items-end gap-1 text-right">
            <p className="poster-kicker flex items-center gap-1.5 text-[10px] text-primary"><span aria-hidden>{icon}</span>{kicker}</p>
            <p className="flex items-center gap-2 text-[11px] tabular-nums text-muted-foreground" data-section-summary>
              {!fold.open && <span className="rounded-full border border-border/80 px-1.5 py-px text-[10px]">{t("newapi.groups.folded")}</span>}
              {summary}
            </p>
          </div>
        </div>
      </header>
      <div id={bodyId} hidden={!fold.open} className="section-band-body space-y-4">{children}</div>
    </section>
  );
}

// Injection page: how Codex, Claude Code, Claude Desktop, OpenCode, Pi, Cursor and the aggregation
// gateway reach the relays. The relays themselves live on their own page.
export function NewApiPage({ onOpenRelays }: { onOpenRelays?: () => void }) {
  const { t } = useTranslation();
  const store = useProviderState();
  const { state, run, error, setError, applyChange } = store;
  const [restartConfirmOpen, setRestartConfirmOpen] = useState(false);
  const [restartPromptOpen, setRestartPromptOpen] = useState(false);
  const codexLaunch = useClientLaunch("codex");
  // A Codex that is not running is simply started; a running one is confirmed first.
  const askRestart = () => codexLaunch.data?.kind === "app" && !codexLaunch.data.running ? restartCodex() : setRestartConfirmOpen(true);

  const providers = state.data?.providers ?? [];
  const router = state.data?.router;
  const nativeAccess = (router?.accessMode ?? "router") === "native";
  const activeId = state.data?.activeProviderId ?? null;
  const selectedProvider = activeId
    ? providers.find((provider) => provider.id === activeId)
    : providers.find((provider) => provider.enabled && provider.hasApiKey && provider.model);
  const nativeCompatible = !!selectedProvider && providerModelProtocol(selectedProvider) === "responses";
  // What each group's band says while it is folded.
  const { links } = useRoutingLinks(router);
  const gateway = useQuery({ queryKey: queryKeys.gateway, queryFn: api.gateway.state, refetchOnWindowFocus: false });
  const aggregates = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const routed = CLIENTS.filter(client => links[client] === "routed").length;
  const direct = CLIENTS.filter(client => links[client] === "direct").length;

  const restartCodex = () => {
    setRestartConfirmOpen(false);
    setRestartPromptOpen(false);
    void run(
      "restart-codex",
      async () => {
        await api.targets.restart("codex");
        toast({ title: t("newapi.restartCodexSuccess"), variant: "success" });
        void state.refetch();
        void codexLaunch.refetch();
      },
      t("newapi.restartCodexFailed"),
    );
  };

  return (
    <div className="space-y-7">
      <RoutingTopology store={store} onOpenRelays={onOpenRelays} onRestart={askRestart} />

      <PortOwnerNotice routerRunning={state.data?.proxyRunning} />

      {(error || state.error) && (
        <div
          role="alert"
          className="flex items-start justify-between gap-3 rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
        >
          <span>{error ? t(error) : formatInvokeError(state.error)}</span>
          <button
            type="button"
            onClick={() => {
              setError(null);
              void state.refetch();
            }}
            aria-label={t("common.cancel")}
            className="shrink-0 rounded p-0.5 transition-colors hover:bg-destructive/10"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      <SectionGroup id="clients" kicker="Clients" icon={<Plug className="h-3 w-3" />} title={t("newapi.groups.clients")}
        description={t("newapi.groups.clientsDesc")} delay={80} expandOn={SELECT_CLIENT_EVENT}
        summary={routed + direct ? t("newapi.groups.clientsSummary", { routed, direct }) : t("newapi.groups.clientsNone")}>
        <ClientConsole
          router={router}
          panels={{
            codex: (
              <CodexAccessCard
                store={store}
                providers={providers}
                selectedProvider={selectedProvider}
                nativeCompatible={nativeCompatible}
                onRestart={askRestart}
                onRestartRequired={() => setRestartPromptOpen(true)}
                onOpenRelays={onOpenRelays}
              />
            ),
            claudeCode: <ClientConfigs target="claudeCode" providers={providers} onOpenRelays={onOpenRelays} />,
            claudeDesktop: <ClaudeDesktopConfigCard providers={providers} />,
            opencode: <ClientConfigs target="opencode" providers={providers} onOpenRelays={onOpenRelays} />,
            pi: <ClientConfigs target="pi" providers={providers} onOpenRelays={onOpenRelays} />,
            harness: <ClientConfigs target="harness" providers={providers} onOpenRelays={onOpenRelays} />,
            cursor: <CursorAccessCard onOpenRelays={onOpenRelays} />,
          }}
          shared={<WebSearchSettings store={store} providers={providers} />}
        />
      </SectionGroup>

      <SectionGroup id="gateway" kicker="Gateway" icon={<Share2 className="h-3 w-3" />} title={t("gateway.groupTitle")}
        description={t("gateway.groupDesc")} delay={120}
        summary={gateway.data && t("newapi.groups.gatewaySummary", {
          state: t(gateway.data.running ? "gateway.running" : "gateway.stopped"), keys: gateway.data.keys.length })}>
        <GatewayPanel providersRevision={state.dataUpdatedAt} />
      </SectionGroup>

      <SectionGroup id="routing" kicker="Routing" icon={<Layers3 className="h-3 w-3" />} title={t("newapi.groups.routing")}
        description={t("newapi.groups.routingDesc")} delay={160}
        summary={aggregates.data && t("newapi.groups.routingSummary", { count: aggregates.data.models.length })}>
        <AggregationManager providers={providers} />
        <RoutingDiagnostics
          disabled={nativeAccess || !state.isSuccess || store.mutationLocked}
          compression={state.data?.compression ?? { mode: "percent", percent: 90 }}
          onCompression={(policy) => {
            void run(
              "compression",
              async () => applyChange(await api.routing.setCompression(policy)),
              t("newapi.compression.saveFailed"),
            );
          }}
        />
      </SectionGroup>

      <AlertDialog open={restartConfirmOpen} onOpenChange={setRestartConfirmOpen}>
        <AlertDialogContent className="max-w-sm">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("newapi.restartConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("newapi.restartConfirmDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction className="bg-destructive text-white hover:bg-destructive/90" onClick={restartCodex}>
              {t("newapi.restartCodex")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* After a connection change while Codex is running. */}
      <AlertDialog open={restartPromptOpen} onOpenChange={setRestartPromptOpen}>
        <AlertDialogContent className="max-w-sm">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("newapi.restartPromptTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("newapi.restartPromptDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("newapi.restartLater")}</AlertDialogCancel>
            <AlertDialogAction onClick={restartCodex}>{t("newapi.restartNow")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
