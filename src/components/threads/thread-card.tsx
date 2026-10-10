import { useState, type MouseEvent, type ReactNode } from "react";
import {
  AlertTriangle, Archive, Check, Copy, Download, FolderOpen, HardDrive, MessageSquare, RefreshCcw, Trash2, Zap,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { ThreadSummary } from "@/types/threads";
import { ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import { formatSize, formatTokenCount } from "./thread-utils";
import { focusRing, onActivateKey } from "@/lib/a11y";
import { ThreadResumeButton } from "./thread-resume-button";

interface ThreadCardProps {
  thread: ThreadSummary;
  selected: boolean;
  detailActive?: boolean;
  /** Stored under a provider Codex does not list right now. */
  misplaced?: boolean;
  onToggleSelect: () => void;
  onExport: () => void;
  onOpenRaw: () => void;
  onRecover?: () => void;
  onDelete?: () => void;
  onDetails?: () => void;
  variant?: "grid" | "compact";
}

const badge = "inline-flex shrink-0 items-center gap-1 rounded-full px-1.5 py-px text-[10px] font-medium";

/** The client's mark with a status light: live, archived or missing its log. */
function ThreadAvatar({ thread, size = "md" }: { thread: ThreadSummary; size?: "md" | "lg" }) {
  const claude = thread.threadSource === "claudeJsonl";
  const state = thread.rolloutStatus === "missing" ? "missing" : thread.archived ? "archived" : "live";
  return (
    <span aria-hidden data-thread-state={state}
      className={cn("thread-avatar relative flex shrink-0 items-center justify-center rounded-xl bg-background ring-1 ring-border/80",
        size === "lg" ? "h-9 w-9" : "h-8 w-8")}>
      <ClientGlyph client={claude ? "claudeCode" : "codex"} className={size === "lg" ? "h-[18px] w-[18px]" : "h-4 w-4"} />
      <span className="thread-avatar-light" />
    </span>
  );
}

export function ThreadCard({
  thread,
  selected,
  detailActive = false,
  misplaced = false,
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

  const isClaude = thread.threadSource === "claudeJsonl";
  const hasRollout = thread.rolloutStatus !== "missing";
  const canRecover = !!onRecover && thread.id.length > 0;
  const title = thread.title || t("threads.untitled");
  const messages = (thread.userMessages || 0) + (thread.agentMessages || 0);
  const modelLabel = thread.model || t("threads.unknownModel");
  const providerLabel = thread.modelProvider || (isClaude ? "" : t("threads.unknownProvider"));

  const handleCopyId = (event: MouseEvent) => {
    event.stopPropagation();
    if (!thread.id) return;
    void navigator.clipboard.writeText(thread.id);
    setCopiedId(true);
    toast({ title: t("threads.copiedId"), description: thread.id, variant: "default" });
    setTimeout(() => setCopiedId(false), 2000);
  };

  const checkbox = (
    <span role="checkbox" aria-checked={selected} aria-label={title} tabIndex={0} onKeyDown={onActivateKey(onToggleSelect)}
      className={cn("thread-check flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-md border transition-colors duration-150",
        focusRing,
        selected ? "border-primary bg-primary text-primary-foreground" : "border-border bg-background text-transparent group-hover:border-primary/60")}>
      <Check className="h-3 w-3" strokeWidth={3} />
    </span>
  );

  const titleButton = (className: string) => (
    <button type="button" data-thread-title aria-current={detailActive ? "true" : undefined}
      className={cn("text-left font-semibold tracking-tight text-foreground transition-colors hover:text-primary", focusRing,
        detailActive && "text-primary", className)}
      title={title} aria-label={t("threads.details.open", { title })}
      onClick={(event) => { event.stopPropagation(); onDetails?.(); }}>
      {title}
    </button>
  );

  const badges = (
    <>
      {!isClaude && thread.archived && (
        <span className={cn(badge, "bg-amber-500/10 text-amber-700 dark:text-amber-400")}>
          <Archive className="h-2.5 w-2.5" />{t("threads.archivedBadge")}
        </span>
      )}
      {misplaced && (
        <span className={cn(badge, "bg-orange-500/10 text-orange-700 dark:text-orange-400")} data-thread-misplaced
          title={t("threads.misplacedHint", { provider: thread.modelProvider })}>
          <AlertTriangle className="h-2.5 w-2.5" />{t("threads.misplacedBadge")}
        </span>
      )}
      {!hasRollout && (
        <span className={cn(badge, "bg-destructive/10 text-destructive")}>
          <AlertTriangle className="h-2.5 w-2.5" />{t("threads.rolloutMissing")}
        </span>
      )}
    </>
  );

  // Optional actions give way to the title on narrow rows; details still offer them.
  const action = (label: string, icon: ReactNode, onClick: () => void, options: { disabled?: boolean; danger?: boolean; optional?: boolean } = {}) => (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={label} disabled={options.disabled} data-row-optional={options.optional || undefined}
          className={cn("h-7 w-7 rounded-lg text-muted-foreground transition-colors active:scale-95",
            options.danger ? "hover:bg-destructive/10 hover:text-destructive" : "hover:bg-primary/10 hover:text-primary")}
          onClick={(event) => { event.stopPropagation(); if (!options.disabled) onClick(); }}>
          {icon}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );

  const actions = (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button variant="ghost" size="icon-sm" onClick={handleCopyId} aria-label={t("threads.copyId")} data-row-optional
            className="h-7 w-7 rounded-lg text-muted-foreground transition-colors hover:bg-primary/10 hover:text-primary active:scale-95">
            {copiedId ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          </Button>
        </TooltipTrigger>
        <TooltipContent>{t("threads.copyId")}</TooltipContent>
      </Tooltip>
      {isClaude && <ThreadResumeButton id={thread.id} />}
      {canRecover && action(t("threads.recoveryAction"), <RefreshCcw className="h-3.5 w-3.5" />, () => onRecover?.())}
      {action(hasRollout ? t("threads.exportThread") : t("threads.exportUnavailable"), <Download className="h-3.5 w-3.5" />, onExport,
        { disabled: !hasRollout })}
      {action(hasRollout ? t("threads.openRawFile") : t("threads.rawUnavailable"), <FolderOpen className="h-3.5 w-3.5" />, onOpenRaw,
        { disabled: !hasRollout, optional: true })}
      {onDelete && action(t("threads.deleteThread"), <Trash2 className="h-3.5 w-3.5" />, onDelete, { danger: true })}
    </>
  );

  const time = (
    <span title={`${t("threads.activeTime", { time: formatDateTime(thread.updatedAt) })}${thread.createdAt
      ? `\n${t("threads.createdTime", { time: formatDateTime(thread.createdAt) })}` : ""}`}>
      {formatRelative(thread.updatedAt)}
    </span>
  );

  // Compact row — fixed height so virtualization can measure rows.
  if (variant === "compact") {
    return (
      <TooltipProvider delayDuration={200}>
        <div data-thread-card={thread.key} data-detail-active={detailActive ? "" : undefined}
          className={cn("thread-compact-card thread-row group relative flex h-13 cursor-pointer items-center gap-3 overflow-hidden rounded-2xl border bg-card/90 px-3 transition-[border-color,box-shadow,background-color] duration-150 motion-reduce:transition-none",
            selected ? "border-primary/60 bg-primary/[0.04] ring-1 ring-primary/30"
              : detailActive ? "border-primary/45" : !hasRollout ? "border-destructive/40" : "border-border/70")}
          onClick={onToggleSelect}>
          {checkbox}
          <ThreadAvatar thread={thread} />
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-1.5">
              {titleButton("truncate text-[13.5px] leading-5")}
              {badges}
            </div>
            <div className="flex min-w-0 items-center gap-1.5 text-[11px] leading-4 text-muted-foreground">
              {time}
              <span aria-hidden className="text-muted-foreground/40">·</span>
              <span data-thread-row-secondary className="min-w-0 truncate font-mono" title={t("threads.providerAndModel", { provider: providerLabel, model: modelLabel })}>
                {modelLabel}
              </span>
            </div>
          </div>
          <div data-thread-row-stats className="hidden shrink-0 items-center gap-3 font-mono text-[11px] text-muted-foreground sm:flex">
            <span className="flex items-center gap-1" title={t("threads.messagesTooltip")}>
              <MessageSquare className="h-3 w-3 opacity-70" />{messages}
            </span>
            <span data-thread-row-secondary className="flex w-14 items-center gap-1" title={t("threads.tokensUsed", { count: thread.tokensUsed || 0 })}>
              <Zap className="h-3 w-3 opacity-70" />{formatTokenCount(thread.tokensUsed)}
            </span>
          </div>
          <div className="thread-row-actions flex shrink-0 items-center gap-0.5">{actions}</div>
        </div>
      </TooltipProvider>
    );
  }

  // Grid card: the name leads, then when and with what; the log stats sit in the footer.
  return (
    <TooltipProvider delayDuration={200}>
      <div data-thread-card={thread.key} data-detail-active={detailActive ? "" : undefined} data-spotlight
        className={cn("thread-tile group relative flex min-h-[168px] cursor-pointer flex-col gap-3 rounded-2xl border bg-card/90 p-3.5 transition-[border-color,box-shadow,transform] duration-200 hover:-translate-y-0.5 hover:shadow-md motion-reduce:transition-none motion-reduce:hover:translate-y-0",
          selected ? "border-primary/60 ring-1 ring-primary/30"
            : detailActive ? "border-primary/45" : !hasRollout ? "border-destructive/40" : "border-border/70")}
        onClick={onToggleSelect}>
        <div className="flex items-start gap-2.5">
          <ThreadAvatar thread={thread} size="lg" />
          <div className="min-w-0 flex-1">
            <h3 className="flex items-start">{titleButton("line-clamp-2 text-[14px] leading-snug")}</h3>
            <p className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
              {time}
              <span aria-hidden className="text-muted-foreground/40">·</span>
              <span className="truncate font-mono" title={t("threads.providerAndModel", { provider: providerLabel, model: modelLabel })}>{modelLabel}</span>
            </p>
          </div>
          {checkbox}
        </div>
        <div className="flex min-h-5 flex-wrap items-center gap-1.5">
          {badges}
          {providerLabel && (
            <span className={cn(badge, "max-w-full bg-muted/70 font-mono text-muted-foreground")} title={t("threads.threadSource")}>
              <span className="truncate">{providerLabel}</span>
            </span>
          )}
          {(thread.cliVersion || (!isClaude && thread.source)) && (
            <span className={cn(badge, "bg-muted/70 font-mono text-muted-foreground")}>
              {thread.cliVersion ? `v${thread.cliVersion}` : thread.source}
            </span>
          )}
        </div>
        <div className="mt-auto flex items-center justify-between gap-2 border-t border-border/50 pt-2.5 text-[11px] text-muted-foreground">
          <div className="flex min-w-0 items-center gap-3 font-mono">
            <span className="flex items-center gap-1" title={t("threads.messagesTooltip")}>
              <MessageSquare className="h-3 w-3 opacity-70" />{messages}
            </span>
            <span className="flex items-center gap-1" title={t("threads.tokensUsed", { count: thread.tokensUsed || 0 })}>
              <Zap className="h-3 w-3 opacity-70" />{formatTokenCount(thread.tokensUsed)}
            </span>
            <span className="flex items-center gap-1">
              <HardDrive className="h-3 w-3 opacity-70" />{formatSize(thread.fileSize)}
            </span>
          </div>
          <div className="thread-row-actions flex items-center gap-0.5">{actions}</div>
        </div>
      </div>
    </TooltipProvider>
  );
}
