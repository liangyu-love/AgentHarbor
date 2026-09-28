import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import type { McpServerSummary } from "@/types";
import { useBusyAction } from "@/hooks/use-busy-action";
import { queryKeys } from "@/lib/query-keys";

const PAGE_SIZE = 12;

export type McpFilter = "all" | "enabled" | "disabled" | "stdio" | "http";

// MCP server list with its toggle/remove mutations, search, filter and pagination.
export function useMcpServers() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<McpServerSummary | "new" | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [currentPage, setCurrentPage] = useState(1);
  const [searchQuery, setSearchQuery] = useState("");
  const [transportFilter, setTransportFilter] = useState<McpFilter>("all");
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);

  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const { data, refetch, error, isPending } = useQuery({
    queryKey: queryKeys.mcpServers,
    queryFn: () => api.loadMcpServers(),
    staleTime: Infinity,
  });

  const refreshBusy = refreshAction.busy;

  const handleRefresh = async () => {
    await refreshAction.run(async () => {
      await refetch();
    });
  };

  const toggleMutation = useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) =>
      api.setMcpServerEnabled(name, enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers }),
  });

  const removeMutation = useMutation({
    mutationFn: (name: string) => api.removeMcpServer(name),
    onSuccess: () => {
      setRemoving(null);
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
    },
  });

  const servers = data?.data.items ?? [];
  const enabledCount = servers.filter((s) => s.enabled).length;
  const disabledCount = servers.length - enabledCount;
  const stdioCount = servers.filter((s) => s.transport === "stdio").length;
  const remoteCount = servers.length - stdioCount;
  const sourcePath = data?.data.sourcePath ?? "";

  const copyPath = () => {
    if (!sourcePath) return;
    navigator.clipboard.writeText(sourcePath);
    toast({
      title: t("mcp.pathCopied"),
      description: t("mcp.pathCopiedDesc"),
      variant: "default",
    });
  };

  const handleOpenConfigFolder = async () => {
    if (!sourcePath) return;
    try {
      const parts = sourcePath.split(/[/\\]/);
      parts.pop();
      const dir = parts.join(sourcePath.includes("\\") ? "\\" : "/");
      await api.openPath(dir || sourcePath);
    } catch (err) {
      toast({
        title: t("common.error"),
        description: String(err),
        variant: "destructive",
      });
    }
  };

  const handleCopyCommand = (server: McpServerSummary) => {
    let text = "";
    if (server.transport === "stdio") {
      text = [server.command, ...(server.args || [])].filter(Boolean).join(" ");
    } else {
      text = server.url ?? "";
    }
    navigator.clipboard.writeText(text);
    setCopiedCommand(server.name);
    setTimeout(() => setCopiedCommand(null), 1800);
    toast({
      title: t("mcp.commandCopied"),
      variant: "default",
    });
  };

  const filteredServers = useMemo(() => {
    return servers.filter((server) => {
      if (transportFilter === "enabled" && !server.enabled) return false;
      if (transportFilter === "disabled" && server.enabled) return false;
      if (transportFilter === "stdio" && server.transport !== "stdio") return false;
      if (transportFilter === "http" && server.transport === "stdio") return false;

      if (!searchQuery.trim()) return true;
      const q = searchQuery.toLowerCase().trim();
      const matchName = server.name.toLowerCase().includes(q);
      const matchCmd = (server.command ?? "").toLowerCase().includes(q);
      const matchArgs = (server.args ?? []).some((a) => a.toLowerCase().includes(q));
      const matchUrl = (server.url ?? "").toLowerCase().includes(q);
      return matchName || matchCmd || matchArgs || matchUrl;
    });
  }, [servers, transportFilter, searchQuery]);

  const totalPages = Math.max(1, Math.ceil(filteredServers.length / PAGE_SIZE));
  const safePage = Math.min(currentPage, totalPages);
  const pagedServers = filteredServers.slice((safePage - 1) * PAGE_SIZE, safePage * PAGE_SIZE);

  const paginationRange = useMemo(() => {
    const range: (number | "ellipsis")[] = [];
    if (totalPages <= 7) {
      for (let i = 1; i <= totalPages; i++) range.push(i);
    } else {
      range.push(1);
      if (safePage > 3) range.push("ellipsis");
      const start = Math.max(2, safePage - 1);
      const end = Math.min(totalPages - 1, safePage + 1);
      for (let i = start; i <= end; i++) range.push(i);
      if (safePage < totalPages - 2) range.push("ellipsis");
      range.push(totalPages);
    }
    return range;
  }, [totalPages, safePage]);

  return {
    data, refetch, error, isPending, servers, sourcePath,
    enabledCount, disabledCount, stdioCount, remoteCount,
    editing, setEditing, removing, setRemoving, currentPage, setCurrentPage,
    searchQuery, setSearchQuery, transportFilter, setTransportFilter, copiedCommand,
    refreshBusy, handleRefresh, toggleMutation, removeMutation,
    copyPath, handleOpenConfigFolder, handleCopyCommand,
    filteredServers, totalPages, pagedServers, paginationRange,
  };
}

export type McpServers = ReturnType<typeof useMcpServers>;
