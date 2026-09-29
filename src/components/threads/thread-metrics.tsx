import { useTranslation } from "react-i18next";
import { CheckSquare, FolderTree, HardDrive, MessagesSquare, Sparkles } from "lucide-react";
import { AuroraCard } from "@/components/ui/aurora-card";
import { formatSize, formatTokenCount } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";
import { focusRing, pressable } from "@/lib/a11y";

export function ThreadMetrics({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    threads, archivedCount, groups, totalInteractions, totalUserMsgs, totalAgentMsgs, totalTokens, totalBytes,
    status, setStatus, selected, clearSelection, selectFiltered,
  } = list;

  return (
    <div className="app-metric-grid grid grid-cols-2 gap-2 lg:grid-cols-4">
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
              <span className="text-xl font-bold tracking-tight tabular-nums">
                {threads.length}
              </span>
              <span className="text-[11px] text-muted-foreground">
                {threads.length - archivedCount} 进行中 · {archivedCount} 归档
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
              <span className="text-xl font-bold tracking-tight tabular-nums">
                {groups.length}
              </span>
            </div>
          </div>
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <FolderTree className="h-5 w-5" />
          </div>
        </div>
      </AuroraCard>

      <AuroraCard color="blue">
        <div className="flex items-center justify-between p-2.5">
          <div className="space-y-0.5">
            <span className="text-xs font-medium text-muted-foreground">
              {t("threads.statsInteractions")}
            </span>
            <div className="flex items-baseline gap-2">
              <span className="text-xl font-bold tracking-tight tabular-nums">
                {totalInteractions.toLocaleString()}
              </span>
              <span className="text-[11px] text-muted-foreground">
                U: {totalUserMsgs} · AI: {totalAgentMsgs} · {formatTokenCount(totalTokens)} {t("threads.tokenUnit")}
              </span>
            </div>
          </div>
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <Sparkles className="h-5 w-5" />
          </div>
        </div>
      </AuroraCard>

      <AuroraCard
        color="blue"
        shine={selected.size > 0}
        className="cursor-pointer hover:border-primary/35"
        onClick={() => {
          if (selected.size > 0) clearSelection();
          else selectFiltered();
        }}
      >
        <div className="flex items-center justify-between p-2.5">
          <div className="space-y-0.5">
            <span className="text-xs font-medium text-muted-foreground">
              {selected.size > 0 ? t("threads.statsSelected", { count: selected.size }) : t("threads.statsStorage")}
            </span>
            <div className="flex items-baseline gap-2">
              <span className="text-xl font-bold tracking-tight tabular-nums">
                {selected.size > 0 ? selected.size : formatSize(totalBytes)}
              </span>
              {selected.size > 0 && <span className="text-[11px] text-muted-foreground">点击取消全选</span>}
            </div>
          </div>
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
            {selected.size > 0 ? <CheckSquare className="h-5 w-5" /> : <HardDrive className="h-5 w-5" />}
          </div>
        </div>
      </AuroraCard>
    </div>
  );
}
