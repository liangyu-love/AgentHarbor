import { useState, type CSSProperties, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Layers3, Plug, X } from "lucide-react";
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
import { RoutingDiagnostics } from "@/components/newapi/routing-diagnostics";
import { AggregationManager } from "@/components/newapi/aggregation-manager";
import { ModelCapabilities } from "@/components/newapi/model-capabilities";
import { useProviderState } from "@/components/newapi/use-provider-state";
import { providerModelProtocol } from "@/lib/provider-models";
import { formatInvokeError } from "@/lib/invoke-error";
import { RoutingTopology } from "./routing-topology";

// Poster-style section opener: an outlined index, an English kicker and the title.
function SectionGroup({ index, kicker, icon, title, description, delay, children }: {
  index: string;
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
        <span aria-hidden className="poster-index">{index}</span>
        <div className="min-w-0 pb-0.5">
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

// Injection page: how Codex, Claude Code, Claude Desktop and the aggregation
// gateway reach the relays. The relays themselves live on their own page.
export function NewApiPage({ onOpenRelays }: { onOpenRelays?: () => void }) {
  const { t } = useTranslation();
  const store = useProviderState();
  const { state, run, error, setError, applyChange } = store;
  const [restartConfirmOpen, setRestartConfirmOpen] = useState(false);
  const [restartPromptOpen, setRestartPromptOpen] = useState(false);

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
      },
      t("newapi.restartCodexFailed"),
    );
  };

  return (
    <div className="space-y-7">
      <RoutingTopology store={store} onOpenRelays={onOpenRelays} onRestart={() => setRestartConfirmOpen(true)} />

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

      <SectionGroup index="01" kicker="Clients" icon={<Plug className="h-3 w-3" />} title={t("newapi.groups.clients")}
        description={t("newapi.groups.clientsDesc")} delay={80}>
        <CodexAccessCard
          store={store}
          providers={providers}
          selectedProvider={selectedProvider}
          nativeCompatible={nativeCompatible}
          onRestart={() => setRestartConfirmOpen(true)}
          onRestartRequired={() => setRestartPromptOpen(true)}
          onOpenRelays={onOpenRelays}
        />
        <ClientConfigs providers={providers} onOpenRelays={onOpenRelays} />
        <ClaudeDesktopConfigCard providers={providers} />
      </SectionGroup>

      <SectionGroup index="02" kicker="Routing" icon={<Layers3 className="h-3 w-3" />} title={t("newapi.groups.routing")}
        description={t("newapi.groups.routingDesc")} delay={160}>
        <AggregationManager providers={providers} />
        <RoutingDiagnostics
          disabled={nativeAccess || !state.isSuccess || store.mutationLocked}
          webSearchProviderId={router?.webSearchProviderId ?? null}
          searchProviders={providers.filter((provider) => provider.webSearchCapable)}
          searchDisabled={!state.isSuccess || store.mutationLocked}
          onWebSearchProvider={(providerId) => {
            void run(
              "web-search-provider",
              async () => applyChange(await api.routing.setWebSearchProvider(providerId)),
              t("newapi.webSearchSaveFailed"),
            );
          }}
          compression={state.data?.compression ?? { mode: "percent", percent: 90 }}
          onCompression={(policy) => {
            void run(
              "compression",
              async () => applyChange(await api.routing.setCompression(policy)),
              t("newapi.compression.saveFailed"),
            );
          }}
        />
        <ModelCapabilities />
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
