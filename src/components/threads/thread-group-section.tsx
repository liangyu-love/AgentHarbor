import { useTranslation } from "react-i18next";
import {
  Check,
  CheckSquare,
  ChevronRight,
  Copy,
  FileDown,
  Folder,
  FolderOpen,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatRelative } from "@/lib/format-time";
import { ThreadCard } from "@/components/threads/thread-card";
import { formatSize, type FolderGroup } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";
import { threadGridStyle } from "./thread-grid-layout";

// One workspace folder in the non-virtual list: header bar plus its thread cards.
export function ThreadGroupSection({
  group,
  list,
  onDetails,
}: {
  group: FolderGroup;
  list: ThreadList;
  onDetails: (key: string) => void;
}) {
  const { t } = useTranslation();
  const {
    collapsedGroups,
    selected,
    toggleGroupCollapse,
    toggleGroupSelect,
    toggleSelect,
    handleCopyPath,
    copiedGroupPath,
    handleOpenPath,
    handleExport,
    handleDelete,
    setRecoveryTarget,
    viewMode,
  } = list;

  const isCollapsed = collapsedGroups.has(group.key);
  const groupKeys = group.items.map((item) => item.key);
  const groupExportKeys = group.items
    .filter((item) => item.rolloutStatus !== "missing")
    .map((item) => item.key);
  const allInGroupSelected =
    groupKeys.length > 0 && groupKeys.every((key) => selected.has(key));
  const someInGroupSelected =
    groupKeys.some((key) => selected.has(key)) && !allInGroupSelected;

  return (
    <section className="space-y-1.5">
      {/* Workspace Folder Header Bar */}
      <div
        className={cn(
          "flex flex-wrap items-center justify-between gap-2.5 rounded-xl border bg-card/80 p-2 px-3 backdrop-blur-md transition-all duration-200 shadow-2xs hover:border-primary/50",
          allInGroupSelected
            ? "border-primary/70 ring-1 ring-primary/30 bg-primary/[0.03]"
            : "border-border/70",
        )}
      >
        {/* Left: Fold button + Folder icon + Folder Info */}
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <button
            type="button"
            onClick={() => toggleGroupCollapse(group.key)}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground transition-all duration-200 active:scale-95"
            title={isCollapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
            aria-label={isCollapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
          >
            <ChevronRight
              className={cn(
                "h-4 w-4 transition-transform duration-200",
                !isCollapsed && "rotate-90 text-foreground",
              )}
            />
          </button>

          <div className="flex h-7.5 w-7.5 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <Folder className="h-4 w-4" />
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-xs sm:text-sm font-bold tracking-tight text-foreground">
                {group.folderName}
              </h2>

              {/* Badges */}
              <span className="rounded-md bg-muted px-2 py-0.5 text-[10px] font-semibold text-muted-foreground">
                {t("threads.threadCount", { count: group.items.length })}
              </span>

              <span className="hidden text-[11px] text-muted-foreground/80 sm:inline-block">
                · {group.totalMessages} {t("threads.groupMessages")}
              </span>

              <span className="hidden text-[11px] text-muted-foreground/80 sm:inline-block">
                · {formatSize(group.totalSize)}
              </span>

              <span className="hidden text-[11px] text-muted-foreground/70 md:inline-block">
                · 活跃于 {formatRelative(group.latestAt)}
              </span>
            </div>

            {/* Path with 1-click copy */}
            {group.cwd && (
              <div className="mt-0.5 flex items-center gap-1.5">
                <p
                  className="truncate font-mono text-[11px] text-muted-foreground/75 cursor-pointer hover:text-foreground transition-colors"
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
                      className="text-muted-foreground/60 hover:text-foreground transition-colors"
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
                className="h-7.5 gap-1.5 rounded-lg px-2.5 text-xs font-semibold"
                onClick={() => toggleGroupSelect(groupKeys)}
              >
                <CheckSquare
                  className={`h-3.5 w-3.5 ${
                    allInGroupSelected || someInGroupSelected
                      ? "text-primary"
                      : "text-muted-foreground"
                  }`}
                />
                <span className="hidden sm:inline">
                  {allInGroupSelected
                    ? t("threads.deselectGroup")
                    : t("threads.selectGroup")}
                </span>
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              {allInGroupSelected
                ? t("threads.deselectGroup")
                : t("threads.selectGroup")}
            </TooltipContent>
          </Tooltip>

          {/* Open in File Explorer */}
          {group.cwd && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7.5 w-7.5 rounded-lg text-muted-foreground hover:bg-primary/10 hover:text-primary"
                  onClick={() => void handleOpenPath(group.cwd)}
                  aria-label={t("threads.openFolder")}
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
                className="h-7.5 gap-1.5 rounded-lg px-2.5 text-xs font-semibold"
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
              ? "grid animate-in fade-in duration-200"
              : "space-y-1.5 animate-in fade-in duration-200"
          }
          style={viewMode === "grid" ? threadGridStyle : undefined}
        >
          {group.items.map((thread) => (
            <ThreadCard
              key={thread.key}
              thread={thread}
              selected={selected.has(thread.key)}
              detailActive={list.detailTarget?.key === thread.key}
              onToggleSelect={() => toggleSelect(thread.key)}
              onExport={() => void handleExport([thread.key])}
              onOpenRaw={() => void handleOpenPath(thread.filePath)}
              misplaced={list.isMisplaced(thread)}
              onRecover={list.canRecover(thread) ? () => setRecoveryTarget(thread) : undefined}
              onDetails={() => onDetails(thread.key)}
              onDelete={() => void handleDelete([thread.key])}
              variant={viewMode}
            />
          ))}
        </div>
      )}
    </section>
  );
}
