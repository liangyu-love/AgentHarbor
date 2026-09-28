import { useTranslation } from "react-i18next";
import { MessagesSquare, X } from "lucide-react";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ThreadRecoveryDialog } from "@/components/threads/thread-recovery-dialog";
import { VirtualThreadList } from "@/components/threads/virtual-thread-list";
import { ThreadGroupSection } from "@/components/threads/thread-group-section";
import { ThreadMetrics } from "@/components/threads/thread-metrics";
import { ThreadNotices } from "@/components/threads/thread-notices";
import { ThreadToolbar } from "@/components/threads/thread-toolbar";
import { ThreadsHeader } from "@/components/threads/threads-header";
import { SelectionBar } from "@/components/threads/selection-bar";
import { useThreadList } from "@/components/threads/use-thread-list";

export function ThreadsPage() {
  const { t } = useTranslation();
  const list = useThreadList();
  const {
    threadsQuery, groups, query, setQuery, useVirtualList, collapsedGroups, selected, viewMode, toggleSelect,
    toggleGroupSelect, toggleGroupCollapse, handleExport, handleOpenPath, setRecoveryTarget, handleDelete,
    handleCopyPath, copiedGroupPath, handleRefresh, recoveryTarget,
  } = list;

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
          groups.map((group) => <ThreadGroupSection key={group.key} group={group} list={list} />)
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
