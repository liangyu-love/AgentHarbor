import {
  Check,
  ChevronRight,
  Copy,
  FileDown,
  Folder,
  FolderOpen,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { FolderGroup } from "@/components/threads/thread-utils";
import { formatSize } from "@/components/threads/thread-utils";
import { formatRelative } from "@/lib/format-time";
import { cn } from "@/lib/utils";

export const GROUP_HEADER_HEIGHT = 44;

export interface ThreadGroupHeaderProps {
  group: FolderGroup;
  collapsed: boolean;
  allSelected: boolean;
  compact?: boolean;
  onToggleCollapse: () => void;
  onToggleGroup: () => void;
  onExport: () => void;
  exportDisabled?: boolean;
  onOpenPath: () => void;
  onCopyPath: () => void;
  copiedGroupPath: string | null;
  t: (key: string, opts?: Record<string, unknown>) => string;
}

export function ThreadGroupHeader({
  group,
  collapsed,
  allSelected,
  compact = false,
  onToggleCollapse,
  onToggleGroup,
  onExport,
  exportDisabled = false,
  onOpenPath,
  onCopyPath,
  copiedGroupPath,
  t,
}: ThreadGroupHeaderProps) {
  return (
    <div
      className={cn(
        "flex items-center justify-between gap-2.5 rounded-xl border bg-card/85 backdrop-blur-md transition-all duration-200 shadow-2xs hover:border-primary/50",
        compact ? "h-[44px] px-3" : "p-2 px-3",
        allSelected ? "border-primary/70 ring-1 ring-primary/30 bg-primary/[0.03]" : "border-border/70",
      )}
    >
      <div className="flex min-w-0 flex-1 items-center gap-2.5">
        <button
          type="button"
          onClick={onToggleCollapse}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground transition-all duration-200 active:scale-95"
          title={collapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
          aria-label={collapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
        >
          <ChevronRight
            className={cn(
              "h-4 w-4 transition-transform duration-200",
              !collapsed && "rotate-90 text-foreground",
            )}
          />
        </button>
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Folder className="h-3.5 w-3.5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="truncate text-xs font-bold tracking-tight text-foreground">
              {group.folderName}
            </h2>
            <span className="rounded-md bg-muted px-1.5 py-0.5 text-[10px] font-semibold text-muted-foreground">
              {t("threads.threadCount", { count: group.items.length })}
            </span>
            <span className="hidden text-[10px] text-muted-foreground/80 sm:inline">
              · {group.totalMessages} {t("threads.groupMessages")} · {formatSize(group.totalSize)} ·{" "}
              {formatRelative(group.latestAt)}
            </span>
          </div>
          {group.cwd && (
            <p
              className="truncate font-mono text-[10px] text-muted-foreground/75"
              title={group.cwd}
            >
              {group.cwd}
            </p>
          )}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={allSelected ? "secondary" : "ghost"}
              size="sm"
              className="h-7 rounded-lg px-2 text-[11px] font-semibold"
              onClick={onToggleGroup}
            >
              {allSelected ? t("threads.deselectGroup") : t("threads.selectGroup")}
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            {allSelected ? t("threads.deselectGroup") : t("threads.selectGroup")}
          </TooltipContent>
        </Tooltip>
        {group.cwd && (
          <>
            <Button
              variant="ghost"
              size="icon-sm"
              className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
              onClick={onCopyPath}
              title={t("threads.copyPath")}
              aria-label={t("threads.copyPath")}
            >
              {copiedGroupPath === group.key ? (
                <Check className="h-3.5 w-3.5 text-emerald-500" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
              onClick={onOpenPath}
              title={t("threads.openFolder")}
              aria-label={t("threads.openFolder")}
            >
              <FolderOpen className="h-3.5 w-3.5" />
            </Button>
          </>
        )}
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="outline"
              size="sm"
              className="h-7 gap-1.5 rounded-lg px-2 text-[11px] font-semibold"
              onClick={onExport}
              disabled={exportDisabled}
              title={exportDisabled ? t("threads.exportUnavailable") : t("threads.exportFolder")}
            >
              <FileDown className="h-3 w-3" />
              <span className="hidden sm:inline">{t("threads.exportFolder")}</span>
            </Button>
          </TooltipTrigger>
          <TooltipContent>
            {exportDisabled ? t("threads.exportUnavailable") : t("threads.exportFolder")}
          </TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}
