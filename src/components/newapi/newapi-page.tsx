import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
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
import { ConfigurationShortcuts } from "./configuration-shortcuts";

// Injection page: how Codex, Claude Code, Claude Desktop and the aggregation
// gateway reach the relays. The relays themselves live on their own page.
export function NewApiPage({ onOpenRelays }: { onOpenRelays?: () => void }) {
  const { t } = useTranslation();
  const store = useProviderState();
  const { state, run, error, setError, applyChange } = store;
  const [restartConfirmOpen, setRestartConfirmOpen] = useState(false);

  const providers = state.data?.providers ?? [];
  const router = state.data?.router;
  const nativeAccess = (router?.accessMode ?? "router") === "native";
  const activeId = state.data?.activeProviderId ?? null;
  const selectedProvider = activeId
    ? providers.find((provider) => provider.id === activeId)
    : providers.find((provider) => provider.enabled && provider.hasApiKey && provider.model);
  const nativeCompatible = !!selectedProvider && providerModelProtocol(selectedProvider) === "responses";

  return (
    <div className="space-y-5">
      <ConfigurationShortcuts />
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

      <CodexAccessCard
        store={store}
        providers={providers}
        selectedProvider={selectedProvider}
        nativeCompatible={nativeCompatible}
        onRestart={() => setRestartConfirmOpen(true)}
        onOpenRelays={onOpenRelays}
      />

      <ClientConfigs providers={providers} onOpenRelays={onOpenRelays} />

      <ClaudeDesktopConfigCard providers={providers} />

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

      <AlertDialog open={restartConfirmOpen} onOpenChange={setRestartConfirmOpen}>
        <AlertDialogContent className="max-w-sm">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("newapi.restartConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("newapi.restartConfirmDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-white hover:bg-destructive/90"
              onClick={() => {
                setRestartConfirmOpen(false);
                void run(
                  "restart-codex",
                  async () => {
                    await api.targets.restart("codex");
                    toast({ title: t("newapi.restartCodexSuccess"), variant: "success" });
                  },
                  t("newapi.restartCodexFailed"),
                );
              }}
            >
              {t("newapi.restartCodex")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
