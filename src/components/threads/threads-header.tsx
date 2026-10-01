import { useTranslation } from "react-i18next";
import { MessagesSquare, RotateCw, Sparkles } from "lucide-react";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import type { ExportFormat } from "@/components/threads/use-thread-list";
import type { ThreadList } from "@/components/threads/use-thread-list";

export function ThreadsHeader({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    format,
    setFormat,
    threads,
    bulkRepairing,
    handleRepairAll,
    refreshAction,
    handleRefresh,
  } = list;

  return (
    <div className="flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-center sm:justify-between">
      <div className="flex items-center gap-2.5">
        <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-gradient-to-br from-primary/20 via-primary/10 to-transparent text-primary shadow-xs ring-1 ring-primary/25">
          <MessagesSquare className="h-4.5 w-4.5" />
        </div>
        <div>
          <h1 className="text-xl font-extrabold tracking-tight text-foreground">
            {t("threads.title")}
          </h1>
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-2.5 sm:justify-end">
        {/* Format Selector */}
        <SegmentedOptions
          items={[
            { value: "markdown", label: t("threads.formatMarkdown") },
            { value: "jsonl", label: t("threads.formatJsonl") },
          ]}
          value={format}
          onChange={(value) => setFormat(value as ExportFormat)}
        />

        <Button
          variant="outline"
          size="sm"
          className="h-9 gap-1.5 rounded-xl border-border/80 px-3.5 text-xs font-semibold shadow-xs hover:border-primary/40 hover:bg-muted/80 active:scale-95 transition-all"
          onClick={() => void handleRepairAll()}
          disabled={bulkRepairing || threads.length === 0}
        >
          {bulkRepairing ? (
            <RotateCw className="h-3.5 w-3.5 animate-spin text-primary" />
          ) : (
            <Sparkles className="h-3.5 w-3.5 text-amber-500" />
          )}
          <span>{bulkRepairing ? t("threads.repairingAll") : t("threads.repairAll")}</span>
        </Button>

        {/* Refresh Button */}
        <RainbowButton
          className="h-9 rounded-xl px-4 text-xs font-semibold shadow-xs transition-transform active:scale-95"
          onClick={() => void handleRefresh()}
          disabled={refreshAction.busy}
        >
          <ButtonBusyContent
            busy={refreshAction.busy}
            idleIcon={<RotateCw className="h-3.5 w-3.5" />}
            idleLabel={t("threads.refresh")}
          />
        </RainbowButton>
      </div>
    </div>
  );
}
