import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import type { McpFailure, McpLibraryView } from "@/types";

/** The MCP library; every change answers with the whole library, which replaces the cached one. */
export function useMcpLibrary() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const query = useQuery({ queryKey: queryKeys.mcpServers, queryFn: api.loadMcpLibrary, staleTime: Infinity });

  const failureText = (failure: McpFailure) => t("mcp.failureLine", {
    name: failure.name || "—", app: t(`mcp.apps.${failure.app}`), error: formatInvokeError(failure.error),
  });

  /** Run a change; resolves with the new library, or null when the change was refused. */
  const run = async (change: () => Promise<McpLibraryView>, success?: (view: McpLibraryView) => string | null) => {
    setBusy(true);
    try {
      const view = await change();
      queryClient.setQueryData(queryKeys.mcpServers, view);
      if (view.failures.length > 0) {
        toast({
          title: t("mcp.failures", { count: view.failures.length }),
          description: view.failures.slice(0, 4).map(failureText).join("\n"),
          variant: "destructive",
        });
      } else {
        const message = success?.(view);
        if (message) toast({ title: message, variant: "success" });
      }
      return view;
    } catch (error) {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
      return null;
    } finally {
      setBusy(false);
    }
  };

  return { ...query, view: query.data, busy, run, failureText };
}

export type McpLibrary = ReturnType<typeof useMcpLibrary>;
