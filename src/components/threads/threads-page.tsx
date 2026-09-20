import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import {
  Archive,
  CheckSquare,
  Download,
  FileDown,
  FileText,
  Folder,
  FolderOpen,
  MessageSquare,
  MessagesSquare,
  RotateCw,
  Search,
  Square,
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
import { toast } from "@/hooks/use-toast";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import { useBusyAction } from "@/hooks/use-busy-action";
import type { ThreadSummary } from "@/types/threads";

type StatusFilter = "all" | "active" | "archived";
type ExportFormat = "markdown" | "jsonl";

const THREADS_KEY = ["codex-threads"] as const;

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

interface FolderGroup {
  key: string;
  cwd: string;
  folderName: string;
  items: ThreadSummary[];
  latestAt: number;
}

export function ThreadsPage() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<StatusFilter>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [format, setFormat] = useState<ExportFormat>("markdown");
  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const threadsQuery = useQuery({
    queryKey: THREADS_KEY,
    queryFn: () => api.listCodexThreads(),
    staleTime: 30_000,
  });

  const threads = useMemo(() => threadsQuery.data?.data.items ?? [], [threadsQuery.data]);

  const filtered = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    return threads.filter((thread) => {
      if (status === "active" && thread.archived) return false;
      if (status === "archived" && !thread.archived) return false;
      if (!keyword) return true;
      return (
        thread.title.toLowerCase().includes(keyword) ||
        thread.cwd.toLowerCase().includes(keyword) ||
        thread.folderName.toLowerCase().includes(keyword)
      );
    });
  }, [threads, status, query]);

  const groups = useMemo<FolderGroup[]>(() => {
    const map = new Map<string, FolderGroup>();
    for (const thread of filtered) {
      const key = thread.cwd || t("threads.unknownFolder");
      const existing = map.get(key);
      if (existing) {
        existing.items.push(thread);
        existing.latestAt = Math.max(existing.latestAt, thread.updatedAt);
      } else {
        map.set(key, {
          key,
          cwd: thread.cwd,
          folderName: thread.folderName || key,
          items: [thread],
          latestAt: thread.updatedAt,
        });
      }
    }
    return [...map.values()].sort((a, b) => b.latestAt - a.latestAt);
  }, [filtered, t]);

  const archivedCount = useMemo(() => threads.filter((item) => item.archived).length, [threads]);

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

  const selectFiltered = () => setSelected(new Set(filtered.map((item) => item.id)));
  const clearSelection = () => setSelected(new Set());

  const handleRefresh = async () => {
    await refreshAction.run(async () => {
      await threadsQuery.refetch();
    });
  };

  const handleExport = async (ids: string[]) => {
    if (ids.length === 0) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const targetDir = await open({ directory: true, multiple: false });
    if (typeof targetDir !== "string") return;
    try {
      const result = await api.exportCodexThreads(ids, targetDir, format);
      const payload = result.data;
      toast({
        title: t("threads.exportDone", { count: payload.exported }),
        description: payload.directory,
        variant: "success",
      });
      setSelected((prev) => {
        const next = new Set(prev);
        for (const id of ids) next.delete(id);
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

  const selectedInFiltered = filtered.filter((item) => selected.has(item.id)).length;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <h1 className="text-xl font-semibold tracking-tight">{t("threads.title")}</h1>
          <p className="text-sm text-muted-foreground">{t("threads.subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <SegmentedOptions
            items={[
              { value: "markdown", label: t("threads.formatMarkdown") },
              { value: "jsonl", label: t("threads.formatJsonl") },
            ]}
            value={format}
            onChange={(value) => setFormat(value as ExportFormat)}
          />
          <RainbowButton
            className="h-9 rounded-[10px] px-3.5 text-[13px]"
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

      {/* Toolbar: search / status / selection */}
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative w-full max-w-xs">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/70" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("threads.searchPlaceholder")}
            className="h-8 pl-8 pr-7 text-[13px]"
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
        <div className="ml-auto flex items-center gap-2">
          <Button variant="outline" size="sm" className="h-8 gap-1.5 text-xs" onClick={selectFiltered}>
            <CheckSquare className="h-3.5 w-3.5" />
            {t("threads.selectAll")}
          </Button>
          {selected.size > 0 && (
            <>
              <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-xs" onClick={clearSelection}>
                <Square className="h-3.5 w-3.5" />
                {t("threads.clearSelection")}
              </Button>
              <RainbowButton
                className="h-8 rounded-[9px] px-3 text-xs"
                onClick={() => void handleExport([...selected])}
              >
                <Download className="h-3.5 w-3.5" />
                {t("threads.exportSelected", { count: selected.size })}
              </RainbowButton>
            </>
          )}
        </div>
      </div>

      {/* Stats strip */}
      <AuroraCard color="blue" className="px-4 py-3">
        <div className="flex flex-wrap items-center gap-x-6 gap-y-1 text-[12px] text-muted-foreground">
          <span className="inline-flex items-center gap-1.5">
            <MessagesSquare className="h-3.5 w-3.5 text-primary" />
            {t("threads.statsThreads", { count: threads.length })}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <Folder className="h-3.5 w-3.5 text-primary" />
            {t("threads.statsFolders", { count: groups.length })}
          </span>
          {selectedInFiltered > 0 && (
            <span className="inline-flex items-center gap-1.5 text-foreground">
              <CheckSquare className="h-3.5 w-3.5 text-primary" />
              {t("threads.statsSelected", { count: selected.size })}
            </span>
          )}
        </div>
      </AuroraCard>

      {/* Content */}
      {threadsQuery.isLoading ? (
        <div className="space-y-4">
          {Array.from({ length: 3 }).map((_, index) => (
            <div key={index} className="space-y-3">
              <Skeleton className="h-5 w-56" />
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                {Array.from({ length: 3 }).map((__, card) => (
                  <Skeleton key={card} className="h-28 rounded-2xl" />
                ))}
              </div>
            </div>
          ))}
        </div>
      ) : groups.length === 0 ? (
        <BentoCard className="flex flex-col items-center justify-center gap-2 py-16 text-center">
          <MessagesSquare className="h-8 w-8 text-muted-foreground/50" />
          <p className="text-sm font-medium">{t("threads.emptyTitle")}</p>
          <p className="max-w-md text-xs text-muted-foreground">{t("threads.emptyDesc")}</p>
        </BentoCard>
      ) : (
        groups.map((group) => (
          <section key={group.key} className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <div className="flex min-w-0 items-center gap-2">
                <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-primary/10 text-primary">
                  <Folder className="h-3.5 w-3.5" />
                </span>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <h2 className="truncate text-sm font-semibold">{group.folderName}</h2>
                    <span className="shrink-0 text-[11px] text-muted-foreground">
                      {t("threads.threadCount", { count: group.items.length })}
                    </span>
                  </div>
                  <p className="truncate font-mono text-[11px] text-muted-foreground/80">
                    {group.cwd}
                  </p>
                </div>
              </div>
              <div className="ml-auto flex items-center gap-1.5">
                {group.cwd && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-7 w-7 text-muted-foreground hover:text-foreground"
                    title={t("threads.openFolder")}
                    onClick={() => void api.openPath(group.cwd)}
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                  </Button>
                )}
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1.5 px-2.5 text-xs"
                  onClick={() => void handleExport(group.items.map((item) => item.id))}
                >
                  <FileDown className="h-3.5 w-3.5" />
                  {t("threads.exportFolder")}
                </Button>
              </div>
            </div>
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {group.items.map((thread) => (
                <ThreadCard
                  key={thread.id}
                  thread={thread}
                  selected={selected.has(thread.id)}
                  onToggleSelect={() => toggleSelect(thread.id)}
                  onExport={() => void handleExport([thread.id])}
                />
              ))}
            </div>
          </section>
        ))
      )}
    </div>
  );
}


function ThreadCard({
  thread,
  selected,
  onToggleSelect,
  onExport,
}: {
  thread: ThreadSummary;
  selected: boolean;
  onToggleSelect: () => void;
  onExport: () => void;
}) {
  const { t } = useTranslation();
  const displayTitle = thread.title || t("threads.untitled");
  return (
    <BentoCard
      className={cn(
        "group cursor-pointer p-4 transition-shadow",
        selected && "ring-2 ring-primary/60",
      )}
      onClick={onToggleSelect}
    >
      <div className="flex items-start gap-2.5">
        <span
          className={cn(
            "mt-0.5 flex h-4.5 w-4.5 shrink-0 items-center justify-center rounded-[5px] border transition-colors",
            selected
              ? "border-primary bg-primary text-primary-foreground"
              : "border-border text-transparent group-hover:border-primary/50",
          )}
        >
          <CheckSquare className="h-3 w-3" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <p className="truncate text-[13px] font-medium leading-snug">{displayTitle}</p>
            {thread.archived && (
              <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                <Archive className="h-2.5 w-2.5" />
                {t("threads.archivedBadge")}
              </span>
            )}
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
            <span className="inline-flex items-center gap-1" title={t("threads.messagesTooltip")}>
              <MessageSquare className="h-3 w-3 opacity-60" />
              {thread.userMessages} / {thread.agentMessages}
            </span>
            <span title={formatDateTime(thread.updatedAt)}>{formatRelative(thread.updatedAt)}</span>
            <span>{formatSize(thread.fileSize)}</span>
            {thread.source && <span className="font-mono opacity-70">{thread.source}</span>}
          </div>
        </div>
        <div className="flex shrink-0 flex-col gap-1 opacity-0 transition-opacity group-hover:opacity-100">
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-6 w-6 text-muted-foreground hover:text-foreground"
            title={t("threads.exportThread")}
            onClick={(event) => {
              event.stopPropagation();
              onExport();
            }}
          >
            <FileText className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-6 w-6 text-muted-foreground hover:text-foreground"
            title={t("threads.openRawFile")}
            onClick={(event) => {
              event.stopPropagation();
              void api.openPath(thread.filePath);
            }}
          >
            <FolderOpen className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </BentoCard>
  );
}

