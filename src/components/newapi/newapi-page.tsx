import { useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Layers3, Plug, Share2, X } from "lucide-react";
import { api } from "@/lib/api";
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
import { ClientConsole } from "./client-console";
import { WebSearchSettings } from "./web-search-settings";
import { PortOwnerNotice } from "./port-owner-notice";
import { useClientLaunch } from "./client-launch";

// Section opener: an English kicker, the title and a hairline to the edge.
function SectionGroup({ kicker, icon, title, description, delay, children }: {
  kicker: string;
  icon: ReactNode;
  title: string;
  description: string;
  delay: number;
  children: ReactNode;
}) {
  return (
    <div className="route-rise space-y-4" style={{ animationDelay: `${delay}ms` } as CSSProperties}>
      <div className="flex items-end gap-3.5 px-1">
        <div className="wallpaper-plate min-w-0 pb-0.5">
          <p className="poster-kicker flex items-center gap-1.5 text-[10px] text-primary"><span aria-hidden>{icon}</span>{kicker}</p>
          <h2 className="font-display text-lg font-semibold tracking-tight text-foreground">{title}</h2>
          <p className="text-xs text-muted-foreground">{description}</p>
        </div>
        <span aria-hidden className="route-group-rule mb-2 ml-2 h-px flex-1" />
      </div>
      {children}
    </div>
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

      <SectionGroup kicker="Clients" icon={<Plug className="h-3 w-3" />} title={t("newapi.groups.clients")}
        description={t("newapi.groups.clientsDesc")} delay={80}>
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

      <SectionGroup kicker="Gateway" icon={<Share2 className="h-3 w-3" />} title={t("gateway.groupTitle")}
        description={t("gateway.groupDesc")} delay={120}>
        <GatewayPanel providersRevision={state.dataUpdatedAt} />
      </SectionGroup>

      <SectionGroup kicker="Routing" icon={<Layers3 className="h-3 w-3" />} title={t("newapi.groups.routing")}
        description={t("newapi.groups.routingDesc")} delay={160}>
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
