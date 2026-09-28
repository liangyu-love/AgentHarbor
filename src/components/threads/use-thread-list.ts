import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { useBusyAction } from "@/hooks/use-busy-action";
import type { ThreadSummary } from "@/types/threads";
import {
  filterAndSortThreads,
  groupThreads,
  type SortOption,
  type StatusFilter,
} from "@/components/threads/thread-utils";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import type { ConfirmRequest } from "@/components/ui/confirm-dialog";

export type ExportFormat = "markdown" | "jsonl";
export type ViewMode = "grid" | "compact";

const THREADS_KEY = queryKeys.codexThreads;
/** Window the list once dense rows would thrash layout/paint. */
const VIRTUALIZE_THRESHOLD = 80;

// Thread scan state, filters, selection and the batch actions behind the threads page.
export function useThreadList() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<StatusFilter>("all");
  const [query, setQuery] = useState("");
  const [sortBy, setSortBy] = useState<SortOption>("updatedAt");
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    const saved = localStorage.getItem(storageKeys.threadsViewMode);
    return saved === "grid" || saved === "compact" ? saved : "compact";
  });
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [format, setFormat] = useState<ExportFormat>("markdown");
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [copiedGroupPath, setCopiedGroupPath] = useState<string | null>(null);
  const [recoveryTarget, setRecoveryTarget] = useState<ThreadSummary | null>(null);
  const [bulkRepairing, setBulkRepairing] = useState(false);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [cancelPendingBusy, setCancelPendingBusy] = useState(false);

  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const threadsQuery = useQuery({
    queryKey: THREADS_KEY,
    queryFn: () => api.conversations.list("codex"),
    staleTime: 30_000,
    refetchInterval: query => query.state.data?.data.pendingDeletionKeys?.length
      || query.state.data?.data.pendingDeletionRecoveryNeeded ? 20_000 : false,
  });

  const threads = useMemo(() => threadsQuery.data?.data.items ?? [], [threadsQuery.data]);
  const pendingDeletionKeys = new Set(threadsQuery.data?.data.pendingDeletionKeys ?? []);
  // Corrupt queue records arrive as `pendingDeletionRecoveryNeeded`; queue cancellations
  // and failures stay visible here so users can see why a deletion did not run.
  const scanWarnings = threadsQuery.data?.warnings ?? [];
  const pendingDeletionRecoveryNeeded = threadsQuery.data?.data.pendingDeletionRecoveryNeeded ?? false;
  const exportableKeys = useMemo(
    () => new Set(threads.filter((thread) => thread.rolloutStatus !== "missing").map((thread) => thread.key)),
    [threads],
  );
  const selectedExportableCount = useMemo(
    () => [...selected].filter((key) => exportableKeys.has(key)).length,
    [selected, exportableKeys],
  );

  // Filter and Sort
  const filtered = useMemo(
    () => filterAndSortThreads(threads, status, query, sortBy),
    [threads, status, query, sortBy],
  );

  // Group by Workspace
  const groups = useMemo(
    () => groupThreads(filtered, t("threads.unknownFolder")),
    [filtered, t],
  );

  // Global aggregate metrics
  const { archivedCount, totalUserMsgs, totalAgentMsgs, totalBytes, totalTokens, missingRolloutCount } = useMemo(() => {
    let archived = 0;
    let user = 0;
    let agent = 0;
    let bytes = 0;
    let tokens = 0;
    let missingRollouts = 0;
    for (const item of threads) {
      if (item.archived) archived++;
      if (item.rolloutStatus === "missing") missingRollouts++;
      user += item.userMessages || 0;
      agent += item.agentMessages || 0;
      bytes += item.fileSize || 0;
      tokens += item.tokensUsed || 0;
    }
    return {
      archivedCount: archived,
      totalUserMsgs: user,
      totalAgentMsgs: agent,
      totalBytes: bytes,
      totalTokens: tokens,
      missingRolloutCount: missingRollouts,
    };
  }, [threads]);

  const totalInteractions = totalUserMsgs + totalAgentMsgs;
  const useVirtualList = filtered.length >= VIRTUALIZE_THRESHOLD;

  // Keep the selection limited to visible threads: batch delete/export must never
  // act on threads hidden by the current filter or search.
  useEffect(() => {
    const visible = new Set(filtered.map((thread) => thread.key));
    setSelected((previous) => {
      const next = new Set([...previous].filter((key) => visible.has(key)));
      return next.size === previous.size ? previous : next;
    });
  }, [filtered]);

  const toggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const selectFiltered = () => setSelected(new Set(filtered.map((item) => item.key)));
  const clearSelection = () => setSelected(new Set());

  // Group selection toggle
  const toggleGroupSelect = (groupKeys: string[]) => {
    const allSelected = groupKeys.every((key) => selected.has(key));
    setSelected((prev) => {
      const next = new Set(prev);
      if (allSelected) {
        for (const key of groupKeys) next.delete(key);
      } else {
        for (const key of groupKeys) next.add(key);
      }
      return next;
    });
  };

  // Group collapse toggle
  const toggleGroupCollapse = (groupKey: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(groupKey)) {
        next.delete(groupKey);
      } else {
        next.add(groupKey);
      }
      return next;
    });
  };

  const expandAllGroups = () => setCollapsedGroups(new Set());
  const collapseAllGroups = () => setCollapsedGroups(new Set(groups.map((g) => g.key)));

  useEffect(() => {
    localStorage.setItem(storageKeys.threadsViewMode, viewMode);
  }, [viewMode]);

  const handleRefresh = async () => {
    await refreshAction.run(async () => {
      await threadsQuery.refetch();
    });
  };

  const handleRepairAll = async () => {
    const candidates = [...new Map(
      threads.filter((thread) => thread.id).map((thread) => [thread.id, thread]),
    ).values()];
    if (candidates.length === 0 || bulkRepairing) return;
    setConfirmRequest({
      title: t("threads.repairAll"),
      description: t("threads.repairAllConfirm", { count: candidates.length }),
      confirmLabel: t("threads.repairAll"),
      onConfirm: () => void repairAll(candidates),
    });
  };

  const repairAll = async (candidates: ThreadSummary[]) => {
    setBulkRepairing(true);
    let repaired = 0;
    let skipped = 0;
    let failed = 0;
    try {
      for (const thread of candidates) {
        try {
          const preview = (await api.conversations.previewRecovery(thread.id)).data;
          if (!preview.canApply) {
            skipped += 1;
            continue;
          }
          const result = (await api.conversations.applyRecovery(thread.id, preview.revision)).data;
          if (result.changed) repaired += 1;
          else skipped += 1;
        } catch {
          failed += 1;
        }
      }
      await threadsQuery.refetch();
      toast({
        title: t("threads.repairAllDone"),
        description: t("threads.repairAllResult", { repaired, skipped, failed }),
        variant: failed === 0 ? "success" : "default",
      });
    } finally {
      setBulkRepairing(false);
    }
  };

  const handleDelete = async (keys: string[]) => {
    const uniqueKeys = [...new Set(keys)].filter(key => !pendingDeletionKeys.has(key));
    if (uniqueKeys.length === 0 || deleteBusy) return;
    setConfirmRequest({
      title: t("threads.deleteThread"),
      description: t("threads.deleteConfirm", { count: uniqueKeys.length }),
      confirmLabel: t("common.delete"),
      destructive: true,
      onConfirm: () => void deleteThreads(uniqueKeys),
    });
  };

  const deleteThreads = async (uniqueKeys: string[]) => {
    setDeleteBusy(true);
    try {
      const result = await api.conversations.delete("codex", uniqueKeys);
      setSelected((previous) => {
        const next = new Set(previous);
        for (const key of uniqueKeys) next.delete(key);
        return next;
      });
      await threadsQuery.refetch();
      toast({
        title: t(result.data.queued ? "threads.deleteQueued" : "threads.deleteDone", {
          count: result.data.queued || result.data.deleted,
        }),
        variant: result.data.queued ? "default" : "success",
      });
    } catch (cause) {
      const message = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "";
      toast({
        title: t("threads.deleteFailed"),
        description: message.includes("CODEX_WRITER_RUNNING")
          ? t("threads.deleteWriterRunning")
          : message.includes("THREAD_DELETE_ALREADY_QUEUED") ? t("threads.deleteAlreadyQueued")
          : message || undefined,
        variant: "destructive",
      });
    } finally {
      setDeleteBusy(false);
    }
  };

  const cancelPendingDeletions = async () => {
    if (cancelPendingBusy) return;
    setCancelPendingBusy(true);
    try {
      const result = await api.conversations.cancelPendingDeletions("codex");
      await threadsQuery.refetch();
      toast({ title: t("threads.deleteQueueCancelled", { count: result.data }), variant: "success" });
    } catch (cause) {
      toast({ title: t("threads.deleteFailed"), description: formatInvokeError(cause), variant: "destructive" });
    } finally { setCancelPendingBusy(false); }
  };

  const handleExport = async (keys: string[]) => {
    const availableKeys = keys.filter((key) => exportableKeys.has(key));
    if (availableKeys.length === 0) {
      toast({
        title: t("threads.exportUnavailable"),
        description: t("threads.exportUnavailableDesc"),
        variant: "destructive",
      });
      return;
    }
    const { open } = await import("@tauri-apps/plugin-dialog");
    const targetDir = await open({ directory: true, multiple: false });
    if (typeof targetDir !== "string") return;
    try {
      const result = await api.conversations.export("codex", availableKeys, targetDir, format);
      const payload = result.data;
      if (payload.exported === 0) {
        throw new Error(t("threads.nothingExported"));
      }
      toast({
        title: t("threads.exportDone", { count: payload.exported }),
        description: payload.directory,
        variant: "success",
      });
      setSelected((prev) => {
        const next = new Set(prev);
        for (const key of availableKeys) next.delete(key);
        return next;
      });
    } catch (error) {
      toast({
        title: t("threads.exportFailed"),
        description: formatInvokeError(error),
        variant: "destructive",
      });
    }
  };

  const handleOpenPath = async (path: string) => {
    try {
      await api.openPath(path);
    } catch (error) {
      toast({
        title: t("threads.openFailed"),
        description: formatInvokeError(error),
        variant: "destructive",
      });
    }
  };

  const handleCopyPath = (path: string, key: string) => {
    navigator.clipboard.writeText(path);
    setCopiedGroupPath(key);
    toast({
      title: t("threads.pathCopied"),
      description: path,
      variant: "default",
    });
    setTimeout(() => setCopiedGroupPath(null), 2000);
  };

  const sortLabels: Record<SortOption, string> = {
    updatedAt: t("threads.sortUpdated"),
    messages: t("threads.sortMessages"),
    fileSize: t("threads.sortSize"),
    createdAt: t("threads.sortCreated"),
  };

  return {
    threadsQuery, threads, pendingDeletionKeys, scanWarnings, pendingDeletionRecoveryNeeded,
    filtered, groups, selectedExportableCount, useVirtualList,
    archivedCount, totalUserMsgs, totalAgentMsgs, totalBytes, totalTokens, missingRolloutCount, totalInteractions,
    status, setStatus, query, setQuery, sortBy, setSortBy, sortLabels, viewMode, setViewMode, format, setFormat,
    selected, toggleSelect, selectFiltered, clearSelection, toggleGroupSelect,
    collapsedGroups, toggleGroupCollapse, expandAllGroups, collapseAllGroups,
    copiedGroupPath, recoveryTarget, setRecoveryTarget, confirmRequest, setConfirmRequest, bulkRepairing, deleteBusy, cancelPendingBusy, refreshAction,
    handleRefresh, handleRepairAll, handleDelete, cancelPendingDeletions, handleExport, handleOpenPath, handleCopyPath,
  };
}

export type ThreadList = ReturnType<typeof useThreadList>;
