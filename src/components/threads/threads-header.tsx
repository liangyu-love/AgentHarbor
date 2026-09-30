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
    format, setFormat, threads, bulkRepairing, handleRepairAll, refreshAction, handleRefresh,
  } = list;

  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="space-y-0.5">
        <h1 className="text-lg font-bold tracking-tight text-foreground flex items-center gap-2">
          <MessagesSquare className="h-4.5 w-4.5 text-primary" />
          {t("threads.title")}
        </h1>
      </div>

      <div className="flex flex-wrap items-center gap-2.5">
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
          className="h-9 gap-1.5 rounded-[10px] px-3 text-[13px]"
          onClick={() => void handleRepairAll()}
          disabled={bulkRepairing || threads.length === 0}
        >
          {bulkRepairing ? (
            <RotateCw className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Sparkles className="h-3.5 w-3.5" />
          )}
          {bulkRepairing ? t("threads.repairingAll") : t("threads.repairAll")}
        </Button>

        {/* Refresh Button */}
        <RainbowButton
          className="h-9 rounded-[10px] px-3.5 text-[13px] font-medium"
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
