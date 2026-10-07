import { useContext, useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { PageActiveContext } from "@/components/layout/page-stage";
import { toast } from "@/hooks/use-toast";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { api } from "@/lib/api";
import type { CodexAccountSummary } from "@/types";
import { queryKeys } from "@/lib/query-keys";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";

const QUERY_KEY = queryKeys.codexAccounts;
const TOKEN_USAGE_QUERY_KEY = queryKeys.tokenUsage;

// Account list and token usage queries (polled only while the page is active),
// the login / switch / delete / quota mutations, and the search + plan filter.
export function useAccounts(refreshInterval: RefreshInterval) {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const queryClient = useQueryClient();
  const [search, setSearch] = useState("");
  const [plan, setPlan] = useState("all");
  const [deleteTarget, setDeleteTarget] = useState<CodexAccountSummary | null>(null);
  const activeLogin = useRef<string | null>(null);
  const [activeLoginId, setActiveLoginId] = useState<string | null>(null);
  const clearLogin = () => { activeLogin.current = null; setActiveLoginId(null); };
  useEffect(() => () => {
    const id = activeLogin.current;
    activeLogin.current = null;
    if (id) void api.accounts.cancelLogin(id).catch(() => {});
  }, []);

  const accountsQuery = useQuery({
    queryKey: QUERY_KEY,
    queryFn: api.accounts.refresh,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
  const tokenUsageQuery = useQuery({
    queryKey: TOKEN_USAGE_QUERY_KEY,
    queryFn: api.usage.overview,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });

  const setData = (data: Awaited<ReturnType<typeof api.accounts.list>>) => {
    queryClient.setQueryData(QUERY_KEY, data);
  };

  const loginMutation = useMutation({
    mutationFn: api.accounts.login,
    onSuccess: (data, id) => {
      if (activeLogin.current !== id) return;
      clearLogin();
      setData(data);
      toast({ title: t("accounts.loginSuccess"), variant: "success" });
    },
    onError: (error, id) => {
      if (activeLogin.current !== id) return;
      clearLogin();
      if (errorMessage(error) === "accounts.loginCancelled") return;
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
    },
  });

  const cancelLoginMutation = useMutation({
    mutationFn: api.accounts.cancelLogin,
    onSuccess: (cancelled, id) => {
      if (!cancelled || activeLogin.current !== id) return;
      clearLogin();
      loginMutation.reset();
      toast({ title: t("accounts.loginCancelled"), variant: "success" });
    },
    onError: (error, id) => {
      if (activeLogin.current !== id) return;
      toast({ title: t("accounts.cancelFailed"), description: formatInvokeError(error), variant: "destructive" });
    },
  });
  const startLogin = () => {
    if (activeLogin.current) return;
    const id = crypto.randomUUID();
    activeLogin.current = id;
    setActiveLoginId(id);
    loginMutation.mutate(id);
  };
  const cancelLogin = () => {
    if (activeLogin.current && !cancelLoginMutation.isPending) cancelLoginMutation.mutate(activeLogin.current);
  };
  const loginPending = activeLoginId !== null;
  const loginCancelling = cancelLoginMutation.isPending && cancelLoginMutation.variables === activeLoginId;

  const switchMutation = useMutation({
    mutationFn: async (accountKey: string) => {
      const data = await api.accounts.switch(accountKey);
      try {
        await api.targets.restart("codex");
        return { data, restartError: null };
      } catch (error) {
        // The auth switch is committed. Report the restart failure separately.
        return { data, restartError: formatInvokeError(error) };
      }
    },
    onSuccess: ({ data, restartError }) => {
      setData(data);
      toast(restartError
        ? { title: t("accounts.switchRestartFailed"), description: restartError, variant: "destructive" }
        : { title: t("accounts.switchSuccess"), variant: "success" });
    },
    onError: (error) => {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: api.accounts.delete,
    onSuccess: (data) => {
      setData(data);
      setDeleteTarget(null);
      toast({ title: t("accounts.deleteSuccess"), variant: "success" });
    },
    onError: (error) => {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
    },
  });

  const refreshQuotaMutation = useMutation({
    mutationFn: async (_key: string) => {
      return await api.accounts.refresh();
    },
    onSuccess: (data) => {
      setData(data);
      toast({ title: "账号额度已刷新", variant: "success" });
    },
    onError: (error) => {
      toast({ title: "刷新失败", description: formatInvokeError(error), variant: "destructive" });
    },
  });

  const refresh = async () => {
    const [accountsResult] = await Promise.all([accountsQuery.refetch(), tokenUsageQuery.refetch()]);
    if (accountsResult.data) toast({ title: t("accounts.refreshSuccess"), variant: "success" });
  };

  const accounts = accountsQuery.data?.items ?? [];
  const plans = useMemo(
    () => Array.from(new Set(accounts.map((account) => account.plan).filter(Boolean))).sort(),
    [accounts],
  );
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return accounts.filter((account) => {
      if (plan !== "all" && account.plan !== plan) return false;
      if (!needle) return true;
      return `${account.email} ${account.displayName}`.toLowerCase().includes(needle);
    });
  }, [accounts, plan, search]);

  return {
    accountsQuery, tokenUsageQuery, accounts, plans, filtered, search, setSearch, plan, setPlan,
    deleteTarget, setDeleteTarget, startLogin, cancelLogin, loginPending, loginCancelling,
    switchMutation, deleteMutation, refreshQuotaMutation, refresh,
  };
}
