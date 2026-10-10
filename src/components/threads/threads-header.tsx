import { useTranslation } from "react-i18next";
import { MessagesSquare, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import { ThreadToolbar } from "@/components/threads/thread-toolbar";
import { ThreadMetrics } from "@/components/threads/thread-metrics";
import type { ConversationSource } from "@/types/threads";
import type { ThreadList } from "@/components/threads/use-thread-list";
import { PosterWatermark } from "@/components/ui/poster-watermark";

// The page's search console: which client, what to find, and what is in scope.
export function ThreadsHeader({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const { source, setSource, sourceBusy, refreshAction, handleRefresh } = list;

  return (
    <section className="route-surface route-rise relative overflow-hidden rounded-3xl border border-border/80 bg-card shadow-sm">
      <PosterWatermark text="THREADS" className="right-6 top-4 text-[clamp(3rem,7vw,5.5rem)]" />
      <div className="relative flex flex-col gap-3 px-5 pt-5 sm:flex-row sm:flex-wrap sm:items-center sm:justify-between sm:px-6">
        <div className="flex min-w-0 items-center gap-3">
          <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-primary/10 text-primary ring-1 ring-primary/20">
            <MessagesSquare className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <h1 className="poster-title text-2xl">{t("threads.title")}</h1>
            <p className="text-[13px] text-muted-foreground">{t("threads.heroSubtitle")}</p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div data-thread-source-controls>
            <SegmentedOptions
              items={[
                { value: "codex", label: "Codex", disabled: sourceBusy },
                { value: "claudeCode", label: "Claude Code", disabled: sourceBusy },
              ]}
              value={source}
              onChange={(value) => setSource(value as ConversationSource)}
            />
          </div>
          <Button variant="outline" className="h-9 rounded-xl bg-background/80 px-3.5 text-xs font-semibold"
            onClick={() => void handleRefresh()} disabled={refreshAction.busy}>
            <ButtonBusyContent busy={refreshAction.busy} idleIcon={<RotateCw className="h-3.5 w-3.5" />} idleLabel={t("threads.refresh")} />
          </Button>
        </div>
      </div>
      <ThreadToolbar list={list} />
      <ThreadMetrics list={list} />
    </section>
  );
}
