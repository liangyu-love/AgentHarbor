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

  // Compact Row View
  if (variant === "compact") {
    return (
      <TooltipProvider delayDuration={200}>
        <div
          className={cn(
            "group relative flex cursor-pointer items-center justify-between gap-3 rounded-xl border border-border/70 bg-card/80 p-3 transition-all duration-200 hover:-translate-y-px hover:border-primary/50 hover:shadow-sm",
            selected
              ? "border-primary/60 bg-primary/[0.04] shadow-xs ring-1 ring-primary/40 dark:border-amber-400/50 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
              : "hover:bg-muted/40",
          )}
          onClick={onToggleSelect}
        >
          {/* Left: Checkbox + Title + Status */}
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <span
              className={cn(
                "flex h-5 w-5 shrink-0 items-center justify-center rounded-[6px] border transition-all duration-200",
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs scale-105"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-3.5 w-3.5" />
            </span>

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <p className="truncate text-sm font-medium leading-none text-foreground group-hover:text-primary transition-colors">
                  {thread.title || t("threads.untitled")}
                </p>
                {thread.archived ? (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                    <Archive className="h-2.5 w-2.5" />
                    {t("threads.archivedBadge")}
                  </span>
                ) : (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                    {t("threads.activeBadge")}
                  </span>
                )}
                {(thread.cliVersion || thread.source) && (
                  <span className="hidden sm:inline-flex items-center gap-1 rounded-[5px] bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                    <Terminal className="h-2.5 w-2.5 opacity-70" />
                    {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
                  </span>
                )}
              </div>
            </div>
          </div>

          {/* Center: Turn counts & Time */}
          <div className="hidden shrink-0 items-center gap-4 text-xs text-muted-foreground sm:flex">
            <div
              className="flex items-center gap-2 font-mono text-[11px]"
              title={t("threads.messagesTooltip")}
            >
              <span className="flex items-center gap-1 text-sky-600 dark:text-sky-400">
                <User className="h-3 w-3" />
                {thread.userMessages}
              </span>
              <span className="text-muted-foreground/40">/</span>
              <span className="flex items-center gap-1 text-violet-600 dark:text-violet-400">
                <Sparkles className="h-3 w-3" />
                {thread.agentMessages}
              </span>
            </div>

            <span className="w-16 text-right font-mono text-[11px]">
              {formatSize(thread.fileSize)}
            </span>

            <span
              className="w-20 text-right text-[11px]"
              title={formatDateTime(thread.updatedAt)}
            >
              {formatRelative(thread.updatedAt)}
            </span>
          </div>

          {/* Right: Actions */}
          <div className="flex shrink-0 items-center gap-1 opacity-80 group-hover:opacity-100">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7 w-7 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={handleCopyId}
                >
                  {copiedId ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.copyId")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7 w-7 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={(e) => {
                    e.stopPropagation();
                    onExport();
                  }}
                >
                  <Download className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.exportThread")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7 w-7 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={(e) => {
                    e.stopPropagation();
                    onOpenRaw();
                  }}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.openRawFile")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
      </TooltipProvider>
    );
  }

  // Bento Card Grid View (Default)
  return (
    <TooltipProvider delayDuration={200}>
      <BentoCard
        className={cn(
          "group relative flex flex-col justify-between cursor-pointer p-4 transition-all duration-300 ease-out hover:-translate-y-1 hover:shadow-lg",
          selected
            ? "border-primary/60 bg-primary/[0.03] ring-2 ring-primary/40 shadow-sm dark:border-amber-400/60 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
            : "hover:border-primary/40",
        )}
        onClick={onToggleSelect}
      >
        {/* Top: Checkbox, Title & Status */}
        <div className="space-y-2.5">
          <div className="flex items-start gap-2.5">
            <span
              className={cn(
                "mt-0.5 flex h-4.5 w-4.5 shrink-0 items-center justify-center rounded-[5px] border transition-all duration-200",
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs scale-105"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-3 w-3" />
            </span>

            <div className="min-w-0 flex-1">
              <h3 className="line-clamp-2 text-[13px] font-semibold leading-snug tracking-tight text-foreground group-hover:text-primary transition-colors">
                {thread.title || t("threads.untitled")}
              </h3>
            </div>
          </div>

          {/* Tags bar */}
          <div className="flex flex-wrap items-center gap-1.5 pl-7">
            {thread.archived ? (
              <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                <Archive className="h-2.5 w-2.5" />
                {t("threads.archivedBadge")}
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                {t("threads.activeBadge")}
              </span>
            )}

            {(thread.cliVersion || thread.source) && (
              <span className="inline-flex items-center gap-1 rounded-[5px] bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                <Terminal className="h-2.5 w-2.5 opacity-70" />
                {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
              </span>
            )}
          </div>
        </div>

        {/* Middle: Dialogue breakdown & visual ratio bar */}
        <div className="mt-3.5 space-y-1.5 rounded-xl bg-muted/30 p-2.5 border border-border/40 transition-colors group-hover:border-border/60">
          <div className="flex items-center justify-between text-[11px] font-medium">
            <span className="flex items-center gap-1.5 text-sky-600 dark:text-sky-400 font-mono">
              <User className="h-3 w-3" />
              {t("threads.userMsgs")}: {thread.userMessages}
            </span>
            <span className="flex items-center gap-1.5 text-violet-600 dark:text-violet-400 font-mono">
              <Sparkles className="h-3 w-3" />
              {t("threads.agentMsgs")}: {thread.agentMessages}
            </span>
          </div>

          {/* Micro Progress Indicator with smooth gradients */}
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted/70 flex">
            <div
              className="bg-gradient-to-r from-sky-500 to-sky-400 transition-all duration-300"
              style={{ width: `${userRatio}%` }}
              title={`用户提问占比: ${userRatio}%`}
            />
            <div
              className="bg-gradient-to-r from-violet-500 to-violet-400 transition-all duration-300"
              style={{ width: `${100 - userRatio}%` }}
              title={`助手回复占比: ${100 - userRatio}%`}
            />
          </div>
        </div>

        {/* Bottom: Meta & Quick Actions */}
        <div className="mt-3.5 flex items-center justify-between border-t border-border/40 pt-2.5 text-[11px] text-muted-foreground">
          <div className="flex items-center gap-3">
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex items-center gap-1 hover:text-foreground transition-colors cursor-help">
                  <Clock className="h-3 w-3 opacity-70" />
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

            <span className="flex items-center gap-1 font-mono">
              <HardDrive className="h-3 w-3 opacity-70" />
              {formatSize(thread.fileSize)}
            </span>
          </div>

          {/* Action dock */}
          <div className="flex items-center gap-1">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6.5 w-6.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  onClick={handleCopyId}
                >
                  {copiedId ? (
                    <Check className="h-3 w-3 text-emerald-500" />
                  ) : (
                    <Copy className="h-3 w-3" />
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
                  className="h-6.5 w-6.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
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
                  className="h-6.5 w-6.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
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
      </BentoCard>
    </TooltipProvider>
  );
}
