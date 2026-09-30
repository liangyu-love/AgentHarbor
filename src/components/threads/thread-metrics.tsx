import { useTranslation } from "react-i18next";
import { FolderTree, HardDrive, MessagesSquare } from "lucide-react";
import { AuroraCard } from "@/components/ui/aurora-card";
import { formatSize, formatTokenCount } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";
import { focusRing, pressable } from "@/lib/a11y";

export function ThreadMetrics({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    scopedThreads: threads, archivedCount, workspaceCount, totalUserMsgs, totalAgentMsgs, totalTokens, totalBytes,
    activeProject, status, setStatus,
  } = list;

  return (
    <section aria-label={t("threads.overview")} data-thread-metrics className="space-y-2">
      <p className="text-xs text-muted-foreground" data-metric-scope>
        {t("threads.statsScope", { name: activeProject?.folderName ?? t("threads.allProjects") })}
      </p>
      <div className="app-metric-grid grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-[1fr_0.7fr_1.5fr_1fr]">
        <AuroraCard
          color="blue"
          className={"cursor-pointer hover:border-primary/35 " + focusRing}
          aria-pressed={status === "active"}
          {...pressable(() => setStatus(status === "active" ? "all" : "active"))}
        >
          <div className="flex items-center justify-between p-2.5">
            <div className="space-y-0.5">
              <span className="text-xs font-medium text-muted-foreground">
                {t("threads.statsThreads")}
              </span>
              <div className="flex items-baseline gap-2">
                <span data-metric="threads" className="text-xl font-bold tracking-tight tabular-nums">
                  {threads.length}
                </span>
                <span className="text-[11px] text-muted-foreground">
                  {t("threads.statsStatus", { active: threads.length - archivedCount, archived: archivedCount })}
                </span>
              </div>
            </div>
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <MessagesSquare className="h-5 w-5" />
            </div>
          </div>
        </AuroraCard>

        <AuroraCard color="blue">
          <div className="flex items-center justify-between p-2.5">
            <div className="space-y-0.5">
              <span className="text-xs font-medium text-muted-foreground">
                {t("threads.statsFolders")}
              </span>
              <div className="flex items-baseline gap-2">
                <span data-metric="projects" className="text-xl font-bold tracking-tight tabular-nums">
                  {workspaceCount}
                </span>
              </div>
            </div>
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <FolderTree className="h-5 w-5" />
            </div>
          </div>
        </AuroraCard>

        <AuroraCard color="blue">
          <div className="space-y-1 p-2.5">
            <dl className="grid grid-cols-2 gap-3">
              <div>
                <dt className="text-xs font-medium text-muted-foreground">{t("threads.statsInput")}</dt>
                <dd data-metric="input" className="text-xl font-bold tracking-tight tabular-nums">{totalUserMsgs.toLocaleString()}</dd>
              </div>
              <div>
                <dt className="text-xs font-medium text-muted-foreground">{t("threads.statsOutput")}</dt>
                <dd data-metric="output" className="text-xl font-bold tracking-tight tabular-nums">{totalAgentMsgs.toLocaleString()}</dd>
              </div>
            </dl>
            <p data-metric="tokens" className="text-[11px] text-muted-foreground">
              {t("threads.statsTokens", { value: formatTokenCount(totalTokens) })}
            </p>
          </div>
        </AuroraCard>

        <AuroraCard color="blue">
          <div className="flex items-center justify-between p-2.5">
            <div className="space-y-0.5">
              <span className="text-xs font-medium text-muted-foreground">
                {t("threads.statsStorage")}
              </span>
              <div className="flex items-baseline gap-2">
                <span data-metric="storage" className="text-xl font-bold tracking-tight tabular-nums">
                  {formatSize(totalBytes)}
                </span>
              </div>
            </div>
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <HardDrive className="h-5 w-5" />
            </div>
          </div>
        </AuroraCard>
      </div>
    </section>
  );
}
