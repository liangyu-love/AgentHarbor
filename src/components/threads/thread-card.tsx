import { useState } from "react";
import {
  Archive,
  Check,
  CheckSquare,
  Clock,
  Copy,
  Download,
  FolderOpen,
  HardDrive,
  Sparkles,
  Terminal,
  User,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { ThreadSummary } from "@/types/threads";
import { formatSize } from "./thread-utils";

interface ThreadCardProps {
  thread: ThreadSummary;
  selected: boolean;
  onToggleSelect: () => void;
  onExport: () => void;
  onOpenRaw: () => void;
  variant?: "grid" | "compact";
}

export function ThreadCard({
  thread,
  selected,
  onToggleSelect,
  onExport,
  onOpenRaw,
  variant = "grid",
}: ThreadCardProps) {
  const { t } = useTranslation();
  const [copiedId, setCopiedId] = useState(false);

  const totalMsgs = (thread.userMessages || 0) + (thread.agentMessages || 0);
  const userRatio =
    totalMsgs > 0 ? Math.round(((thread.userMessages || 0) / totalMsgs) * 100) : 50;

  const handleCopyId = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!thread.id) return;
    navigator.clipboard.writeText(thread.id);
    setCopiedId(true);
    toast({
      title: t("threads.copiedId"),
      description: thread.id,
      variant: "default",
    });
    setTimeout(() => setCopiedId(false), 2000);
  };

  // Compact Row View — fixed height so virtualization can measure rows.
  if (variant === "compact") {
    return (
      <TooltipProvider delayDuration={200}>
        <div
          className={cn(
            "group relative flex h-10 cursor-pointer items-center justify-between gap-2 overflow-hidden rounded-lg border border-border/60 bg-card/80 px-2 transition-all duration-150 hover:border-primary/45 hover:bg-muted/40",
            selected
              ? "border-primary/60 bg-primary/[0.04] shadow-xs ring-1 ring-primary/35 dark:border-amber-400/50 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
              : "",
          )}
          onClick={onToggleSelect}
        >
          {/* Left: Checkbox + Title + Status */}
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <span
              className={cn(
                "flex h-4 w-4 shrink-0 items-center justify-center rounded-[5px] border transition-all duration-150",
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-2.5 w-2.5" />
            </span>

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <p className="truncate text-xs font-medium leading-none text-foreground group-hover:text-primary transition-colors">
                  {thread.title || t("threads.untitled")}
                </p>
                {thread.archived ? (
                  <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-amber-500/10 px-1.5 py-px text-[9px] font-medium text-amber-600 dark:text-amber-400">
                    <Archive className="h-2 w-2" />
                    {t("threads.archivedBadge")}
                  </span>
                ) : (
                  <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-emerald-500/10 px-1.5 py-px text-[9px] font-medium text-emerald-600 dark:text-emerald-400">
                    <span className="h-1 w-1 rounded-full bg-emerald-500 animate-pulse" />
                    {t("threads.activeBadge")}
                  </span>
                )}
                {(thread.cliVersion || thread.source) && (
                  <span className="hidden md:inline-flex items-center gap-0.5 rounded-[4px] bg-muted/60 px-1 py-px font-mono text-[9px] text-muted-foreground">
                    <Terminal className="h-2 w-2 opacity-70" />
                    {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
                  </span>
                )}
              </div>
            </div>
          </div>

          {/* Center: Turn counts & Time */}
          <div className="hidden shrink-0 items-center gap-2.5 text-[10px] text-muted-foreground sm:flex">
            <div
              className="flex items-center gap-1 font-mono"
              title={t("threads.messagesTooltip")}
            >
              <span className="flex items-center gap-0.5 text-sky-600 dark:text-sky-400">
                <User className="h-2.5 w-2.5" />
                {thread.userMessages}
              </span>
              <span className="text-muted-foreground/40">/</span>
              <span className="flex items-center gap-0.5 text-purple-600 dark:text-purple-400">
                <Sparkles className="h-2.5 w-2.5" />
                {thread.agentMessages}
              </span>
            </div>

            <span className="w-12 text-right font-mono">
              {formatSize(thread.fileSize)}
            </span>

            <span
              className="w-14 text-right"
              title={formatDateTime(thread.updatedAt)}
            >
              {formatRelative(thread.updatedAt)}
            </span>
          </div>

          {/* Right: Actions */}
          <div className="flex shrink-0 items-center gap-0.5 opacity-70 group-hover:opacity-100">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={handleCopyId}
                >
                  {copiedId ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.copyId")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={(e) => {
                    e.stopPropagation();
                    onExport();
                  }}
                >
                  <Download className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.exportThread")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenRaw();
                  }}
                >
                  <FolderOpen className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.openRawFile")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
      </TooltipProvider>
    );
  }

  // Bento Card Grid View (Default) — dense card
  return (
    <TooltipProvider delayDuration={200}>
      <BentoCard
        className={cn(
          "group relative flex flex-col justify-between cursor-pointer p-2.5 transition-all duration-150 ease-out hover:-translate-y-px hover:border-primary/40 hover:shadow-sm",
          selected
            ? "border-primary/60 bg-primary/[0.03] ring-1 ring-primary/35 shadow-xs dark:border-amber-400/60 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
            : "",
        )}
        onClick={onToggleSelect}
      >
        {/* Top: Checkbox, Title & Status */}
        <div className="space-y-1.5">
          <div className="flex items-start gap-1.5">
            <span
              className={cn(
                "mt-px flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-[4px] border transition-all duration-150",
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-2.5 w-2.5" />
            </span>

            <div className="min-w-0 flex-1">
              <h3 className="line-clamp-1 text-xs font-semibold leading-snug tracking-tight text-foreground group-hover:text-primary transition-colors">
                {thread.title || t("threads.untitled")}
              </h3>
            </div>
          </div>

          {/* Tags bar */}
          <div className="flex flex-wrap items-center gap-1 pl-5">
            {thread.archived ? (
              <span className="inline-flex items-center gap-0.5 rounded-full bg-amber-500/10 px-1.5 py-px text-[9px] font-medium text-amber-600 dark:text-amber-400">
                <Archive className="h-2 w-2" />
                {t("threads.archivedBadge")}
              </span>
            ) : (
              <span className="inline-flex items-center gap-0.5 rounded-full bg-emerald-500/10 px-1.5 py-px text-[9px] font-medium text-emerald-600 dark:text-emerald-400">
                <span className="h-1 w-1 rounded-full bg-emerald-500 animate-pulse" />
                {t("threads.activeBadge")}
              </span>
            )}

            {(thread.cliVersion || thread.source) && (
              <span className="inline-flex items-center gap-0.5 rounded-[4px] bg-muted/60 px-1 py-px font-mono text-[9px] text-muted-foreground">
                <Terminal className="h-2 w-2 opacity-70" />
                {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
              </span>
            )}
          </div>
        </div>

        {/* Middle: Dialogue breakdown — single compact line */}
        <div className="mt-1.5 flex items-center justify-between gap-2 text-[10px]">
          <div className="flex min-w-0 items-center gap-1.5 font-mono">
            <span className="flex items-center gap-0.5 text-sky-600 dark:text-sky-400">
              <User className="h-2.5 w-2.5" />
              {thread.userMessages}
            </span>
            <span className="text-muted-foreground/40">/</span>
            <span className="flex items-center gap-0.5 text-violet-600 dark:text-violet-400">
              <Sparkles className="h-2.5 w-2.5" />
              {thread.agentMessages}
            </span>
          </div>
              <div className="h-1 w-16 shrink-0 overflow-hidden rounded-full bg-muted/70 flex" title={`用户提问占比: ${userRatio}%`}>
            <div
              className="bg-gradient-to-r from-sky-500 to-sky-400"
              style={{ width: `${userRatio}%` }}
            />
            <div
              className="bg-gradient-to-r from-fuchsia-500 to-purple-400"
              style={{ width: `${100 - userRatio}%` }}
            />
          </div>
        </div>

        {/* Bottom: Meta & Quick Actions */}
        <div className="mt-1.5 flex items-center justify-between border-t border-border/30 pt-1.5 text-[10px] text-muted-foreground">
          <div className="flex min-w-0 items-center gap-2">
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex items-center gap-0.5 hover:text-foreground transition-colors cursor-help">
                  <Clock className="h-2.5 w-2.5 opacity-70" />
                  {formatRelative(thread.updatedAt)}
                </span>
              </TooltipTrigger>
              <TooltipContent className="text-xs space-y-0.5">
                <div>活跃时间: {formatDateTime(thread.updatedAt)}</div>
                {thread.createdAt && (
                  <div className="text-muted-foreground">
                    创建时间: {formatDateTime(thread.createdAt)}
                  </div>
                )}
              </TooltipContent>
            </Tooltip>

            <span className="flex items-center gap-0.5 font-mono">
              <HardDrive className="h-2.5 w-2.5 opacity-70" />
              {formatSize(thread.fileSize)}
            </span>
          </div>

          {/* Action dock */}
          <div className="flex items-center gap-0.5">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-5.5 w-5.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={handleCopyId}
                >
                  {copiedId ? (
                    <Check className="h-2.5 w-2.5 text-emerald-500" />
                  ) : (
                    <Copy className="h-2.5 w-2.5" />
                  )}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.copyId")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-5.5 w-5.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={(e) => {
                    e.stopPropagation();
                    onExport();
                  }}
                >
                  <Download className="h-2.5 w-2.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.exportThread")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-5.5 w-5.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenRaw();
                  }}
                >
                  <FolderOpen className="h-2.5 w-2.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.openRawFile")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
      </BentoCard>
    </TooltipProvider>
  );
}
