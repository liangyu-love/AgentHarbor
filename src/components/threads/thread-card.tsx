import { useState } from "react";
import {
  AlertTriangle,
  Archive,
  ArrowDown,
  ArrowUp,
  Check,
  CheckSquare,
  Clock,
  Coins,
  Copy,
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
  detailActive?: boolean;
  onToggleSelect: () => void;
  onExport: () => void;
  onOpenRaw: () => void;
  onRecover: () => void;
  onDelete: () => void;
  onDetails?: () => void;
  variant?: "grid" | "compact";
}

export function ThreadCard({
  thread,
  selected,
  detailActive = false,
  onToggleSelect,
  onExport,
  onOpenRaw,
  onRecover,
  onDelete,
  onDetails,
  variant = "grid",
}: ThreadCardProps) {
  const { t } = useTranslation();
  const [copiedId, setCopiedId] = useState(false);

  const hasTokenSplit = thread.inputTokens != null || thread.outputTokens != null;
  const inputTokens = thread.inputTokens ?? 0;
  const outputTokens = thread.outputTokens ?? 0;
  const splitTotal = inputTokens + outputTokens;
  const inputRatio = splitTotal > 0 ? Math.round((inputTokens / splitTotal) * 100) : 50;
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

  // Compact Row View — fixed height so virtualization can measure rows
  if (variant === "compact") {
    return (
      <TooltipProvider delayDuration={200}>
        <div
          data-thread-card={thread.key}
          data-detail-active={detailActive ? "" : undefined}
          className={cn(
            "thread-compact-card group relative flex h-13 cursor-pointer items-center justify-between gap-3 overflow-hidden rounded-xl border border-border/60 bg-card/85 px-3 backdrop-blur-sm transition-all duration-150 hover:border-primary/50 hover:bg-muted/40",
            selected
              ? "border-primary/70 bg-primary/[0.04] shadow-xs ring-1 ring-primary/40 dark:border-amber-400/50 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
              : !hasRollout
                ? "border-destructive/45 bg-destructive/[0.025]"
                : "",
          )}
          onClick={onToggleSelect}
        >
          {/* Left: Checkbox + Title + Status */}
          <div className="flex min-w-0 flex-1 items-center gap-2.5">
            <span
              role="checkbox"
              aria-checked={selected}
              aria-label={thread.title || t("threads.untitled")}
              tabIndex={0}
              onKeyDown={onActivateKey(onToggleSelect)}
              className={cn(
                "flex h-4.5 w-4.5 shrink-0 items-center justify-center rounded-md border transition-all duration-150",
                focusRing,
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-3 w-3" />
            </span>

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  data-thread-title
                  aria-current={detailActive ? "true" : undefined}
                  className={cn(
                    "truncate text-left text-sm font-semibold leading-none text-foreground hover:text-primary transition-colors",
                    focusRing,
                    detailActive && "text-primary",
                  )}
                  title={thread.title || t("threads.untitled")}
                  aria-label={t("threads.details.open", {
                    title: thread.title || t("threads.untitled"),
                  })}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDetails?.();
                  }}
                >
                  {thread.title || t("threads.untitled")}
                </button>

                {thread.archived ? (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                    <Archive className="h-2.5 w-2.5" />
                    {t("threads.archivedBadge")}
                  </span>
                ) : (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                    {t("threads.activeBadge")}
                  </span>
                )}

                {!hasRollout && (
                  <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-destructive/10 px-2 py-0.5 text-[10px] font-medium text-destructive">
                    <AlertTriangle className="h-2.5 w-2.5" />
                    {t("threads.rolloutMissing")}
                  </span>
                )}

                <span
                  data-thread-row-secondary
                  className="hidden max-w-44 shrink-0 truncate rounded-md bg-primary/10 px-2 py-0.5 font-mono text-[10px] font-medium text-primary md:inline-flex"
                  title={t("threads.providerAndModel", {
                    provider: providerLabel,
                    model: modelLabel,
                  })}
                >
                  {providerLabel} · {modelLabel}
                </span>

                <span
                  data-thread-row-secondary
                  className="hidden shrink-0 items-center gap-1 rounded-md bg-muted/70 px-1.5 py-0.5 text-[9.5px] text-muted-foreground lg:inline-flex"
                  title={t("threads.threadSource")}
                >
                  <Database className="h-2.5 w-2.5 opacity-70" />
                  {sourceLabel}
                </span>

                {(thread.cliVersion || thread.source) && (
                  <span data-thread-row-secondary className="hidden md:inline-flex items-center gap-1 rounded-md bg-muted/70 px-1.5 py-0.5 font-mono text-[9.5px] text-muted-foreground">
                    <Terminal className="h-2.5 w-2.5 opacity-70" />
                    {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
                  </span>
                )}
              </div>
            </div>
          </div>

          {/* Center: Turn counts & Time */}
          <div data-thread-row-stats className="hidden shrink-0 items-center gap-3.5 text-xs text-muted-foreground sm:flex">
            <div
              data-thread-row-secondary
              className="flex items-center gap-1.5 font-mono rounded-lg bg-muted/50 px-2 py-1"
              title={t("threads.messagesTooltip")}
            >
              <span className="flex items-center gap-1 text-sky-600 dark:text-sky-400 font-semibold">
                <User className="h-3 w-3" />
                {thread.userMessages}
              </span>
              <span className="text-muted-foreground/40">/</span>
              <span className="flex items-center gap-1 text-purple-600 dark:text-purple-400 font-semibold">
                <Sparkles className="h-3 w-3" />
                {thread.agentMessages}
              </span>
            </div>

            <span
              className="flex items-center gap-1 font-mono text-[11px]"
              title={t("threads.tokensUsed", { count: thread.tokensUsed || 0 })}
            >
              <Coins className="h-3 w-3 opacity-70" />
              {formatTokenCount(thread.tokensUsed)}
            </span>

            <span className="w-13 text-right font-mono text-[11px]">
              {formatSize(thread.fileSize)}
            </span>

            <span
              data-thread-row-secondary
              className="w-16 text-right text-[11px]"
              title={formatDateTime(thread.updatedAt)}
            >
              {formatRelative(thread.updatedAt)}
            </span>
          </div>

          {/* Right: Actions */}
          <div className="flex shrink-0 items-center gap-1 opacity-70 group-hover:opacity-100 transition-opacity">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                  onClick={handleCopyId}
                  aria-label={t("threads.copyId")}
                >
                  {copiedId ? (
                    <Check className="h-3.5 w-3.5 text-emerald-500" />
                  ) : (
                    <Copy className="h-3.5 w-3.5" />
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
                    className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                    onClick={(e) => {
                      e.stopPropagation();
                      onRecover();
                    }}
                    aria-label={t("threads.recoveryAction")}
                  >
                    <RefreshCcw className="h-3.5 w-3.5" />
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
                  className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                  disabled={!hasRollout}
                  aria-label={
                    hasRollout
                      ? t("threads.exportThread")
                      : t("threads.exportUnavailable")
                  }
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!hasRollout) return;
                    onExport();
                  }}
                >
                  <Download className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {hasRollout
                  ? t("threads.exportThread")
                  : t("threads.exportUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                  disabled={!hasRollout}
                  aria-label={
                    hasRollout
                      ? t("threads.openRawFile")
                      : t("threads.rawUnavailable")
                  }
                  onClick={(e) => {
                    e.stopPropagation();
                    if (!hasRollout) return;
                    onOpenRaw();
                  }}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>
                {hasRollout
                  ? t("threads.openRawFile")
                  : t("threads.rawUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-destructive/10 hover:text-destructive active:scale-95 transition-all"
                  aria-label={t("threads.deleteThread")}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDelete();
                  }}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.deleteThread")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
      </TooltipProvider>
    );
  }

  // Bento Card Grid View (Default) — high-craft interactive card
  return (
    <TooltipProvider delayDuration={200}>
      <BentoCard
        data-thread-card={thread.key}
        data-detail-active={detailActive ? "" : undefined}
        className={cn(
          "group relative flex min-h-[180px] flex-col justify-between cursor-pointer rounded-2xl p-3.5 transition-all duration-200 ease-out hover:-translate-y-1 hover:border-primary/50 hover:shadow-md",
          selected
            ? "border-primary/70 bg-primary/[0.04] ring-2 ring-primary/30 shadow-md dark:border-amber-400/60 dark:ring-amber-400/30 dark:bg-amber-400/[0.03]"
            : !hasRollout
              ? "border-destructive/45 bg-destructive/[0.025]"
              : "border-border/70 bg-gradient-to-b from-card/95 via-card/85 to-card/65 backdrop-blur-md shadow-2xs",
        )}
        onClick={onToggleSelect}
      >
        {/* Top: Checkbox, Title & Status */}
        <div className="space-y-2">
          <div className="flex items-start gap-2">
            <span
              role="checkbox"
              aria-checked={selected}
              aria-label={thread.title || t("threads.untitled")}
              tabIndex={0}
              onKeyDown={onActivateKey(onToggleSelect)}
              className={cn(
                "mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-md border transition-all duration-150",
                focusRing,
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-xs"
                  : "border-border/80 bg-background text-transparent group-hover:border-primary/60",
              )}
            >
              <CheckSquare className="h-3 w-3" />
            </span>

            <div className="min-w-0 flex-1">
              <h3 className="flex min-h-[2.75em] items-center">
                <button
                  type="button"
                  data-thread-title
                  aria-current={detailActive ? "true" : undefined}
                  className={cn(
                    "line-clamp-2 text-left text-sm font-bold leading-snug tracking-tight text-foreground hover:text-primary transition-colors",
                    focusRing,
                    detailActive && "text-primary",
                  )}
                  title={thread.title || t("threads.untitled")}
                  aria-label={t("threads.details.open", {
                    title: thread.title || t("threads.untitled"),
                  })}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDetails?.();
                  }}
                >
                  {thread.title || t("threads.untitled")}
                </button>
              </h3>
            </div>
          </div>

          {/* Badges bar */}
          <div className="flex flex-wrap items-center gap-1.5">
            {thread.archived ? (
              <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[9.5px] font-medium text-amber-600 dark:text-amber-400">
                <Archive className="h-2.5 w-2.5" />
                {t("threads.archivedBadge")}
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[9.5px] font-medium text-emerald-600 dark:text-emerald-400">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                {t("threads.activeBadge")}
              </span>
            )}

            {!hasRollout && (
              <span className="inline-flex items-center gap-1 rounded-full bg-destructive/10 px-2 py-0.5 text-[9.5px] font-medium text-destructive">
                <AlertTriangle className="h-2.5 w-2.5" />
                {t("threads.rolloutMissing")}
              </span>
            )}

            <span
              className="inline-flex max-w-full items-center rounded-md bg-primary/10 px-1.5 py-0.5 font-mono text-[9.5px] font-medium text-primary"
              title={t("threads.providerAndModel", {
                provider: providerLabel,
                model: modelLabel,
              })}
            >
              <span className="max-w-[180px] sm:max-w-[220px] truncate">
                {providerLabel} · {modelLabel}
              </span>
            </span>

            <span
              className="inline-flex items-center gap-1 rounded-md bg-muted/70 px-1.5 py-0.5 text-[9.5px] text-muted-foreground"
              title={t("threads.threadSource")}
            >
              <Database className="h-2.5 w-2.5 opacity-70" />
              {sourceLabel}
            </span>

            {(thread.cliVersion || thread.source) && (
              <span className="inline-flex items-center gap-1 rounded-md bg-muted/70 px-1.5 py-0.5 font-mono text-[9.5px] text-muted-foreground">
                <Terminal className="h-2.5 w-2.5 opacity-70" />
                {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
              </span>
            )}
          </div>
        </div>

        {/* Middle: Token split capsule */}
        <div className="mt-3 flex items-center justify-between gap-2 rounded-xl bg-muted/35 px-2.5 py-1.5 text-[11px] border border-border/30">
          <div className="flex min-w-0 items-center gap-2 font-mono">
            {hasTokenSplit && (
              <>
                <span
                  className="flex items-center gap-1 text-sky-600 dark:text-sky-400 font-semibold"
                  title={t("threads.statsInputLabel")}
                >
                  <ArrowDown className="h-3 w-3" />
                  {formatTokenCount(inputTokens)}
                </span>
                <span className="text-muted-foreground/30">/</span>
                <span
                  className="flex items-center gap-1 text-amber-600 dark:text-amber-400 font-semibold"
                  title={t("threads.statsOutputLabel")}
                >
                  <ArrowUp className="h-3 w-3" />
                  {formatTokenCount(outputTokens)}
                </span>
                <span className="text-muted-foreground/30">·</span>
              </>
            )}
            <span
              className="flex min-w-0 items-center gap-1 text-muted-foreground/80"
              title={t("threads.tokensUsed", { count: thread.tokensUsed || 0 })}
            >
              <Coins className="h-3 w-3 opacity-70" />
              {formatTokenCount(thread.tokensUsed)}
            </span>
          </div>

          {hasTokenSplit && (
            <div
              className="h-2 w-16 shrink-0 overflow-hidden rounded-full bg-muted/80 flex ring-1 ring-border/20 shadow-2xs"
              title={t("threads.tokenSplit", { input: inputRatio, output: 100 - inputRatio })}
            >
              <div
                className="bg-gradient-to-r from-sky-500 to-sky-400"
                style={{ width: `${inputRatio}%` }}
              />
              <div
                className="bg-gradient-to-r from-amber-500 to-amber-400"
                style={{ width: `${100 - inputRatio}%` }}
              />
            </div>
          )}
        </div>

        {/* Bottom: Meta & Quick Actions */}
        <div className="mt-2.5 flex items-center justify-between border-t border-border/40 pt-2 text-[11px] text-muted-foreground">
          <div className="flex min-w-0 items-center gap-2.5">
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex items-center gap-1 hover:text-foreground transition-colors cursor-help">
                  <Clock className="h-3 w-3 opacity-70" />
                  {formatRelative(thread.updatedAt)}
                </span>
              </TooltipTrigger>
              <TooltipContent className="text-xs space-y-0.5">
                <div>{t("threads.activeTime", { time: formatDateTime(thread.updatedAt) })}</div>
                {thread.createdAt && (
                  <div className="text-muted-foreground">
                    {t("threads.createdTime", { time: formatDateTime(thread.createdAt) })}
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
          <div className="flex items-center gap-0.5">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 rounded-md text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                  onClick={handleCopyId}
                  aria-label={t("threads.copyId")}
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

            {canPreviewRecovery && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-6 w-6 rounded-md text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
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
                  className="h-6 w-6 rounded-md text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                  disabled={!hasRollout}
                  aria-label={
                    hasRollout
                      ? t("threads.exportThread")
                      : t("threads.exportUnavailable")
                  }
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
                {hasRollout
                  ? t("threads.exportThread")
                  : t("threads.exportUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 rounded-md text-muted-foreground hover:bg-primary/10 hover:text-primary active:scale-95 transition-all"
                  disabled={!hasRollout}
                  aria-label={
                    hasRollout
                      ? t("threads.openRawFile")
                      : t("threads.rawUnavailable")
                  }
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
                {hasRollout
                  ? t("threads.openRawFile")
                  : t("threads.rawUnavailable")}
              </TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 rounded-md text-muted-foreground hover:bg-destructive/10 hover:text-destructive active:scale-95 transition-all"
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
      </BentoCard>
    </TooltipProvider>
  );
}
