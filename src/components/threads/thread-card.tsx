import { useState } from "react";
import {
  AlertTriangle,
  Archive,
  Check,
  CheckSquare,
  Clock,
  Copy,
  Coins,
  Database,
  Download,
  FolderOpen,
  HardDrive,
  RefreshCcw,
  Sparkles,
  Terminal,
  Trash2,
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
import { formatSize, formatTokenCount } from "./thread-utils";
import { focusRing, onActivateKey } from "@/lib/a11y";

interface ThreadCardProps {
  thread: ThreadSummary;
  selected: boolean;
  onToggleSelect: () => void;
  onExport: () => void;
  onOpenRaw: () => void;
  onRecover: () => void;
  onDelete: () => void;
  variant?: "grid" | "compact";
}

export function ThreadCard({
  thread,
  selected,
  onToggleSelect,
  onExport,
  onOpenRaw,
  onRecover,
  onDelete,
  variant = "grid",
}: ThreadCardProps) {
  const { t } = useTranslation();
  const [copiedId, setCopiedId] = useState(false);

  const totalMsgs = (thread.userMessages || 0) + (thread.agentMessages || 0);
  const userRatio =
    totalMsgs > 0 ? Math.round(((thread.userMessages || 0) / totalMsgs) * 100) : 50;
  const hasRollout = thread.rolloutStatus !== "missing";
  const canPreviewRecovery = thread.id.length > 0;
  const providerLabel = thread.modelProvider || t("threads.unknownProvider");
  const modelLabel = thread.model || t("threads.unknownModel");
  const sourceLabel =
    thread.threadSource === "sqlite"
      ? t("threads.threadSourceSqlite")
      : t("threads.threadSourceRollout");

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
            "group relative flex h-12 cursor-pointer items-center justify-between gap-2.5 overflow-hidden rounded-lg border border-border/60 bg-card/80 px-2.5 transition-all duration-150 hover:border-primary/45 hover:bg-muted/40",
            selected
              ? "border-primary/60 bg-primary/[0.04] shadow-xs ring-1 ring-primary/35 dark:border-amber-400/50 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
              : !hasRollout
                ? "border-destructive/45 bg-destructive/[0.025]"
                : "",
          )}
          onClick={onToggleSelect}
        >
          {/* Left: Checkbox + Title + Status */}
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <span
              role="checkbox"
              aria-checked={selected}
              aria-label={thread.title || t("threads.untitled")}
              tabIndex={0}
              onKeyDown={onActivateKey(onToggleSelect)}
              className={cn(
                "flex h-4 w-4 shrink-0 items-center justify-center rounded-[5px] border transition-all duration-150",
                focusRing,
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-2.5 w-2.5" />
            </span>

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <p className="truncate text-sm font-medium leading-none text-foreground group-hover:text-primary transition-colors">
                  {thread.title || t("threads.untitled")}
                </p>
                {thread.archived ? (
                  <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                    <Archive className="h-2.5 w-2.5" />
                    {t("threads.archivedBadge")}
                  </span>
                ) : (
                  <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                    {t("threads.activeBadge")}
                  </span>
                )}
                {!hasRollout && (
                  <span className="inline-flex shrink-0 items-center gap-0.5 rounded-full bg-destructive/10 px-1.5 py-0.5 text-[10px] font-medium text-destructive">
                    <AlertTriangle className="h-2.5 w-2.5" />
                    {t("threads.rolloutMissing")}
                  </span>
                )}
                <span
                  className="hidden max-w-40 shrink-0 truncate rounded-[4px] bg-primary/8 px-1.5 py-0.5 font-mono text-[10px] text-primary md:inline-flex"
                  title={t("threads.providerAndModel", { provider: providerLabel, model: modelLabel })}
                >
                  {providerLabel} · {modelLabel}
                </span>
                <span
                  className="hidden shrink-0 items-center gap-0.5 rounded-[4px] bg-muted/60 px-1 py-px text-[9px] text-muted-foreground lg:inline-flex"
                  title={t("threads.threadSource")}
                >
                  <Database className="h-2 w-2 opacity-70" />
                  {sourceLabel}
                </span>
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
          <div className="hidden shrink-0 items-center gap-3 text-[11px] text-muted-foreground sm:flex">
            <div
              className="flex items-center gap-1 font-mono rounded bg-muted/40 px-1.5 py-0.5"
              title={t("threads.messagesTooltip")}
            >
              <span className="flex items-center gap-0.5 text-sky-600 dark:text-sky-400 font-medium">
                <User className="h-2.5 w-2.5" />
                {thread.userMessages}
              </span>
              <span className="text-muted-foreground/30">/</span>
              <span className="flex items-center gap-0.5 text-purple-600 dark:text-purple-400 font-medium">
                <Sparkles className="h-2.5 w-2.5" />
                {thread.agentMessages}
              </span>
            </div>
            <span
              className="flex items-center gap-0.5 font-mono text-[10.5px]"
              title={t("threads.tokensUsed", { count: thread.tokensUsed || 0 })}
            >
              <Coins className="h-2.5 w-2.5 opacity-70" />
              {formatTokenCount(thread.tokensUsed)}
            </span>

            <span className="w-12 text-right font-mono text-[10.5px]">
              {formatSize(thread.fileSize)}
            </span>

            <span
              className="w-14 text-right text-[10.5px]"
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
                  aria-label={t("threads.copyId")}
                >
                  {copiedId ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.copyId")}</TooltipContent>
            </Tooltip>

            {canPreviewRecovery && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-6 w-6 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                    onClick={(e) => {
                      e.stopPropagation();
                      onRecover();
                    }}
                    aria-label={t("threads.recoveryAction")}
                  >
                    <RefreshCcw className="h-3 w-3" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("threads.recoveryAction")}</TooltipContent>
              </Tooltip>
            )}

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  disabled={!hasRollout}
                  aria-label={hasRollout ? t("threads.exportThread") : t("threads.exportUnavailable")}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!hasRollout) return;
                    onExport();
                  }}
                >
                  <Download className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {hasRollout ? t("threads.exportThread") : t("threads.exportUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  disabled={!hasRollout}
                  aria-label={hasRollout ? t("threads.openRawFile") : t("threads.rawUnavailable")}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!hasRollout) return;
                    onOpenRaw();
                  }}
                >
                  <FolderOpen className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {hasRollout ? t("threads.openRawFile") : t("threads.rawUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:bg-destructive/10 hover:text-destructive active:scale-95 transition-transform"
                  aria-label={t("threads.deleteThread")}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete();
                  }}
                >
                  <Trash2 className="h-3 w-3" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.deleteThread")}</TooltipContent>
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
          "group relative flex min-h-[168px] flex-col justify-between cursor-pointer p-3.5 transition-all duration-150 ease-out hover:-translate-y-px hover:border-primary/40 hover:shadow-sm",
          selected
            ? "border-primary/60 bg-primary/[0.03] ring-1 ring-primary/35 shadow-xs dark:border-amber-400/60 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
            : !hasRollout
              ? "border-destructive/45 bg-destructive/[0.025]"
              : "",
        )}
        onClick={onToggleSelect}
      >
        {/* Top: Checkbox, Title & Status */}
        <div className="space-y-1.5">
          <div className="flex items-start gap-1.5">
            <span
              role="checkbox"
              aria-checked={selected}
              aria-label={thread.title || t("threads.untitled")}
              tabIndex={0}
              onKeyDown={onActivateKey(onToggleSelect)}
              className={cn(
                "mt-px flex h-3.5 w-3.5 shrink-0 items-center justify-center rounded-[4px] border transition-all duration-150",
                focusRing,
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-2.5 w-2.5" />
            </span>

            <div className="min-w-0 flex-1">
              <h3 className="line-clamp-2 text-sm font-semibold leading-snug tracking-tight text-foreground group-hover:text-primary transition-colors">
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
            {!hasRollout && (
              <span className="inline-flex items-center gap-0.5 rounded-full bg-destructive/10 px-1.5 py-px text-[9px] font-medium text-destructive">
                <AlertTriangle className="h-2 w-2" />
                {t("threads.rolloutMissing")}
              </span>
            )}
            <span
              className="inline-flex max-w-full items-center rounded-[4px] bg-primary/8 px-1 py-px font-mono text-[9px] text-primary"
              title={t("threads.providerAndModel", { provider: providerLabel, model: modelLabel })}
            >
              <span className="max-w-36 truncate">{providerLabel} · {modelLabel}</span>
            </span>
            <span
              className="inline-flex items-center gap-0.5 rounded-[4px] bg-muted/60 px-1 py-px text-[9px] text-muted-foreground"
              title={t("threads.threadSource")}
            >
              <Database className="h-2 w-2 opacity-70" />
              {sourceLabel}
            </span>

            {(thread.cliVersion || thread.source) && (
              <span className="inline-flex items-center gap-0.5 rounded-[4px] bg-muted/60 px-1 py-px font-mono text-[9px] text-muted-foreground">
                <Terminal className="h-2 w-2 opacity-70" />
                {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
              </span>
            )}
          </div>
        </div>

        {/* Middle: Dialogue breakdown — single compact line */}
        <div className="mt-2.5 flex items-center justify-between gap-2 rounded-lg bg-muted/30 px-2 py-1 text-[11px]">
          <div className="flex min-w-0 items-center gap-1.5 font-mono">
            <span className="flex items-center gap-1 text-sky-600 dark:text-sky-400 font-medium">
              <User className="h-3 w-3" />
              {thread.userMessages}
            </span>
            <span className="text-muted-foreground/30">/</span>
            <span className="flex items-center gap-1 text-purple-600 dark:text-purple-400 font-medium">
              <Sparkles className="h-3 w-3" />
              {thread.agentMessages}
            </span>
            <span
              className="flex items-center gap-1 text-muted-foreground/80 ml-1 border-l border-border/40 pl-2"
              title={t("threads.tokensUsed", { count: thread.tokensUsed || 0 })}
            >
              <Coins className="h-2.5 w-2.5 opacity-70" />
              {formatTokenCount(thread.tokensUsed)}
            </span>
          </div>
          <div className="h-1.5 w-16 shrink-0 overflow-hidden rounded-full bg-muted/80 flex ring-1 ring-border/20" title={`用户提问占比: ${userRatio}%`}>
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
        <div className="mt-2 flex items-center justify-between border-t border-border/30 pt-2 text-[11px] text-muted-foreground">
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
                  aria-label={t("threads.copyId")}
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

            {canPreviewRecovery && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-5.5 w-5.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                    onClick={(e) => {
                      e.stopPropagation();
                      onRecover();
                    }}
                    aria-label={t("threads.recoveryAction")}
                  >
                    <RefreshCcw className="h-2.5 w-2.5" />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{t("threads.recoveryAction")}</TooltipContent>
              </Tooltip>
            )}

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-5.5 w-5.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  disabled={!hasRollout}
                  aria-label={hasRollout ? t("threads.exportThread") : t("threads.exportUnavailable")}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!hasRollout) return;
                    onExport();
                  }}
                >
                  <Download className="h-2.5 w-2.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {hasRollout ? t("threads.exportThread") : t("threads.exportUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-5.5 w-5.5 text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-transform"
                  disabled={!hasRollout}
                  aria-label={hasRollout ? t("threads.openRawFile") : t("threads.rawUnavailable")}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!hasRollout) return;
                    onOpenRaw();
                  }}
                >
                  <FolderOpen className="h-2.5 w-2.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {hasRollout ? t("threads.openRawFile") : t("threads.rawUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-5.5 w-5.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive active:scale-95 transition-transform"
                  aria-label={t("threads.deleteThread")}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete();
                  }}
                >
                  <Trash2 className="h-2.5 w-2.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.deleteThread")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
      </BentoCard>
    </TooltipProvider>
  );
}
