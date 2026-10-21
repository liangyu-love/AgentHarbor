import { useEffect, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Layers3, Plug, Server, Share2, X } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { aggregationQueryKey } from "@/types/aggregation";
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
import { InjectTabs, type InjectTab } from "./inject-tabs";
import { CLIENTS, useRoutingLinks } from "./use-routing-links";
import { WebSearchSettings } from "./web-search-settings";
import { RemoteHostsPanel } from "./remote-hosts-panel";
import { PortOwnerNotice } from "./port-owner-notice";
import { useClientLaunch } from "./client-launch";

// Which tab holds the section a shortcut unfolds (see configuration-shortcuts).
const tabForFold: Record<string, InjectTabId> = {
  "codex-access": "clients",
  "client-configs": "clients",
  "claude-desktop": "clients",
  "gateway-models": "gateway",
  "gateway-export": "gateway",
  aggregation: "routing",
};

type InjectTabId = "clients" | "gateway" | "routing" | "remote";

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
  // What each group's tab says about it.
  const { links } = useRoutingLinks(router);
  const gateway = useQuery({ queryKey: queryKeys.gateway, queryFn: api.gateway.state, refetchOnWindowFocus: false });
  const aggregates = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const remoteHosts = useQuery({ queryKey: queryKeys.remoteHosts, queryFn: api.remote.hosts });
  const routed = CLIENTS.filter(client => links[client] === "routed").length;
  const direct = CLIENTS.filter(client => links[client] === "direct").length;
  // One group at a time. A shortcut that lands in another group switches to it;
  // hidden panels stay mounted so unsaved edits survive.
  const [tab, setTab] = useState<InjectTabId>("clients");
  useEffect(() => {
    const toClients = () => setTab("clients");
    const toFold = (event: Event) => {
      const next = tabForFold[(event as CustomEvent<string>).detail];
      if (next) setTab(next);
    };
    window.addEventListener(SELECT_CLIENT_EVENT, toClients);
    window.addEventListener("agentharbor:expand-section", toFold);
    return () => {
      window.removeEventListener(SELECT_CLIENT_EVENT, toClients);
      window.removeEventListener("agentharbor:expand-section", toFold);
    };
  }, []);
  const tabs: InjectTab<InjectTabId>[] = [
    { id: "clients", icon: <Plug />, title: t("newapi.groups.clients"), description: t("newapi.groups.clientsDesc"),
      summary: routed + direct ? t("newapi.groups.clientsSummary", { routed, direct }) : t("newapi.groups.clientsNone") },
    { id: "gateway", icon: <Share2 />, title: t("gateway.groupTitle"), description: t("gateway.groupDesc"),
      summary: gateway.data && t("newapi.groups.gatewaySummary", {
        state: t(gateway.data.running ? "gateway.running" : "gateway.stopped"), keys: gateway.data.keys.length }) },
    { id: "routing", icon: <Layers3 />, title: t("newapi.groups.routing"), description: t("newapi.groups.routingDesc"),
      summary: aggregates.data && t("newapi.groups.routingSummary", { count: aggregates.data.models.length }) },
    { id: "remote", icon: <Server />, title: t("remote.groupTitle"), description: t("remote.groupDesc"),
      summary: remoteHosts.data && (remoteHosts.data.length ? t("remote.groupSummary", { count: remoteHosts.data.length }) : t("remote.groupNone")) },
  ];
  const panel = (id: InjectTabId) => ({
    role: "tabpanel", id: `group-panel-${id}`, "aria-labelledby": `group-tab-${id}`,
    hidden: tab !== id, "data-inject-section": id, className: "group-panel space-y-4",
  });

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

      <div className="route-rise space-y-5" style={{ animationDelay: "80ms" } as CSSProperties}>
      <InjectTabs tabs={tabs} active={tab} onChange={setTab} label={t("newapi.groups.label")} />

      <section {...panel("clients")}>
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
      </section>

      <section {...panel("gateway")}>
        <GatewayPanel providersRevision={state.dataUpdatedAt} />
      </section>

      <section {...panel("remote")}>
        <RemoteHostsPanel providers={providers} />
      </section>

      <section {...panel("routing")}>
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
      </section>
      </div>

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
