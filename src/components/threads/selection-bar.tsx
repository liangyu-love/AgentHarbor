import { useTranslation } from "react-i18next";
import { Download, RotateCw, Square, Trash2 } from "lucide-react";
import { BentoCard } from "@/components/ui/bento-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { Button } from "@/components/ui/button";
import type { ThreadList } from "@/components/threads/use-thread-list";

// Sticky batch bar shown while threads are selected.
export function SelectionBar({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    selected, format, clearSelection, handleDelete, deleteBusy, handleExport, selectedExportableCount,
  } = list;

  return (
    <>
      {selected.size > 0 && (
        <div className="sticky bottom-4 z-30 transition-all duration-300 animate-in fade-in slide-in-from-bottom-3">
          <BentoCard className="flex flex-wrap items-center justify-between gap-3 border-primary/50 bg-card/95 backdrop-blur-md p-3 shadow-2xl ring-1 ring-primary/20 dark:border-amber-400/40 dark:ring-amber-400/20">
          <div className="flex items-center gap-2 text-xs font-medium">
            <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-primary-foreground text-[11px] font-bold">
              {selected.size}
            </span>
            <span>{t("threads.statsSelected", { count: selected.size })}</span>
            <span className="text-muted-foreground/60">·</span>
            <span className="text-muted-foreground font-mono text-[11px]">
              {t("threads.exportFormat")}: {format === "markdown" ? "Markdown (.md)" : "JSONL (.jsonl)"}
            </span>
          </div>

          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-7.5 gap-1 text-xs text-muted-foreground hover:text-foreground"
              onClick={clearSelection}
            >
              <Square className="h-3 w-3" />
              {t("threads.clearSelection")}
            </Button>
            {list.isCodex && <Button
              variant="outline"
              size="sm"
              className="h-7.5 gap-1 px-2.5 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive"
              onClick={() => void handleDelete([...selected])}
              disabled={deleteBusy}
            >
              {deleteBusy ? (
                <RotateCw className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Trash2 className="h-3.5 w-3.5" />
              )}
              {t("threads.deleteSelected", { count: selected.size })}
            </Button>}
            <RainbowButton
              className="h-7.5 rounded-[8px] px-3 text-xs font-medium"
              onClick={() => void handleExport([...selected])}
              disabled={selectedExportableCount === 0}
            >
              <Download className="h-3.5 w-3.5" />
              {t("threads.exportSelected", { count: selectedExportableCount })}
            </RainbowButton>
          </div>
        </BentoCard>
        </div>
      )}
    </>
  );
}
