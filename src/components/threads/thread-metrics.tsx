import { useTranslation } from "react-i18next";
import {
  ArrowDown,
  ArrowUp,
  Coins,
  FolderTree,
  HardDrive,
  Layers,
  MessagesSquare,
} from "lucide-react";
import { formatSize, formatTokenCount } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";
import { focusRing, pressable } from "@/lib/a11y";
import { cn } from "@/lib/utils";

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
    { name: "projects", label: t("threads.statsFolders"), value: workspaceCount, Icon: FolderTree },
    { name: "storage", label: t("threads.statsStorage"), value: formatSize(totalBytes), Icon: HardDrive },
  ];
  const tokenParts = [
    { name: "input", label: t("threads.statsInputLabel"), value: totalInputTokens,
      hint: t("threads.statsInputHint"), Icon: ArrowDown, color: "text-sky-500" },
    { name: "cached", label: t("threads.statsCachedLabel"), value: totalCachedTokens,
      hint: t("threads.statsCachedHint"), Icon: Layers, color: "text-emerald-500" },
    { name: "output", label: t("threads.statsOutputLabel"), value: totalOutputTokens,
      hint: t("threads.statsOutputHint"), Icon: ArrowUp, color: "text-amber-500" },
  ];

  return (
    <section aria-label={t("threads.overview")} data-thread-metrics className="thread-overview space-y-1.5">
      <p className="text-xs text-muted-foreground" data-metric-scope>
        {t("threads.statsScope", { name: activeProject?.folderName ?? t("threads.allProjects") })}
      </p>
      <div className="thread-overview-grid">
        {summaries.map(({ name, label, value, Icon, hint }) => (
          <div
            key={name}
            className={cn(
              "thread-overview-card",
              name === "storage" && "thread-storage-summary",
              list.isCodex && name === "threads" && "cursor-pointer transition-colors hover:border-primary/40 " + focusRing,
              name === "threads" && status === "active" && "border-primary/50",
            )}
            {...(list.isCodex && name === "threads" ? {
              "aria-pressed": status === "active",
              ...pressable(() => setStatus(status === "active" ? "all" : "active")),
            } : {})}
          >
            <dl className="thread-metric-item">
              <dt className="thread-overview-label" title={label}>
                <Icon aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
                <span className="truncate">{label}</span>
              </dt>
              <dd>
                <span data-metric={name} className="thread-overview-value">{value}</span>
                <p className="thread-overview-hint">{hint || "\u00a0"}</p>
              </dd>
            </dl>
          </div>
        ))}

        <div className="thread-overview-card thread-token-summary">
          {hasTokenBreakdown ? (
            <>
              <dl className="thread-token-parts">
                {tokenParts.map(({ name, label, value, hint, Icon, color }) => (
                  <div key={name} className="thread-metric-item">
                    <dt className="thread-overview-label" title={label}>
                      <Icon aria-hidden="true" className={cn("h-3.5 w-3.5 shrink-0", color)} />
                      <span className="truncate">{label}</span>
                    </dt>
                    <dd>
                      <span data-metric={name} className="thread-overview-value">{formatTokenCount(value)}</span>
                      <p className="thread-overview-hint">{hint}</p>
                    </dd>
                  </div>
                ))}
              </dl>
              <p data-metric="tokens-total" className="thread-token-total">
                {t("threads.statsTokens", { value: formatTokenCount(totalTokens) })}
              </p>
            </>
          ) : (
            <dl className="thread-metric-item">
              <dt className="thread-overview-label" title={t("threads.statsTokensLabel")}>
                <Coins aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
                <span className="truncate">{t("threads.statsTokensLabel")}</span>
              </dt>
              <dd>
                <span data-metric="tokens" className="thread-overview-value">{formatTokenCount(totalTokens)}</span>
                <p className="thread-overview-hint">{t("threads.statsBreakdownUnknown")}</p>
              </dd>
            </dl>
          )}
        </div>
      </div>
    </section>
  );
}
