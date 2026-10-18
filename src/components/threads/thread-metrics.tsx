import { useTranslation } from "react-i18next";
import { Coins, FolderTree, HardDrive, MessagesSquare } from "lucide-react";
import { formatSize, formatTokenCount } from "@/components/threads/thread-utils";
import { formatCacheRate } from "@/lib/cache-usage";
import type { ThreadList } from "@/components/threads/use-thread-list";
import { focusRing, pressable } from "@/lib/a11y";
import { cn } from "@/lib/utils";

// What the current project scope holds: conversations, projects, disk and tokens.
export function ThreadMetrics({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    scopedThreads: threads, archivedCount, workspaceCount, totalTokens,
    totalInputTokens, totalCachedTokens, totalOutputTokens, hasTokenBreakdown,
    totalBytes, activeProject, status, setStatus,
  } = list;
  const summaries = [
    {
      name: "threads", label: t("threads.statsThreads"), value: threads.length, Icon: MessagesSquare,
      hint: list.isCodex ? t("threads.statsStatus", { active: threads.length - archivedCount, archived: archivedCount }) : "Claude Code",
    },
    { name: "projects", label: t("threads.statsFolders"), value: workspaceCount, Icon: FolderTree, hint: "" },
    { name: "storage", label: t("threads.statsStorage"), value: formatSize(totalBytes), Icon: HardDrive, hint: "" },
  ];
  // Prompt tokens include the cached subset, so input reads as the usage
  // page's: what was not cached. The hit rate is the cached share of input.
  const fresh = Math.max(0, totalInputTokens - totalCachedTokens);
  const cacheRate = totalInputTokens > 0 ? Math.min(totalCachedTokens, totalInputTokens) / totalInputTokens * 100 : null;
  const parts = [
    { name: "input", label: t("threads.statsInputLabel"), value: formatTokenCount(fresh) },
    { name: "cached", label: t("threads.statsCachedLabel"), value: formatTokenCount(totalCachedTokens) },
    { name: "output", label: t("threads.statsOutputLabel"), value: formatTokenCount(totalOutputTokens) },
    { name: "cache-rate", label: t("accounts.cacheHitRate"), value: formatCacheRate(cacheRate) },
  ];

  return (
    <section aria-label={t("threads.overview")} data-thread-metrics className="thread-stats relative border-t border-border/60 bg-background/40 px-5 py-3.5 sm:px-6">
      <p className="text-[11px] text-muted-foreground" data-metric-scope>
        {t("threads.statsScope", { name: activeProject?.folderName ?? t("threads.allProjects") })}
      </p>
      <div className="thread-stats-grid mt-2">
        {summaries.map(({ name, label, value, Icon, hint }) => (
          <div key={name}
            className={cn("thread-stat",
              list.isCodex && name === "threads" && "cursor-pointer transition-colors hover:border-primary/40 " + focusRing,
              name === "threads" && status === "active" && "border-primary/50 bg-primary/[0.05]")}
            {...(list.isCodex && name === "threads" ? {
              "aria-pressed": status === "active",
              ...pressable(() => setStatus(status === "active" ? "all" : "active")),
            } : {})}>
            <span className="thread-stat-label"><Icon aria-hidden className="h-3.5 w-3.5" />{label}</span>
            <span data-metric={name} className="thread-stat-value">{value}</span>
            <span className="thread-stat-hint">{hint || " "}</span>
          </div>
        ))}

        <div className="thread-stat thread-stat-tokens">
          {hasTokenBreakdown ? (
            <>
              <div className="flex items-center justify-between gap-2">
                <span className="thread-stat-label"><Coins aria-hidden className="h-3.5 w-3.5" />Token</span>
                <span data-metric="tokens-total" className="thread-stat-hint">
                  {t("threads.statsTokens", { value: formatTokenCount(totalTokens) })}
                </span>
              </div>
              <div className="thread-token-parts mt-1.5">
                {parts.map(({ name, label, value }) => (
                  <div key={name} className="min-w-0">
                    <span className="block truncate text-[11px] text-muted-foreground">{label}</span>
                    <span data-metric={name} className="thread-stat-value text-lg">{value}</span>
                  </div>
                ))}
              </div>
            </>
          ) : (
            <>
              <span className="thread-stat-label"><Coins aria-hidden className="h-3.5 w-3.5" />{t("threads.statsTokensLabel")}</span>
              <span data-metric="tokens" className="thread-stat-value">{formatTokenCount(totalTokens)}</span>
              <span className="thread-stat-hint">{t("threads.statsBreakdownUnknown")}</span>
            </>
          )}
        </div>
      </div>
    </section>
  );
}
