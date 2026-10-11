import { useContext, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { PageActiveContext } from "@/components/layout/page-stage";
import { toast } from "@/hooks/use-toast";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { api } from "@/lib/api";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import type { AccountSourceKind, AccountSourceListPayload } from "@/types/account-sources";

type Pending = { id: string; kind: AccountSourceKind };

// Account sources (WorkBuddy, ChatGPT…): their accounts, one cancellable
// browser sign-in at a time, and their share with model inject.
export function useAccountSources(refreshInterval: RefreshInterval) {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const queryClient = useQueryClient();
  const attempt = useRef<Pending | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const clearAttempt = () => { attempt.current = null; setPending(null); };
  useEffect(() => () => {
    const current = attempt.current;
    attempt.current = null;
    if (current) void api.accountSources.cancelSignIn(current.id).catch(() => {});
  }, []);

  const query = useQuery({
    queryKey: queryKeys.accountSources,
    queryFn: api.accountSources.list,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
  // A shared source is a router provider: model inject reads it from the provider state.
  const settle = (data: AccountSourceListPayload) => {
    queryClient.setQueryData(queryKeys.accountSources, data);
    void queryClient.invalidateQueries({ queryKey: queryKeys.providers });
  };
  const failed = (error: unknown) =>
    toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });

  const signInMutation = useMutation({
    mutationFn: ({ id, kind }: Pending) => api.accountSources.signIn(kind, id),
    onSuccess: (data, { id }) => {
      if (attempt.current?.id !== id) return;
      clearAttempt();
      settle(data);
      toast({ title: t("accounts.loginSuccess"), variant: "success" });
    },
    onError: (error, { id }) => {
      if (attempt.current?.id !== id) return;
      clearAttempt();
      if (errorMessage(error) === "accounts.loginCancelled") return;
      failed(error);
    },
  });
  const cancelMutation = useMutation({
    mutationFn: api.accountSources.cancelSignIn,
    onSuccess: (cancelled, id) => {
      if (!cancelled || attempt.current?.id !== id) return;
      clearAttempt();
      signInMutation.reset();
      toast({ title: t("accounts.loginCancelled"), variant: "success" });
    },
    onError: (error, id) => {
      if (attempt.current?.id !== id) return;
      toast({ title: t("accounts.cancelFailed"), description: formatInvokeError(error), variant: "destructive" });
    },
  });
  const signIn = (kind: AccountSourceKind) => {
    if (attempt.current) return;
    const next = { id: crypto.randomUUID(), kind };
    attempt.current = next;
    setPending(next);
    signInMutation.mutate(next);
  };
  const cancelSignIn = () => {
    if (attempt.current && !cancelMutation.isPending) cancelMutation.mutate(attempt.current.id);
  };
  const cancelling = cancelMutation.isPending && cancelMutation.variables === pending?.id;

  const activate = useMutation({ mutationFn: api.accountSources.activate, onSuccess: settle, onError: failed });
  const remove = useMutation({
    mutationFn: api.accountSources.remove,
    onSuccess: (data) => { settle(data); toast({ title: t("accounts.deleteSuccess"), variant: "success" }); },
    onError: failed,
  });
  const refreshQuota = useMutation({
    mutationFn: api.accountSources.refreshQuota,
    onSuccess: (data) => { settle(data); toast({ title: t("accounts.quotaRefreshed"), variant: "success" }); },
    onError: failed,
  });
  const refreshModels = useMutation({
    mutationFn: api.accountSources.refreshModels,
    onSuccess: (data) => { settle(data); toast({ title: t("accountSources.modelsRefreshed"), variant: "success" }); },
    onError: failed,
  });
  const setExposed = useMutation({
    mutationFn: ({ kind, exposed }: { kind: AccountSourceKind; exposed: boolean }) => api.accountSources.setExposed(kind, exposed),
    onSuccess: settle,
    onError: failed,
  });

  return {
    query, sources: query.data?.sources ?? [], pending, cancelling, signIn, cancelSignIn,
    activate, remove, refreshQuota, refreshModels, setExposed,
  };
}
