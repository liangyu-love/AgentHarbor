import { useContext, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { checkinTally } from "@/components/accounts/source-checkin";
import { PageActiveContext } from "@/components/layout/page-stage";
import { toast } from "@/hooks/use-toast";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { api } from "@/lib/api";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import type { AccountSourceKind, AccountSourceListPayload, AccountSourceMethod, AccountSourceSignInPrompt } from "@/types/account-sources";

type Pending = { id: string; kind: AccountSourceKind; method?: string; input?: string };
/** A sign-in waiting on the user: a risk to accept, or something to type. */
export type SignInAsk = { kind: AccountSourceKind; method: AccountSourceMethod };

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
  // A source with a risk is signed in to only once the user accepts it, and
  // a method that asks for a key only once it is typed.
  const [riskFor, setRiskFor] = useState<SignInAsk | null>(null);
  const [inputFor, setInputFor] = useState<SignInAsk | null>(null);
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
    mutationFn: ({ id, kind, method, input }: Pending) => api.accountSources.signIn(kind, id, method, input),
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
  const startSignIn = (kind: AccountSourceKind, method?: string, input?: string) => {
    if (attempt.current) return;
    const next = { id: crypto.randomUUID(), kind, method, input };
    attempt.current = next;
    setPending(next);
    signInMutation.mutate(next);
  };
  // The source's first way is its default: only another is named to the backend.
  const methodOf = (ask: SignInAsk) =>
    query.data?.sources.find(item => item.kind === ask.kind)?.methods[0]?.id === ask.method.id ? undefined : ask.method.id;
  const askInput = (ask: SignInAsk) => {
    if (ask.method.input) setInputFor(ask);
    else startSignIn(ask.kind, methodOf(ask));
  };
  /** Sign in to a source by one of its methods (its first when none is named). */
  const signIn = (kind: AccountSourceKind, methodId?: string) => {
    if (attempt.current) return;
    const source = query.data?.sources.find(item => item.kind === kind);
    const method = source?.methods.find(item => item.id === methodId) ?? source?.methods[0] ?? { id: "account", input: null };
    if (source?.risk) setRiskFor({ kind, method });
    else askInput({ kind, method });
  };
  const acceptRisk = () => {
    const ask = riskFor;
    setRiskFor(null);
    if (ask) askInput(ask);
  };
  const submitInput = (value: string) => {
    const ask = inputFor;
    setInputFor(null);
    if (ask && value.trim()) startSignIn(ask.kind, methodOf(ask), value.trim());
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
  const setCheckin = useMutation({
    mutationFn: ({ kind, on }: { kind: AccountSourceKind; on: boolean }) => api.accountSources.setCheckin(kind, on),
    onSuccess: settle,
    onError: failed,
  });
  const setCheckinOff = useMutation({
    mutationFn: ({ id, off }: { id: string; off: boolean }) => api.accountSources.setCheckinOff(id, off),
    onSuccess: settle,
    onError: failed,
  });
  const checkIn = useMutation({
    mutationFn: api.accountSources.checkIn,
    // Say what came of it: a press can find every account already in, or
    // leave some out (captcha, the vendor's own app, a failed request).
    onSuccess: (data, kind) => {
      settle(data);
      const tally = checkinTally(data, kind);
      const description = (["claimed", "done", "missed"] as const)
        .filter(part => tally[part] > 0)
        .map(part => t(`accountSources.checkin.tally.${part}`, { count: tally[part] }))
        .join(" · ");
      toast({
        title: t(tally.missed > 0 ? "accountSources.checkin.ranPartly" : "accountSources.checkin.ran"),
        description: description || undefined,
        variant: tally.missed > 0 ? "warning" : "success",
      });
    },
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
    inputFor, submitInput, dismissInput: () => setInputFor(null), today: query.data?.today ?? "",
    activate, remove, refreshQuota, refreshModels, setExposed, setHiddenModels, setProject, setCheckin, setCheckinOff, checkIn,
  };
}
