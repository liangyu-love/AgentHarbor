import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowUpDown,
  Check,
  CheckSquare,
  ChevronDown,
  ChevronRight,
  Clock3,
  Copy,
  Download,
  FileDown,
  Folder,
  FolderOpen,
  FolderTree,
  HardDrive,
  LayoutGrid,
  List,
  MessagesSquare,
  RotateCw,
  Search,
  Sparkles,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { BentoCard } from "@/components/ui/bento-card";
import { AuroraCard } from "@/components/ui/aurora-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { Input } from "@/components/ui/input";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { toast } from "@/hooks/use-toast";
import { useBusyAction } from "@/hooks/use-busy-action";
import { formatRelative } from "@/lib/format-time";
import { ThreadCard } from "@/components/threads/thread-card";
import { ThreadRecoveryDialog } from "@/components/threads/thread-recovery-dialog";
import { VirtualThreadList } from "@/components/threads/virtual-thread-list";
import type { ThreadSummary } from "@/types/threads";
import {
  filterAndSortThreads,
  formatSize,
  formatTokenCount,
  groupThreads,
  type SortOption,
  type StatusFilter,
} from "@/components/threads/thread-utils";

type ExportFormat = "markdown" | "jsonl";
type ViewMode = "grid" | "compact";

const THREADS_KEY = ["threads", "codex"] as const;
/** Window the list once dense rows would thrash layout/paint. */
const VIRTUALIZE_THRESHOLD = 80;

export function ThreadsPage() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<StatusFilter>("all");
  const [query, setQuery] = useState("");
  const [sortBy, setSortBy] = useState<SortOption>("updatedAt");
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    const saved = localStorage.getItem("threads_view_mode");
    return saved === "grid" || saved === "compact" ? saved : "compact";
  });
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [format, setFormat] = useState<ExportFormat>("markdown");
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [copiedGroupPath, setCopiedGroupPath] = useState<string | null>(null);
  const [recoveryTarget, setRecoveryTarget] = useState<ThreadSummary | null>(null);
  const [bulkRepairing, setBulkRepairing] = useState(false);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [cancelPendingBusy, setCancelPendingBusy] = useState(false);

  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const threadsQuery = useQuery({
    queryKey: THREADS_KEY,
    queryFn: () => api.conversations.list("codex"),
    staleTime: 30_000,
    refetchInterval: query => query.state.data?.data.pendingDeletionKeys?.length ? 20_000 : false,
  });

  const threads = useMemo(() => threadsQuery.data?.data.items ?? [], [threadsQuery.data]);
  const pendingDeletionKeys = new Set(threadsQuery.data?.data.pendingDeletionKeys ?? []);
  const scanWarnings = threadsQuery.data?.warnings ?? [];
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

  // Clean stale selection when threads change
  useEffect(() => {
    const available = new Set(threads.map((thread) => thread.key));
    setSelected((previous) => {
      const next = new Set([...previous].filter((key) => available.has(key)));
      return next.size === previous.size ? previous : next;
    });
  }, [threads]);

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
    localStorage.setItem("threads_view_mode", viewMode);
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
    if (!window.confirm(t("threads.repairAllConfirm", { count: candidates.length }))) return;
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
    if (!window.confirm(t("threads.deleteConfirm", { count: uniqueKeys.length }))) return;
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
      toast({ title: t("threads.deleteFailed"), description: String(cause), variant: "destructive" });
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
        description: String(error),
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
        description: String(error),
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

  return (
    <TooltipProvider delayDuration={200}>
      <div className="space-y-3">
        {/* Page Header */}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="space-y-0.5">
            <h1 className="text-lg font-bold tracking-tight text-foreground flex items-center gap-2">
              <MessagesSquare className="h-4.5 w-4.5 text-primary" />
              {t("threads.title")}
            </h1>
          </div>

          <div className="flex items-center gap-2.5">
            {/* Format Selector */}
            <SegmentedOptions
              items={[
                { value: "markdown", label: t("threads.formatMarkdown") },
                { value: "jsonl", label: t("threads.formatJsonl") },
              ]}
              value={format}
              onChange={(value) => setFormat(value as ExportFormat)}
            />

            <Button
              variant="outline"
              size="sm"
              className="h-9 gap-1.5 rounded-[10px] px-3 text-[13px]"
              onClick={() => void handleRepairAll()}
              disabled={bulkRepairing || threads.length === 0}
            >
              {bulkRepairing ? (
                <RotateCw className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Sparkles className="h-3.5 w-3.5" />
              )}
              {bulkRepairing ? t("threads.repairingAll") : t("threads.repairAll")}
            </Button>

            {/* Refresh Button */}
            <RainbowButton
              className="h-9 rounded-[10px] px-3.5 text-[13px] font-medium"
              onClick={() => void handleRefresh()}
              disabled={refreshAction.busy}
            >
              <ButtonBusyContent
                busy={refreshAction.busy}
                idleIcon={<RotateCw className="h-3.5 w-3.5" />}
                idleLabel={t("threads.refresh")}
              />
            </RainbowButton>
          </div>
        </div>

        {scanWarnings.length > 0 && (
          <div role="status" className="rounded-xl border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
            <details>
              <summary className="cursor-pointer font-medium">
                {t("threads.scanWarnings", { count: scanWarnings.length })}
              </summary>
              <ul className="mt-2 max-h-40 space-y-1 overflow-y-auto break-words text-xs text-muted-foreground">
                {scanWarnings.map((warning, index) => (
                  <li key={index}>{warning.message}</li>
                ))}
              </ul>
            </details>
          </div>
        )}

        {pendingDeletionKeys.size > 0 && (
          <div role="status" className="flex flex-wrap items-center gap-3 rounded-xl border border-amber-500/30 bg-amber-500/5 px-3 py-2.5 text-sm text-foreground motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-200">
            <Clock3 className="h-4 w-4 shrink-0 text-amber-700 dark:text-amber-400" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <p className="font-medium">{t("threads.deletePending", { count: pendingDeletionKeys.size })}</p>
              <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{t("threads.deletePendingDetail")}</p>
            </div>
            <Button type="button" size="sm" variant="outline" disabled={cancelPendingBusy} onClick={() => void cancelPendingDeletions()}>
              {t("threads.cancelPendingDeletion")}
            </Button>
          </div>
        )}

        {missingRolloutCount > 0 && (
          <div role="status" className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-sm">
            <AlertTriangle className="h-4 w-4 shrink-0 text-destructive" />
            <p className="font-medium text-destructive">
              {t("threads.rolloutMissingSummary", { count: missingRolloutCount })}
            </p>
          </div>
        )}

        {/* 4-Metric Mission Control HUD */}
        <div className="app-metric-grid grid grid-cols-2 gap-2 lg:grid-cols-4">
          <AuroraCard color="blue" className="cursor-pointer hover:border-primary/35" onClick={() => setStatus(status === "active" ? "all" : "active")}>
            <div className="flex items-center justify-between p-2.5">
              <div className="space-y-0.5">
                <span className="text-xs font-medium text-muted-foreground">
                  {t("threads.statsThreads")}
                </span>
                <div className="flex items-baseline gap-2">
                  <span className="text-xl font-bold tracking-tight tabular-nums">
                    {threads.length}
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    {threads.length - archivedCount} 进行中 · {archivedCount} 归档
                  </span>
                </div>
              </div>
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <MessagesSquare className="h-5 w-5" />
              </div>
            </div>
          </AuroraCard>

          <AuroraCard color="blue">
            <div className="flex items-center justify-between p-2.5">
              <div className="space-y-0.5">
                <span className="text-xs font-medium text-muted-foreground">
                  {t("threads.statsFolders")}
                </span>
                <div className="flex items-baseline gap-2">
                  <span className="text-xl font-bold tracking-tight tabular-nums">
                    {groups.length}
                  </span>
                </div>
              </div>
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <FolderTree className="h-5 w-5" />
              </div>
            </div>
          </AuroraCard>

          <AuroraCard color="blue">
            <div className="flex items-center justify-between p-2.5">
              <div className="space-y-0.5">
                <span className="text-xs font-medium text-muted-foreground">
                  {t("threads.statsInteractions")}
                </span>
                <div className="flex items-baseline gap-2">
                  <span className="text-xl font-bold tracking-tight tabular-nums">
                    {totalInteractions.toLocaleString()}
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    U: {totalUserMsgs} · AI: {totalAgentMsgs} · {formatTokenCount(totalTokens)} {t("threads.tokenUnit")}
                  </span>
                </div>
              </div>
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <Sparkles className="h-5 w-5" />
              </div>
            </div>
          </AuroraCard>

          <AuroraCard
            color="blue"
            shine={selected.size > 0}
            className="cursor-pointer hover:border-primary/35"
            onClick={() => {
              if (selected.size > 0) clearSelection();
              else selectFiltered();
            }}
          >
            <div className="flex items-center justify-between p-2.5">
              <div className="space-y-0.5">
                <span className="text-xs font-medium text-muted-foreground">
                  {selected.size > 0 ? t("threads.statsSelected", { count: selected.size }) : t("threads.statsStorage")}
                </span>
                <div className="flex items-baseline gap-2">
                  <span className="text-xl font-bold tracking-tight tabular-nums">
                    {selected.size > 0 ? selected.size : formatSize(totalBytes)}
                  </span>
                  {selected.size > 0 && <span className="text-[11px] text-muted-foreground">点击取消全选</span>}
                </div>
              </div>
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
                {selected.size > 0 ? <CheckSquare className="h-5 w-5" /> : <HardDrive className="h-5 w-5" />}
              </div>
            </div>
          </AuroraCard>
        </div>

        {/* Power Toolbar */}
        <div className="flex flex-wrap items-center justify-between gap-3">
          {/* Left: Search & Filter Tabs */}
          <div className="flex flex-wrap items-center gap-2.5">
            {/* Search Box */}
            <div className="relative w-full sm:w-64">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/70" />
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("threads.searchPlaceholder")}
                className="h-8.5 pl-8 pr-7 text-[13px] bg-background/80"
              />
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground/70 hover:text-foreground"
                  aria-label={t("common.cancel")}
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>

            {/* Status Tabs */}
            <SegmentedOptions
              items={[
                { value: "all", label: t("threads.filterAll", { count: threads.length }) },
                {
                  value: "active",
                  label: t("threads.filterActive", { count: threads.length - archivedCount }),
                },
                { value: "archived", label: t("threads.filterArchived", { count: archivedCount }) },
              ]}
              value={status}
              onChange={(value) => setStatus(value as StatusFilter)}
            />

            {/* Sorting Dropdown */}
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="sm" className="h-8.5 gap-1.5 px-2.5 text-xs">
                  <ArrowUpDown className="h-3 w-3 text-muted-foreground" />
                  <span>{sortLabels[sortBy]}</span>
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-36">
                {(["updatedAt", "messages", "fileSize", "createdAt"] as SortOption[]).map((option) => (
                  <DropdownMenuItem
                    key={option}
                    onClick={() => setSortBy(option)}
                    className="flex items-center justify-between text-xs"
                  >
                    <span>{sortLabels[option]}</span>
                    {sortBy === option && <Check className="h-3.5 w-3.5 text-primary" />}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>

          {/* Right: View mode & Folder Fold / Batch Select */}
          <div className="flex items-center gap-2">
            {/* View Mode Switcher */}
            <div className="flex items-center rounded-lg border border-border/70 p-0.5 bg-muted/20">
              <Button
                variant={viewMode === "grid" ? "secondary" : "ghost"}
                size="icon-sm"
                className="h-7 w-7 rounded-md"
                title={t("threads.viewGrid")}
                onClick={() => setViewMode("grid")}
              >
                <LayoutGrid className="h-3.5 w-3.5" />
              </Button>
              <Button
                variant={viewMode === "compact" ? "secondary" : "ghost"}
                size="icon-sm"
                className="h-7 w-7 rounded-md"
                title={t("threads.viewCompact")}
                onClick={() => setViewMode("compact")}
              >
                <List className="h-3.5 w-3.5" />
              </Button>
            </div>

            {/* Fold / Unfold All */}
            <Button
              variant="outline"
              size="sm"
              className="h-8.5 gap-1 text-xs"
              onClick={collapsedGroups.size > 0 ? expandAllGroups : collapseAllGroups}
            >
              {collapsedGroups.size > 0 ? (
                <>
                  <ChevronDown className="h-3.5 w-3.5" />
                  {t("threads.expandAll")}
                </>
              ) : (
                <>
                  <ChevronRight className="h-3.5 w-3.5" />
                  {t("threads.collapseAll")}
                </>
              )}
            </Button>

            {/* Select All in filtered */}
            <Button
              variant="outline"
              size="sm"
              className="h-8.5 gap-1.5 text-xs"
              onClick={selectFiltered}
            >
              <CheckSquare className="h-3.5 w-3.5" />
              {t("threads.selectAll")}
            </Button>
          </div>
        </div>

        {/* Dynamic Sticky Batch Selection Bar */}
        {selected.size > 0 && (
          <div className="sticky bottom-4 z-30 transition-all duration-300 animate-in fade-in slide-in-from-bottom-3">
            <BentoCard className="flex flex-wrap items-center justify-between gap-3 border-primary/50 bg-card/95 backdrop-blur-md p-3 shadow-2xl ring-1 ring-primary/20 dark:border-amber-400/40 dark:ring-amber-400/20">
            <div className="flex items-center gap-2 text-xs font-medium">
              <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-primary-foreground text-[11px] font-bold">
                {selected.size}
              </span>
              <span>{t("threads.statsSelected", { count: selected.size })}</span>
              <span className="text-muted-foreground/60">·</span>
              <span className="text-muted-foreground font-mono text-[11px]">
                导出格式: {format === "markdown" ? "Markdown (.md)" : "原始 JSONL (.jsonl)"}
              </span>
            </div>

            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                className="h-7.5 gap-1 text-xs text-muted-foreground hover:text-foreground"
                onClick={clearSelection}
              >
                <Square className="h-3 w-3" />
                {t("threads.clearSelection")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                className="h-7.5 gap-1 px-2.5 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
                onClick={() => void handleDelete([...selected])}
                disabled={deleteBusy}
              >
                {deleteBusy ? (
                  <RotateCw className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Trash2 className="h-3.5 w-3.5" />
                )}
                {t("threads.deleteSelected", { count: selected.size })}
              </Button>
              <RainbowButton
                className="h-7.5 rounded-[8px] px-3 text-xs font-medium"
                onClick={() => void handleExport([...selected])}
                disabled={selectedExportableCount === 0}
              >
                <Download className="h-3.5 w-3.5" />
                {t("threads.exportSelected", { count: selectedExportableCount })}
              </RainbowButton>
            </div>
          </BentoCard>
          </div>
        )}

        {/* Content Section */}
        {threadsQuery.isLoading ? (
          <div className="space-y-3">
            {Array.from({ length: 2 }).map((_, index) => (
              <div key={index} className="space-y-1.5">
                <Skeleton className="h-7 w-56 rounded-lg" />
                <div className="grid gap-2.5 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4">
                  {Array.from({ length: 4 }).map((__, card) => (
                    <Skeleton key={card} className="h-40 rounded-lg" />
                  ))}
                </div>
              </div>
            ))}
          </div>
        ) : threadsQuery.isError ? (
          <BentoCard className="flex flex-col items-center justify-center gap-3 py-16 text-center">
            <MessagesSquare className="h-10 w-10 text-destructive/70" />
            <p className="text-sm font-semibold">{t("threads.loadFailed")}</p>
            <p className="max-w-md text-xs text-muted-foreground font-mono">
              {String(threadsQuery.error)}
            </p>
            <Button variant="outline" size="sm" onClick={() => void handleRefresh()}>
              {t("threads.retry")}
            </Button>
          </BentoCard>
        ) : groups.length === 0 ? (
          <BentoCard className="flex flex-col items-center justify-center gap-3 py-16 text-center">
            <MessagesSquare className="h-10 w-10 text-muted-foreground/40" />
            <p className="text-sm font-semibold">
              {query ? t("threads.noFilteredResults") : t("threads.emptyTitle")}
            </p>
            <p className="max-w-md text-xs text-muted-foreground">
              {query ? t("threads.noFilteredResultsDesc") : t("threads.emptyDesc")}
            </p>
            {query && (
              <Button
                variant="outline"
                size="sm"
                className="mt-1 h-8 gap-1.5 text-xs"
                onClick={() => setQuery("")}
              >
                <X className="h-3.5 w-3.5" />
                {t("threads.clearSearch")}
              </Button>
            )}
          </BentoCard>
        ) : useVirtualList ? (
          <VirtualThreadList
            groups={groups}
            collapsedGroups={collapsedGroups}
            selected={selected}
            viewMode={viewMode}
            onToggleSelect={toggleSelect}
            onToggleGroup={toggleGroupSelect}
            onToggleCollapse={toggleGroupCollapse}
            onExport={keys => void handleExport(keys)}
            onOpenPath={path => void handleOpenPath(path)}
            onRecover={setRecoveryTarget}
            onDelete={(thread) => void handleDelete([thread.key])}
            onCopyPath={handleCopyPath}
            copiedGroupPath={copiedGroupPath}
            t={t}
          />
        ) : (
          groups.map((group) => {
            const isCollapsed = collapsedGroups.has(group.key);
            const groupKeys = group.items.map((item) => item.key);
            const groupExportKeys = group.items
              .filter((item) => item.rolloutStatus !== "missing")
              .map((item) => item.key);
            const allInGroupSelected = groupKeys.length > 0 && groupKeys.every((key) => selected.has(key));
            const someInGroupSelected = groupKeys.some((key) => selected.has(key)) && !allInGroupSelected;

            return (
              <section key={group.key} className="space-y-1.5">
                {/* Workspace Folder Header Bar */}
                <div className={cn(
                  "flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-card/75 p-1.5 px-2.5 backdrop-blur-sm transition-all duration-150 hover:border-primary/45",
                  allInGroupSelected ? "border-primary/60 ring-1 ring-primary/25 bg-primary/[0.02]" : "border-border/60",
                )}>
                  {/* Left: Fold button + Folder icon + Folder Info */}
                  <div className="flex min-w-0 flex-1 items-center gap-2.5">
                    <button
                      type="button"
                      onClick={() => toggleGroupCollapse(group.key)}
                      className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground transition-all duration-200"
                      title={isCollapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
                      aria-label={isCollapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
                    >
                      <ChevronRight className={cn("h-4 w-4 transition-transform duration-200", !isCollapsed && "rotate-90")} />
                    </button>

                    <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-primary/10 text-primary">
                      <Folder className="h-3.5 w-3.5" />
                    </div>

                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <h2 className="truncate text-sm font-semibold tracking-tight text-foreground">
                          {group.folderName}
                        </h2>

                        {/* Badges */}
                        <span className="rounded-md bg-muted px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground">
                          {t("threads.threadCount", { count: group.items.length })}
                        </span>

                        <span className="hidden text-[10px] text-muted-foreground/80 sm:inline-block">
                          · {group.totalMessages} {t("threads.groupMessages")}
                        </span>

                        <span className="hidden text-[10px] text-muted-foreground/80 sm:inline-block">
                          · {formatSize(group.totalSize)}
                        </span>

                        <span className="hidden text-[10px] text-muted-foreground/70 md:inline-block">
                          · 活跃于 {formatRelative(group.latestAt)}
                        </span>
                      </div>

                      {/* Path with 1-click copy */}
                      {group.cwd && (
                        <div className="flex items-center gap-1.5">
                          <p
                            className="truncate font-mono text-[11px] text-muted-foreground/75 cursor-pointer hover:text-foreground"
                            onClick={() => handleCopyPath(group.cwd, group.key)}
                            title={group.cwd}
                          >
                            {group.cwd}
                          </p>
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <button
                                type="button"
                                onClick={() => handleCopyPath(group.cwd, group.key)}
                                className="text-muted-foreground/60 hover:text-foreground"
                              >
                                {copiedGroupPath === group.key ? (
                                  <Check className="h-3 w-3 text-emerald-500" />
                                ) : (
                                  <Copy className="h-3 w-3" />
                                )}
                              </button>
                            </TooltipTrigger>
                            <TooltipContent>{t("threads.copyPath")}</TooltipContent>
                          </Tooltip>
                        </div>
                      )}
                    </div>
                  </div>

                  {/* Right: Group Actions */}
                  <div className="flex items-center gap-1.5 shrink-0">
                    {/* Select All in Group Toggle */}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant={allInGroupSelected ? "secondary" : "ghost"}
                          size="sm"
                          className="h-7 gap-1 px-2 text-xs"
                          onClick={() => toggleGroupSelect(groupKeys)}
                        >
                          <CheckSquare
                            className={`h-3.5 w-3.5 ${
                              allInGroupSelected || someInGroupSelected ? "text-primary" : "text-muted-foreground"
                            }`}
                          />
                          <span className="hidden sm:inline">
                            {allInGroupSelected ? t("threads.deselectGroup") : t("threads.selectGroup")}
                          </span>
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>
                        {allInGroupSelected ? t("threads.deselectGroup") : t("threads.selectGroup")}
                      </TooltipContent>
                    </Tooltip>

                    {/* Open in File Explorer */}
                    {group.cwd && (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            className="h-7 w-7 text-muted-foreground hover:bg-primary/10 hover:text-primary"
                            onClick={() => void handleOpenPath(group.cwd)}
                          >
                            <FolderOpen className="h-3.5 w-3.5" />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>{t("threads.openFolder")}</TooltipContent>
                      </Tooltip>
                    )}

                    {/* Export Workspace */}
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          variant="outline"
                          size="sm"
                          className="h-7 gap-1.5 px-2.5 text-xs"
                          onClick={() => void handleExport(groupExportKeys)}
                          disabled={groupExportKeys.length === 0}
                        >
                          <FileDown className="h-3.5 w-3.5" />
                          <span className="hidden md:inline">{t("threads.exportFolder")}</span>
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>
                        {groupExportKeys.length === 0
                          ? t("threads.exportUnavailable")
                          : t("threads.exportFolder")}
                      </TooltipContent>
                    </Tooltip>
                  </div>
                </div>

                {/* Group Content (Collapsible) */}
                {!isCollapsed && (
                  <div
                    className={
                      viewMode === "grid"
                        ? "grid gap-2.5 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4 animate-in fade-in duration-200"
                        : "space-y-1 animate-in fade-in duration-200"
                    }
                  >
                    {group.items.map((thread) => (
                      <ThreadCard
                        key={thread.key}
                        thread={thread}
                        selected={selected.has(thread.key)}
                        onToggleSelect={() => toggleSelect(thread.key)}
                        onExport={() => void handleExport([thread.key])}
                        onOpenRaw={() => void handleOpenPath(thread.filePath)}
                        onRecover={() => setRecoveryTarget(thread)}
                        onDelete={() => void handleDelete([thread.key])}
                        variant={viewMode}
                      />
                    ))}
                  </div>
                )}
              </section>
            );
          })
        )}
        <ThreadRecoveryDialog
          thread={recoveryTarget}
          onOpenChange={(open) => !open && setRecoveryTarget(null)}
          onRecovered={async () => {
            await threadsQuery.refetch();
          }}
        />
      </div>
    </TooltipProvider>
  );
}
