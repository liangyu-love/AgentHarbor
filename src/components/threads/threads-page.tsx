import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import {
  CheckSquare,
  Download,
  FileDown,
  Folder,
  FolderOpen,
  MessagesSquare,
  RotateCw,
  Search,
  Square,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { BentoCard } from "@/components/ui/bento-card";
import { AuroraCard } from "@/components/ui/aurora-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { Input } from "@/components/ui/input";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/hooks/use-toast";
import { useBusyAction } from "@/hooks/use-busy-action";
import { ThreadCard } from "@/components/threads/thread-card";
import {
  filterThreads,
  groupThreads,
  type StatusFilter,
} from "@/components/threads/thread-utils";

type ExportFormat = "markdown" | "jsonl";

const THREADS_KEY = ["threads", "codex"] as const;

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

  const filtered = useMemo(() => filterThreads(threads, status, query), [threads, status, query]);
  const groups = useMemo(
    () => groupThreads(filtered, t("threads.unknownFolder")),
    [filtered, t],
  );

  const archivedCount = useMemo(() => threads.filter((item) => item.archived).length, [threads]);

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

  const handleRefresh = async () => {
    await refreshAction.run(async () => {
      await threadsQuery.refetch();
    });
  };

  const handleExport = async (keys: string[]) => {
    if (keys.length === 0) return;
    const { open } = await import("@tauri-apps/plugin-dialog");
    const targetDir = await open({ directory: true, multiple: false });
    if (typeof targetDir !== "string") return;
    try {
      const result = await api.exportCodexThreads(keys, targetDir, format);
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
        for (const key of keys) next.delete(key);
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

  const selectedInFiltered = filtered.filter((item) => selected.has(item.key)).length;

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
      ) : threadsQuery.isError ? (
        <BentoCard className="flex flex-col items-center justify-center gap-3 py-16 text-center">
          <MessagesSquare className="h-8 w-8 text-destructive/70" />
          <p className="text-sm font-medium">{t("threads.loadFailed")}</p>
          <p className="max-w-md text-xs text-muted-foreground">{String(threadsQuery.error)}</p>
          <Button variant="outline" size="sm" onClick={() => void handleRefresh()}>
            {t("threads.retry")}
          </Button>
        </BentoCard>
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
                    onClick={() => void handleOpenPath(group.cwd)}
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                  </Button>
                )}
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 gap-1.5 px-2.5 text-xs"
                  onClick={() => void handleExport(group.items.map((item) => item.key))}
                >
                  <FileDown className="h-3.5 w-3.5" />
                  {t("threads.exportFolder")}
                </Button>
              </div>
            </div>
            <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
              {group.items.map((thread) => (
                <ThreadCard
                  key={thread.key}
                  thread={thread}
                  selected={selected.has(thread.key)}
                  onToggleSelect={() => toggleSelect(thread.key)}
                  onExport={() => void handleExport([thread.key])}
                  onOpenRaw={() => void handleOpenPath(thread.filePath)}
                />
              ))}
            </div>
          </section>
        ))
      )}
    </div>
  );
}

