import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Cpu,
  Monitor,
  Network,
  RadioTower,
  Sparkles,
  Terminal,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
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
import { ProviderBalanceDialog } from "@/components/newapi/provider-balance-dialog";
import { ProviderDialog } from "@/components/newapi/provider-dialog";
import { ProviderTestDialog } from "@/components/newapi/provider-test-dialog";
import { ClientConfigs } from "@/components/newapi/client-configs";
import { ClaudeDesktopConfigCard } from "@/components/newapi/claude-desktop-config-card";
import { CodexAccessCard } from "@/components/newapi/codex-access-card";
import { RoutingDiagnostics } from "@/components/newapi/routing-diagnostics";
import { AggregationManager } from "@/components/newapi/aggregation-manager";
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
  const [dialog, setDialog] = useState<{ open: boolean; provider: NewApiProvider | null }>({
    open: false,
    provider: null,
  });
  const [testDialog, setTestDialog] = useState<NewApiProvider | null>(null);
  const [balanceId, setBalanceId] = useState<string | null>(null);
  const [removing, setRemoving] = useState<NewApiProvider | null>(null);
  const [restartConfirmOpen, setRestartConfirmOpen] = useState(false);

  const providers = state.data?.providers ?? [];
  const router = state.data?.router;
  const nativeAccess = (router?.accessMode ?? "router") === "native";
  const activeId = state.data?.activeProviderId ?? null;
  const selectedProvider = activeId
    ? providers.find((provider) => provider.id === activeId)
    : providers.find((provider) => provider.enabled && provider.hasApiKey && provider.model);
  const nativeCompatible = !!selectedProvider && providerModelProtocol(selectedProvider) === "responses";
  const proxyRunning = !!state.data?.proxyRunning;
  const view = useRelayView(providers, state.isSuccess);

  const scrollToSection = (id: string) => {
    const el = document.getElementById(id);
    if (el) {
      el.scrollIntoView({
        behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth",
        block: "start",
      });
      el.setAttribute("tabindex", "-1");
      el.focus({ preventScroll: true });
    }
  };

  return (
    <div className="space-y-6">
      {/* Top Banner & Quick Navigator */}
      <div className="rounded-2xl border border-border/80 bg-gradient-to-br from-card/90 via-card/60 to-background p-5 shadow-sm backdrop-blur-md">
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <div className="flex h-6 w-6 items-center justify-center rounded-lg bg-primary/10 text-primary">
                <Network className="h-3.5 w-3.5" />
              </div>
              <h1 className="text-base font-semibold tracking-tight text-foreground">
                {t("newapi.title")}
              </h1>
            </div>
            <p className="text-xs leading-relaxed text-muted-foreground">{t("newapi.description")}</p>
          </div>

          <div className="flex flex-wrap items-center gap-2 shrink-0">
            {nativeAccess ? (
              <Badge variant="outline" className="text-xs">
                {t("newapi.nativeNoRouter")}
              </Badge>
            ) : (
              <div className="flex items-center gap-2 rounded-full border border-border/80 bg-background/80 py-1 pl-3 pr-2 text-xs shadow-sm backdrop-blur-md">
                <span className="relative flex h-2 w-2 items-center justify-center">
                  {proxyRunning && (
                    <span
                      aria-hidden
                      className="absolute inline-flex h-full w-full animate-ping motion-reduce:animate-none rounded-full bg-emerald-400 opacity-60"
                    />
                  )}
                  <span
                    className={cn(
                      "relative inline-flex h-2 w-2 rounded-full",
                      proxyRunning ? "bg-emerald-500" : "bg-zinc-400 dark:bg-zinc-600",
                    )}
                  />
                </span>
                <span className="font-medium text-foreground">
                  {t(proxyRunning ? "newapi.proxyRunning" : "newapi.proxyStopped")}
                </span>
                <span className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
                  http://127.0.0.1:{router?.port ?? 43127}
                </span>
              </div>
            )}
          </div>
        </div>

        {/* Section Fast Jumper Pills */}
        <div className="mt-4 flex flex-wrap items-center gap-1.5 border-t border-border/40 pt-3">
          <span className="text-[11px] font-medium text-muted-foreground mr-1">{t("newapi.quickNavigation")}</span>
          <button
            type="button"
            onClick={() => scrollToSection("newapi-routing-title")}
            className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground transition-all hover:border-primary/40 hover:bg-muted hover:text-foreground motion-safe:active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Sparkles className="h-3 w-3 text-primary" />
            <span>{t("newapi.codexAccess")}</span>
          </button>
          <button
            type="button"
            onClick={() => scrollToSection("client-configs-title")}
            className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground transition-all hover:border-primary/40 hover:bg-muted hover:text-foreground motion-safe:active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Terminal className="h-3 w-3 text-amber-500" />
            <span>{t("clients.title")}</span>
          </button>
          <button
            type="button"
            onClick={() => scrollToSection("claude-desktop-title")}
            className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground transition-all hover:border-primary/40 hover:bg-muted hover:text-foreground motion-safe:active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Monitor className="h-3 w-3 text-primary" />
            <span>Claude Desktop</span>
          </button>
          <button
            type="button"
            onClick={() => scrollToSection("aggregation-title")}
            className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground transition-all hover:border-primary/40 hover:bg-muted hover:text-foreground motion-safe:active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Cpu className="h-3 w-3 text-indigo-500" />
            <span>{t("aggregation.title")}</span>
          </button>
          <button
            type="button"
            onClick={() => scrollToSection("newapi-providers-title")}
            className="flex items-center gap-1 rounded-lg border border-border/60 bg-muted/30 px-2.5 py-1 text-xs text-muted-foreground transition-all hover:border-primary/40 hover:bg-muted hover:text-foreground motion-safe:active:scale-95 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <RadioTower className="h-3 w-3 text-sky-500" />
            <span>{t("newapi.providersSection")}</span>
          </button>
        </div>
      </div>

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

      {/* 1. Codex 注入 */}
      <CodexAccessCard
        store={store}
        providers={providers}
        selectedProvider={selectedProvider}
        nativeCompatible={nativeCompatible}
        onRestart={() => setRestartConfirmOpen(true)}
      />

      {/* 2. 客户端配置 (Claude Code / OpenCode) */}
      <ClientConfigs providers={providers} />

      {/* 3. Claude Desktop */}
      <ClaudeDesktopConfigCard providers={providers} />

      {/* 4. 模型聚合 */}
      <AggregationManager providers={providers} />

      {/* 5. 路由诊断与模型能力 */}
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

      {/* 6. 中转模型列表 */}
      <RelayList
        store={store}
        view={view}
        providers={providers}
        onEdit={(provider) => setDialog({ open: true, provider })}
        onTest={setTestDialog}
        onRemove={setRemoving}
        onBalance={(provider) => setBalanceId(provider.id)}
      />

      <ProviderDialog
        open={dialog.open}
        provider={dialog.provider}
        onOpenChange={(open) => setDialog((current) => ({ ...current, open }))}
        onSaved={applyChange}
      />

      {balanceId && providers.find((provider) => provider.id === balanceId) && (
        <ProviderBalanceDialog
          key={balanceId}
          provider={providers.find((provider) => provider.id === balanceId)!}
          store={store}
          onClose={() => setBalanceId(null)}
        />
      )}
      <GroupAliasDialog view={view} />

      <ProviderTestDialog
        open={testDialog !== null}
        provider={testDialog}
        onOpenChange={(open) => !open && setTestDialog(null)}
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

      <AlertDialog open={removing !== null} onOpenChange={(open) => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("newapi.confirmDeleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("newapi.confirmDeleteBody", { name: removing?.name ?? "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const provider = removing;
                setRemoving(null);
                if (provider) {
                  void run(
                    `delete:${provider.id}`,
                    async () => {
                      const change = await api.providers.delete(provider.id);
                      applyChange(change);
                    },
                    t("newapi.deleteFailed"),
                  );
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
