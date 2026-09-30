import { useTranslation } from "react-i18next";
import { MessagesSquare, X } from "lucide-react";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ThreadRecoveryDialog } from "@/components/threads/thread-recovery-dialog";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { VirtualThreadList } from "@/components/threads/virtual-thread-list";
import { ThreadGroupSection } from "@/components/threads/thread-group-section";
import { ThreadMetrics } from "@/components/threads/thread-metrics";
import { ThreadNotices } from "@/components/threads/thread-notices";
import { ThreadToolbar } from "@/components/threads/thread-toolbar";
import { ThreadsHeader } from "@/components/threads/threads-header";
import { ThreadProjectTree } from "@/components/threads/thread-project-tree";
import { ThreadDetailSheet } from "@/components/threads/thread-detail-sheet";
import { SelectionBar } from "@/components/threads/selection-bar";
import { useThreadList } from "@/components/threads/use-thread-list";
import { formatInvokeError } from "@/lib/invoke-error";

export function ThreadsPage() {
  const { t } = useTranslation();
  const list = useThreadList();
  const {
    threadsQuery, groups, query, setQuery, useVirtualList, collapsedGroups, selected, viewMode, toggleSelect,
    toggleGroupSelect, toggleGroupCollapse, handleExport, handleOpenPath, setRecoveryTarget, handleDelete,
    handleCopyPath, copiedGroupPath, handleRefresh, recoveryTarget,
  } = list;
  const hasFilters = !!query.trim() || list.status !== "all" || !!list.activeProject;

  return (
    <TooltipProvider delayDuration={200}>
      <div className="space-y-3">
        {/* Page Header */}
        <ThreadsHeader list={list} />

        <ThreadNotices list={list} />

        {/* 4-Metric Mission Control HUD */}
        <ThreadMetrics list={list} />

        {/* Power Toolbar */}
        <ThreadToolbar list={list} />

        {/* Dynamic Sticky Batch Selection Bar */}
        <SelectionBar list={list} />

        <div className={list.projects.length ? "grid min-w-0 items-start gap-3 xl:grid-cols-[220px_minmax(0,1fr)]" : "min-w-0"}>
          {list.projects.length > 0 && <ThreadProjectTree list={list} />}
          <div className="min-w-0 space-y-3" data-thread-results>
            {list.activeProject && <div className="flex min-w-0 items-center gap-2 text-xs">
              <span className="min-w-0 flex-1 truncate text-muted-foreground" title={list.activeProject.cwd}>
                {list.activeProject.folderName}{list.focusedThread ? ` / ${list.focusedThread.title || t("threads.untitled")}` : ""}
              </span>
              <Button variant="ghost" size="sm" className="h-7 shrink-0 gap-1 text-xs"
                onClick={() => list.selectProject(list.focusedThread ? list.activeProject?.key ?? null : null)}>
                <X className="h-3 w-3" />{t(list.focusedThread ? "threads.backToProject" : "threads.allThreads")}
              </Button>
            </div>}
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
                  {formatInvokeError(threadsQuery.error)}
                </p>
                <Button variant="outline" size="sm" onClick={() => void handleRefresh()}>
                  {t("threads.retry")}
                </Button>
              </BentoCard>
            ) : groups.length === 0 ? (
              <BentoCard className="flex flex-col items-center justify-center gap-3 py-16 text-center">
                <MessagesSquare className="h-10 w-10 text-muted-foreground/40" />
                <p className="text-sm font-semibold">
                  {hasFilters ? t("threads.noFilteredResults") : t("threads.emptyTitle")}
                </p>
                <p className="max-w-md text-xs text-muted-foreground">
                  {hasFilters ? t("threads.noFilteredResultsDesc") : t("threads.emptyDesc")}
                </p>
                {hasFilters && (
                  <Button
                    variant="outline"
                    size="sm"
                    className="mt-1 h-8 gap-1.5 text-xs"
                    onClick={() => { setQuery(""); list.setStatus("all"); list.selectProject(null); }}
                  >
                    <X className="h-3.5 w-3.5" />
                    {t("threads.clearFilters")}
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
                onDetails={thread => list.setDetailKey(thread.key)}
                onDelete={(thread) => void handleDelete([thread.key])}
                onCopyPath={handleCopyPath}
                copiedGroupPath={copiedGroupPath}
                t={t}
              />
            ) : (
              groups.map((group) => <ThreadGroupSection key={group.key} group={group} list={list} />)
            )}
          </div>
        </div>
        <ThreadRecoveryDialog
          thread={recoveryTarget}
          onOpenChange={(open) => !open && setRecoveryTarget(null)}
          onRecovered={async () => {
            await threadsQuery.refetch();
          }}
        />
        {list.detailTarget && <ThreadDetailSheet key={list.detailTarget.key} thread={list.detailTarget}
          onClose={() => list.setDetailKey(null)} onExport={() => void handleExport([list.detailTarget!.key])}
          onDelete={() => void handleDelete([list.detailTarget!.key])} onOpenRaw={() => void handleOpenPath(list.detailTarget!.filePath)} />}
        <ConfirmDialog request={list.confirmRequest} onClose={() => list.setConfirmRequest(null)} />
      </div>
    </TooltipProvider>
  );
}
