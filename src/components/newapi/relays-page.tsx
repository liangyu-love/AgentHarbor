import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { api } from "@/lib/api";
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
import { GroupAliasDialog } from "@/components/newapi/group-alias-dialog";
import { RelayList } from "@/components/newapi/relay-list";
import { useProviderState } from "@/components/newapi/use-provider-state";
import { useRelayView } from "@/components/newapi/use-relay-view";
import type { NewApiProvider } from "@/types/newapi";
import { formatInvokeError } from "@/lib/invoke-error";

// Relay model library: add, test, edit and remove the upstream relays.
export function RelayModelsPage() {
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

  // Account sources publish router providers too; the Accounts page owns them.
  const providers = useMemo(() => (state.data?.providers ?? []).filter((provider) => !provider.accountSource), [state.data?.providers]);
  const balanceProvider = balanceId ? providers.find((provider) => provider.id === balanceId) : undefined;
  const view = useRelayView(providers, state.isSuccess);

  return (
    <div className="space-y-6">
      {(error || state.error) && (
        <div
          role="alert"
          className="flex items-start justify-between gap-3 rounded-2xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
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

      {balanceProvider && (
        <ProviderBalanceDialog
          key={balanceProvider.id}
          provider={balanceProvider}
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
