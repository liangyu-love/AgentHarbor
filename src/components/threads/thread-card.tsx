import { Archive, CheckSquare, FileText, FolderOpen, MessageSquare } from "lucide-react";
import { useTranslation } from "react-i18next";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import type { ThreadSummary } from "@/types/threads";

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

interface ThreadCardProps {
  thread: ThreadSummary;
  selected: boolean;
  onToggleSelect: () => void;
  onExport: () => void;
  onOpenRaw: () => void;
}

export function ThreadCard({
  thread,
  selected,
  onToggleSelect,
  onExport,
  onOpenRaw,
}: ThreadCardProps) {
  const { t } = useTranslation();
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
            <p className="truncate text-[13px] font-medium leading-snug">
              {thread.title || t("threads.untitled")}
            </p>
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
              onOpenRaw();
            }}
          >
            <FolderOpen className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </BentoCard>
  );
}
