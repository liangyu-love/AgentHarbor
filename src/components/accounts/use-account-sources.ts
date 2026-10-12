import { useContext, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { PageActiveContext } from "@/components/layout/page-stage";
import { toast } from "@/hooks/use-toast";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { api } from "@/lib/api";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import type { AccountSourceKind, AccountSourceListPayload, AccountSourceSignInPrompt } from "@/types/account-sources";

type Pending = { id: string; kind: AccountSourceKind };

// Account sources (WorkBuddy, ChatGPT, Copilot…): their accounts, one
// cancellable sign-in at a time (with the device code a device sign-in asks
// the user to type), and their share with model inject.
export function useAccountSources(refreshInterval: RefreshInterval) {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const queryClient = useQueryClient();
  const attempt = useRef<Pending | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);
  const [prompt, setPrompt] = useState<AccountSourceSignInPrompt | null>(null);
  // A source with a risk is signed in to only once the user accepts it.
  const [riskFor, setRiskFor] = useState<AccountSourceKind | null>(null);
  const clearAttempt = () => { attempt.current = null; setPending(null); setPrompt(null); };
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void api.accountSources.onSignInPrompt(next => {
      if (attempt.current?.id === next.attemptId) setPrompt(next);
    }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
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
  const startSignIn = (kind: AccountSourceKind) => {
    if (attempt.current) return;
    const next = { id: crypto.randomUUID(), kind };
    attempt.current = next;
    setPending(next);
    signInMutation.mutate(next);
  };
  const signIn = (kind: AccountSourceKind) => {
    if (attempt.current) return;
    if (query.data?.sources.find(source => source.kind === kind)?.risk) setRiskFor(kind);
    else startSignIn(kind);
  };
  const acceptRisk = () => {
    const kind = riskFor;
    setRiskFor(null);
    if (kind) startSignIn(kind);
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
  const setProject = useMutation({
    mutationFn: ({ id, project }: { id: string; project: string }) => api.accountSources.setProject(id, project),
    onSuccess: (data) => { settle(data); toast({ title: t("accountSources.projectSaved"), variant: "success" }); },
    onError: failed,
  });
  const setExposed = useMutation({
    mutationFn: ({ kind, exposed }: { kind: AccountSourceKind; exposed: boolean }) => api.accountSources.setExposed(kind, exposed),
    onSuccess: settle,
    onError: failed,
  });
  const setHiddenModels = useMutation({
    mutationFn: ({ kind, hidden }: { kind: AccountSourceKind; hidden: string[] }) => api.accountSources.setHiddenModels(kind, hidden),
    onSuccess: settle,
    onError: failed,
  });

  return {
    query, sources: query.data?.sources ?? [], pending, prompt: pending && prompt?.attemptId === pending.id ? prompt : null,
    cancelling, signIn, cancelSignIn, riskFor, acceptRisk, dismissRisk: () => setRiskFor(null),
    activate, remove, refreshQuota, refreshModels, setExposed, setHiddenModels, setProject,
  };
}
