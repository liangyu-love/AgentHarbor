import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { useBusyAction } from "@/hooks/use-busy-action";
import type { ConversationSource, ThreadBatchCandidate, ThreadSummary } from "@/types/threads";
import {
  filterAndSortThreads,
  groupThreads,
  threadProjectKey,
  DIRECT_CHATS_KEY,
  type SortOption,
  type StatusFilter,
} from "@/components/threads/thread-utils";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import type { ConfirmRequest } from "@/components/ui/confirm-dialog";

export type ExportFormat = "markdown" | "jsonl";
export type ViewMode = "grid" | "compact";

/** Window the list once dense rows would thrash layout/paint. */
const VIRTUALIZE_THRESHOLD = 80;

// Thread scan state, filters, selection and the batch actions behind the threads page.
export function useThreadList(source: ConversationSource = "codex", setSource: (source: ConversationSource) => void = () => {}) {
  const { t } = useTranslation();
  const isCodex = source === "codex";
  const [status, setStatus] = useState<StatusFilter>("all");
  const [query, setQuery] = useState("");
  const [projectKey, setProjectKey] = useState<string | null>(null);
  const [focusedThreadKey, setFocusedThreadKey] = useState<string | null>(null);
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
  const [detailKey, setDetailKey] = useState<string | null>(null);
  const [bulkRepairing, setBulkRepairing] = useState(false);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [cancelPendingBusy, setCancelPendingBusy] = useState(false);

  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const threadsQuery = useQuery({
    queryKey: queryKeys.conversations(source),
    queryFn: () => api.conversations.list(source),
    staleTime: 30_000,
    refetchInterval: query => query.state.data?.data.pendingDeletionKeys?.length
      || query.state.data?.data.pendingDeletionRecoveryNeeded ? 20_000 : false,
  });

  const threads = useMemo(() => threadsQuery.data?.data.items ?? [], [threadsQuery.data]);
  const detailTarget = threads.find(thread => thread.key === detailKey) ?? null;
  useEffect(() => { if (detailKey && !detailTarget) setDetailKey(null); }, [detailKey, detailTarget]);
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
  const matching = useMemo(
    () => filterAndSortThreads(threads, status, query, sortBy),
    [threads, status, query, sortBy],
  );
  const projects = useMemo(() => groupThreads(threads, t("threads.unknownFolder"), t("threads.directChats")), [threads, t]);
  const matchingKeys = useMemo(() => new Set(matching.map(thread => thread.key)), [matching]);
  const activeProject = projects.find(project => project.key === projectKey) ?? null;
  const focusCandidate = matching.find(thread => thread.key === focusedThreadKey);
  const focusedThread = focusCandidate && activeProject && threadProjectKey(focusCandidate) === activeProject.key ? focusCandidate : null;
  const filtered = useMemo(() => matching.filter(thread =>
    (!activeProject || threadProjectKey(thread) === activeProject.key)
    && (!focusedThread || thread.key === focusedThread.key)), [matching, activeProject, focusedThread]);
  // Filters/refetch may remove the focused item or its project; never leave a stale target.
  useEffect(() => {
    if (projectKey && !activeProject) setProjectKey(null);
    if (focusedThreadKey && !focusedThread) setFocusedThreadKey(null);
  }, [projectKey, activeProject, focusedThreadKey, focusedThread]);
  const selectProject = (key: string | null) => {
    setProjectKey(key);
    setFocusedThreadKey(null);
    setCollapsedGroups(new Set());
  };
  const focusThread = (project: string, key: string) => {
    setProjectKey(project);
    setFocusedThreadKey(key);
    setCollapsedGroups(new Set());
  };

  // Group by Workspace
  const groups = useMemo(
    () => groupThreads(filtered, t("threads.unknownFolder"), t("threads.directChats")),
    [filtered, t],
  );

  // The overview follows project selection, independent of list search/status/focus.
  const scopedThreads = activeProject?.items ?? threads;
  const workspaceCount = (activeProject ? [activeProject] : projects)
    .filter(project => project.key !== DIRECT_CHATS_KEY && project.cwd).length;
  const missingRolloutCount = threads.filter(thread => thread.rolloutStatus === "missing").length;
  const {
    archivedCount, totalBytes, totalTokens,
    totalInputTokens, totalCachedTokens, totalOutputTokens, hasTokenBreakdown,
  } = useMemo(() => {
    let archived = 0;
    let bytes = 0;
    let tokens = 0;
    let inputTokens = 0;
    let cachedTokens = 0;
    let outputTokens = 0;
    let breakdown = false;
    for (const item of scopedThreads) {
      if (item.archived) archived++;
      bytes += item.fileSize || 0;
      tokens += item.tokensUsed || 0;
      if (item.inputTokens != null || item.outputTokens != null) breakdown = true;
      inputTokens += item.inputTokens ?? 0;
      cachedTokens += item.cachedTokens ?? 0;
      outputTokens += item.outputTokens ?? 0;
    }
    return {
      archivedCount: archived,
      totalBytes: bytes,
      totalTokens: tokens,
      totalInputTokens: inputTokens,
      totalCachedTokens: cachedTokens,
      totalOutputTokens: outputTokens,
      hasTokenBreakdown: breakdown,
    };
  }, [scopedThreads]);

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

  const routeOverviewQuery = useQuery({
    queryKey: ["threads", "codex", "routes"],
    queryFn: api.conversations.routeOverview,
    enabled: isCodex,
    staleTime: 30_000,
  });
  const routeStates = routeOverviewQuery.data?.states ?? {};
  const routerActive = routeOverviewQuery.data?.routerActive ?? false;
  const unroutedIds = useMemo(() => [...new Set(threads
    .filter(thread => thread.id && routeStates[thread.id] && routeStates[thread.id] !== "routed")
    .map(thread => thread.id))], [threads, routeStates]);

  // Batch adoption into the Router: preview, confirm, then one backend call
  // that stops Codex once. A single failure never stops the remaining threads.
  const handleRepairAll = async () => {
    if (!isCodex || bulkRepairing || unroutedIds.length === 0) return;
    setBulkRepairing(true);
    try {
      const preview = await api.conversations.previewBatch("router", { threadIds: unroutedIds });
      if (preview.ready.length === 0) {
        toast({
          title: t("threads.repairAllNothing"),
          description: t("threads.repairAllSkipped", { count: preview.skipped.length }),
        });
        return;
      }
      const providers = Object.entries(preview.providers)
        .map(([provider, count]) => `${provider || t("threads.unknownProvider")} × ${count}`)
        .join(" · ");
      setConfirmRequest({
        title: t("threads.repairAll"),
        description: t("threads.repairAllConfirm", {
          count: preview.ready.length,
          skipped: preview.skipped.length,
          providers,
        }),
        confirmLabel: t("threads.repairAllApply"),
        onConfirm: () => void applyBatch(preview.ready),
      });
    } catch (cause) {
      toast({ title: t("threads.recoveryFailed"), description: formatInvokeError(cause), variant: "destructive" });
    } finally {
      setBulkRepairing(false);
    }
  };

  const applyBatch = async (ready: ThreadBatchCandidate[]) => {
    setBulkRepairing(true);
    try {
      const result = await api.conversations.applyBatch("router", ready);
      await Promise.all([threadsQuery.refetch(), routeOverviewQuery.refetch()]);
      toast({
        title: t("threads.repairAllDone"),
        description: t("threads.repairAllResult", {
          repaired: result.changed,
          skipped: result.unchanged,
          failed: result.failed.length,
        }),
        variant: result.failed.length === 0 ? "success" : "default",
      });
      if (result.reopenError) {
        toast({ title: t("threads.routerAdoptionOpenFailed"),
          description: t("threads.routerAdoptionOpenFailedDesc"), variant: "destructive" });
      }
    } catch (cause) {
      toast({ title: t("threads.recoveryFailed"), description: formatInvokeError(cause), variant: "destructive" });
    } finally {
      setBulkRepairing(false);
    }
  };

  const handleDelete = async (keys: string[]) => {
    if (!isCodex) return;
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
      const result = await api.conversations.delete(source, uniqueKeys);
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
    if (!isCodex || cancelPendingBusy) return;
    setCancelPendingBusy(true);
    try {
      const result = await api.conversations.cancelPendingDeletions(source);
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
      const result = await api.conversations.export(source, availableKeys, targetDir, format);
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
    source, setSource, isCodex, sourceBusy: bulkRepairing || deleteBusy || cancelPendingBusy,
    threadsQuery, threads, pendingDeletionKeys, scanWarnings, pendingDeletionRecoveryNeeded,
    filtered, groups, projects, matchingKeys, activeProject, focusedThread, selectProject, focusThread, scopedThreads, workspaceCount,
    selectedExportableCount, useVirtualList,
    archivedCount, totalBytes, totalTokens,
    totalInputTokens, totalCachedTokens, totalOutputTokens, hasTokenBreakdown,
    missingRolloutCount, routeStates, routerActive, unroutedIds, routeOverviewQuery,
    status, setStatus, query, setQuery, sortBy, setSortBy, sortLabels, viewMode, setViewMode, format, setFormat,
    selected, toggleSelect, selectFiltered, clearSelection, toggleGroupSelect,
    collapsedGroups, toggleGroupCollapse, expandAllGroups, collapseAllGroups,
    copiedGroupPath, recoveryTarget, setRecoveryTarget, detailTarget, setDetailKey, confirmRequest, setConfirmRequest, bulkRepairing, deleteBusy, cancelPendingBusy, refreshAction,
    handleRefresh, handleRepairAll, handleDelete, cancelPendingDeletions, handleExport, handleOpenPath, handleCopyPath,
  };
}

export type ThreadList = ReturnType<typeof useThreadList>;
