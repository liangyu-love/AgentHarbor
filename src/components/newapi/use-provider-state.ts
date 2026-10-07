import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { useProviderBalanceUpdates } from "@/hooks/use-provider-balance-updates";
import { toast } from "@/hooks/use-toast";
import { queryKeys } from "@/lib/query-keys";
import { invalidateClaudeDesktop } from "@/lib/claude-desktop";
import { aggregationQueryKey } from "@/types/aggregation";
import { providerErrorKey } from "@/components/newapi/relay-groups";
import type { NewApiProvider, NewApiState, NewApiStateChange } from "@/types/newapi";

const STATE_KEY = queryKeys.providers;

// Provider state plus the mutation plumbing shared by every control on the page:
// one mutation at a time, balance refreshes and exports tracked separately.
export function useProviderState() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: STATE_KEY, queryFn: api.providers.loadState, refetchOnWindowFocus: false });
  const [busy, setBusy] = useState<string | null>(null);
  const mutationInFlight = useRef(false);
  const balanceInFlight = useRef(new Set<string>());
  const [balanceBusy, setBalanceBusy] = useState<Set<string>>(() => new Set());
  const [balanceErrors, setBalanceErrors] = useState<Record<string, string | undefined>>({});
  const exportInFlight = useRef(false);
  const [exportBusy, setExportBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Cache-backed so the injection and relay pages show the same latest backup path.
  const appliedNote = useQuery<NewApiStateChange["applied"]>({
    queryKey: queryKeys.providerApplied,
    queryFn: () => null,
    enabled: false,
    initialData: null,
    staleTime: Infinity,
    gcTime: Infinity,
  }).data;

  useProviderBalanceUpdates();

  const replaceState = (next: NewApiState) => queryClient.setQueryData(STATE_KEY, next);
  const applyChange = (change: NewApiStateChange) => {
    replaceState(change.state);
    if (change.applied) queryClient.setQueryData(queryKeys.providerApplied, change.applied);
    void invalidateClaudeDesktop(queryClient);
    void queryClient.invalidateQueries({ queryKey: aggregationQueryKey });
    void queryClient.invalidateQueries({ queryKey: queryKeys.clientConfigs });
  };

  // Check if provider matches balance query context
  const matchesBalanceContext = (item: NewApiProvider, provider: NewApiProvider) =>
    item.id === provider.id
    && item.baseUrl === provider.baseUrl
    && item.userId === provider.userId
    && item.networkMode === provider.networkMode
    && (item.balanceScript ?? "") === (provider.balanceScript ?? "");

  const patchProviderBalance = (provider: NewApiProvider) => {
    const current = queryClient.getQueryData<NewApiState>(STATE_KEY);
    const ownerId = provider.balanceSharedFrom ?? provider.id;
    if (current) replaceState({
      ...current,
      providers: current.providers.map(item => item.baseUrl === provider.baseUrl
        && (matchesBalanceContext(item, provider) || item.id === ownerId || item.balanceSharedFrom === ownerId)
        ? { ...item, balance: provider.balance, balanceSource: provider.balanceSource,
            ...(item.id === provider.id ? { balanceSharedFrom: provider.balanceSharedFrom } : {}) }
        : item
      )
    });
  };
  const patchBalanceCredentials = (provider: NewApiProvider) => {
    queryClient.setQueryData<NewApiState>(STATE_KEY, current => current && ({
      ...current, providers: current.providers.map(item => item.id === provider.id && item.baseUrl === provider.baseUrl
        ? { ...item, userId: provider.userId, hasAccessToken: provider.hasAccessToken, balance: provider.balance, balanceSource: provider.balanceSource, balanceScript: provider.balanceScript, balanceSharedFrom: provider.balanceSharedFrom }
        : item.balanceSharedFrom === provider.id && item.baseUrl === provider.baseUrl
        ? { ...item, userId: provider.userId, hasAccessToken: provider.hasAccessToken, balance: provider.balance, balanceSource: provider.balanceSource, balanceScript: provider.balanceScript }
        : item),
    }));
  };
  const patchProviderTest = (provider: NewApiProvider) => {
    const current = queryClient.getQueryData<NewApiState>(STATE_KEY);
    if (current) replaceState({ ...current, providers: current.providers.map(item => (item.id === provider.id
      ? { ...item, testStatus: provider.testStatus, testAt: provider.testAt }
      : item)) });
  };
  const run = async (key: string, action: () => Promise<void>, failureTitle: string) => {
    if (mutationInFlight.current) return;
    mutationInFlight.current = true;
    setBusy(key);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(providerErrorKey(e));
      toast({ title: failureTitle, description: t(providerErrorKey(e)), variant: "destructive" });
      // The backend may have persisted partial state (e.g. a failed test marker) before erroring.
      void state.refetch();
    } finally {
      mutationInFlight.current = false;
      setBusy(null);
    }
  };
  const refreshBalance = async (id: string) => {
    if (balanceInFlight.current.has(id)) return;
    balanceInFlight.current.add(id);
    setBalanceBusy(new Set(balanceInFlight.current));
    setBalanceErrors(errors => ({ ...errors, [id]: undefined }));
    try {
      patchProviderBalance(await api.providers.queryBalance(id));
    } catch (cause) {
      setBalanceErrors(errors => ({ ...errors, [id]: providerErrorKey(cause) }));
      toast({ title: t("newapi.balanceFailed"), description: t(providerErrorKey(cause)), variant: "destructive" });
    } finally {
      balanceInFlight.current.delete(id);
      setBalanceBusy(new Set(balanceInFlight.current));
    }
  };
  const pickImportFile = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    return open({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] });
  };
  const importProviders = () =>
    run("import", async () => {
      const path = await pickImportFile();
      if (typeof path !== "string") return;
      const result = await api.providers.import(path);
      applyChange(result);
      toast({ title: t("newapi.importedResult", { imported: result.imported, skipped: result.skipped }), variant: "success" });
    }, t("newapi.importFailed"));
  const exportProviders = async () => {
    if (exportInFlight.current) return;
    exportInFlight.current = true;
    setExportBusy(true);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({ defaultPath: "agentharbor-relays.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      await api.providers.export(path);
      toast({ title: t("newapi.exportedDone", { path }), variant: "success" });
    } catch (cause) {
      toast({ title: t("newapi.exportFailed"), description: t(providerErrorKey(cause)), variant: "destructive" });
    } finally {
      exportInFlight.current = false;
      setExportBusy(false);
    }
  };

  return {
    state, busy, mutationLocked: busy !== null, error, setError, appliedNote, balanceBusy, exportBusy,
    run, applyChange, patchProviderTest, patchBalanceCredentials, balanceErrors, refreshBalance, importProviders, exportProviders,
  };
}

export type ProviderState = ReturnType<typeof useProviderState>;
