import { useTranslation } from "react-i18next";
import { MessagesSquare, RotateCw } from "lucide-react";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import type { ConversationSource } from "@/types/threads";
import type { ThreadList } from "@/components/threads/use-thread-list";

export function ThreadsHeader({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    source,
    setSource,
    sourceBusy,
    refreshAction,
    handleRefresh,
  } = list;

  return (
    <div className="flex flex-col gap-2.5 sm:flex-row sm:flex-wrap sm:items-center sm:justify-between">
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

      <div className="flex flex-wrap items-center gap-2 sm:justify-end">
        {/* Client source selector */}
        <div data-thread-source-controls><SegmentedOptions
          items={[
            { value: "codex", label: "Codex", disabled: sourceBusy },
            { value: "claudeCode", label: "Claude Code", disabled: sourceBusy },
          ]}
          value={source}
          onChange={(value) => setSource(value as ConversationSource)}
        /></div>

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
