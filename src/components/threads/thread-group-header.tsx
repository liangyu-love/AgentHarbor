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
  onOpenPath,
  onCopyPath,
  copiedGroupPath,
  t,
}: ThreadGroupHeaderProps) {
  return (
    <div
      className={cn(
        "flex items-center justify-between gap-2 rounded-lg border bg-card/80 backdrop-blur-sm",
        compact ? "h-[44px] px-2.5" : "p-1.5 px-2.5",
        allSelected ? "border-primary/50" : "border-border/60",
      )}
    >
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <button
          type="button"
          onClick={onToggleCollapse}
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground"
          title={collapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
          aria-label={collapsed ? t("threads.expandGroup") : t("threads.collapseGroup")}
        >
          <ChevronRight
            className={cn("h-3.5 w-3.5 transition-transform", !collapsed && "rotate-90")}
          />
        </button>
        <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
          <Folder className="h-3 w-3" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h2 className="truncate text-xs font-semibold text-foreground">
              {group.folderName}
            </h2>
            <span className="rounded bg-muted px-1 py-px text-[10px] text-muted-foreground">
              {t("threads.threadCount", { count: group.items.length })}
            </span>
            <span className="hidden text-[10px] text-muted-foreground/80 sm:inline">
              · {group.totalMessages} {t("threads.groupMessages")} · {formatSize(group.totalSize)} ·{" "}
              {formatRelative(group.latestAt)}
            </span>
          </div>
          {group.cwd && (
            <p
              className="truncate font-mono text-[10px] text-muted-foreground/70"
              title={group.cwd}
            >
              {group.cwd}
            </p>
          )}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-0.5">
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant={allSelected ? "secondary" : "ghost"}
              size="sm"
              className="h-6 px-1.5 text-[11px]"
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
              className="h-6 w-6 text-muted-foreground"
              onClick={onCopyPath}
              title={t("threads.copyPath")}
              aria-label={t("threads.copyPath")}
            >
              {copiedGroupPath === group.key ? (
                <Check className="h-3 w-3 text-emerald-500" />
              ) : (
                <Copy className="h-3 w-3" />
              )}
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              className="h-6 w-6 text-muted-foreground"
              onClick={onOpenPath}
              title={t("threads.openFolder")}
              aria-label={t("threads.openFolder")}
            >
              <FolderOpen className="h-3 w-3" />
            </Button>
          </>
        )}
        <Button
          variant="outline"
          size="sm"
          className="h-6 gap-1 px-1.5 text-[11px]"
          onClick={onExport}
        >
          <FileDown className="h-3 w-3" />
          <span className="hidden sm:inline">{t("threads.exportFolder")}</span>
        </Button>
      </div>
    </div>
  );
}
