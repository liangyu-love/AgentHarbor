import { useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { toast } from "@/hooks/use-toast";
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { ProviderBalanceDialog } from "@/components/newapi/provider-balance-dialog";
import { ProviderDialog } from "@/components/newapi/provider-dialog";
import { ProviderTestDialog } from "@/components/newapi/provider-test-dialog";
import { ClientConfigs } from "@/components/newapi/client-configs";
import { ClaudeDesktopConfigCard } from "@/components/newapi/claude-desktop-config-card";
import { CodexAccessCard } from "@/components/newapi/codex-access-card";
import { RoutingDiagnostics } from "@/components/newapi/routing-diagnostics";
import { ModelCapabilities } from "@/components/newapi/model-capabilities";
import { GroupAliasDialog } from "@/components/newapi/group-alias-dialog";
import { RelayList } from "@/components/newapi/relay-list";
import { useProviderState } from "@/components/newapi/use-provider-state";
import { useRelayView } from "@/components/newapi/use-relay-view";
import { providerModelProtocol } from "@/lib/provider-models";
import type { NewApiProvider } from "@/types/newapi";
import { formatInvokeError } from "@/lib/invoke-error";

export function NewApiPage() {
  const { t } = useTranslation();
  const store = useProviderState();
  const { state, run, error, setError, applyChange, patchProviderTest } = store;
  const [dialog, setDialog] = useState<{ open: boolean; provider: NewApiProvider | null }>({ open: false, provider: null });
  const [testDialog, setTestDialog] = useState<NewApiProvider | null>(null);
  const [balanceId, setBalanceId] = useState<string | null>(null);
  const [removing, setRemoving] = useState<NewApiProvider | null>(null);
  const [restartConfirmOpen, setRestartConfirmOpen] = useState(false);

  const providers = state.data?.providers ?? [];
  const router = state.data?.router;
  const nativeAccess = (router?.accessMode ?? "router") === "native";
  const activeId = state.data?.activeProviderId ?? null;
  const selectedProvider = activeId
    ? providers.find(provider => provider.id === activeId)
    : providers.find(provider => provider.enabled && provider.hasApiKey && provider.model);
  const nativeCompatible = !!selectedProvider && providerModelProtocol(selectedProvider) === "responses";
  const proxyRunning = !!state.data?.proxyRunning;
  const view = useRelayView(providers, state.isSuccess);

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-base text-foreground/90">{t("newapi.description")}</p>
        {nativeAccess ? <Badge variant="outline">{t("newapi.nativeNoRouter")}</Badge> : <div className="flex items-center gap-2.5 rounded-full border bg-card py-1.5 pl-4 pr-2 text-sm shadow-sm">
          <span className="relative flex h-2.5 w-2.5 items-center justify-center">
            {proxyRunning && (
              <span aria-hidden className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60 motion-reduce:hidden" />
            )}
            <span className={cn("relative inline-flex h-2.5 w-2.5 rounded-full", proxyRunning ? "bg-emerald-500" : "bg-zinc-400 dark:bg-zinc-600")} />
          </span>
          <span className="font-medium">{t(proxyRunning ? "newapi.proxyRunning" : "newapi.proxyStopped")}</span>
          <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
            http://127.0.0.1:{router?.port ?? 43127}
          </span>
        </div>}
      </div>

      {(error || state.error) && (
        <div
          role="alert"
          className="flex items-start justify-between gap-3 rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
        >
          <span>{error ? t(error) : formatInvokeError(state.error)}</span>
          <button
            type="button"
            onClick={() => { setError(null); void state.refetch(); }}
            aria-label={t("common.cancel")}
            className="shrink-0 rounded p-0.5 transition-colors hover:bg-destructive/10"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      <CodexAccessCard store={store} providers={providers} selectedProvider={selectedProvider} nativeCompatible={nativeCompatible} onRestart={() => setRestartConfirmOpen(true)} />

      <ClientConfigs providers={providers} />

      <ClaudeDesktopConfigCard providers={providers} />

      <RoutingDiagnostics failover={router?.sameModelFailover ?? false} disabled={nativeAccess || !state.isSuccess || store.mutationLocked}
        webSearchProviderId={router?.webSearchProviderId ?? null}
        searchProviders={providers.filter(provider => provider.webSearchCapable)}
        searchDisabled={!state.isSuccess || store.mutationLocked}
        onWebSearchProvider={providerId => { void run("web-search-provider", async () => applyChange(await api.routing.setWebSearchProvider(providerId)), t("newapi.webSearchSaveFailed")); }}
        compression={state.data?.compression ?? { mode: "percent", percent: 90 }}
        onCompression={policy => { void run("compression", async () => applyChange(await api.routing.setCompression(policy)), t("newapi.compression.saveFailed")); }}
        onFailover={enabled => { void run("failover", async () => applyChange(await api.routing.setFailover(enabled)), t("newapi.traceSettingsFailed")); }} />
      <ModelCapabilities />

      <RelayList store={store} view={view} providers={providers}
        onEdit={provider => setDialog({ open: true, provider })} onTest={setTestDialog} onRemove={setRemoving} onBalance={provider => setBalanceId(provider.id)} />

      <ProviderDialog
        open={dialog.open}
        provider={dialog.provider}
        onOpenChange={open => setDialog(current => ({ ...current, open }))}
        onSaved={applyChange}
      />

      {balanceId && providers.find(provider => provider.id === balanceId) && <ProviderBalanceDialog
        key={balanceId} provider={providers.find(provider => provider.id === balanceId)!} store={store}
        onClose={() => setBalanceId(null)} />}
      <GroupAliasDialog view={view} />

      <ProviderTestDialog
        open={testDialog !== null}
        provider={testDialog}
        onOpenChange={open => !open && setTestDialog(null)}
        onTested={patchProviderTest}
        onFailed={() => void state.refetch()}
      />

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
                void run("restart-codex", async () => {
                  await api.targets.restart("codex");
                  toast({ title: t("newapi.restartCodexSuccess"), variant: "success" });
                }, t("newapi.restartCodexFailed"));
              }}
            >
              {t("newapi.restartCodex")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={removing !== null} onOpenChange={open => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("newapi.confirmDeleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("newapi.confirmDeleteBody", { name: removing?.name ?? "" })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const provider = removing;
                setRemoving(null);
                if (provider) {
                  void run(`delete:${provider.id}`, async () => {
                    const change = await api.providers.delete(provider.id);
                    applyChange(change);
                  }, t("newapi.deleteFailed"));
                }
              }}
            >
              {t("common.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
