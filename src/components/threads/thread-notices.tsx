import { useTranslation } from "react-i18next";
import { AlertTriangle, Clock3 } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ThreadList } from "@/components/threads/use-thread-list";

// Scan warnings, the pending deletion queue and missing rollout files.
export function ThreadNotices({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    scanWarnings, pendingDeletionKeys, pendingDeletionRecoveryNeeded, cancelPendingBusy, cancelPendingDeletions, missingRolloutCount,
  } = list;

  return (
    <>
      {scanWarnings.length > 0 && (
        <div role="status" className="rounded-xl border border-amber-500/30 bg-amber-500/5 p-3 text-sm">
          <details>
            <summary className="cursor-pointer font-medium">
              {t("threads.scanWarnings", { count: scanWarnings.length })}
            </summary>
            <ul className="mt-2 max-h-40 space-y-1 overflow-y-auto break-words text-xs text-muted-foreground">
              {scanWarnings.map((warning, index) => (
                <li key={index}>{warning.message}</li>
              ))}
            </ul>
          </details>
        </div>
      )}

      {(pendingDeletionKeys.size > 0 || pendingDeletionRecoveryNeeded) && (
        <div role="status" className="flex flex-wrap items-center gap-3 rounded-xl border border-amber-500/30 bg-amber-500/5 px-3 py-2.5 text-sm text-foreground motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-200">
          <Clock3 className="h-4 w-4 shrink-0 text-amber-700 dark:text-amber-400" aria-hidden="true" />
          <div className="min-w-0 flex-1">
            <p className="font-medium">{pendingDeletionRecoveryNeeded
              ? t("threads.deleteQueueRecoveryNeeded") : t("threads.deletePending", { count: pendingDeletionKeys.size })}</p>
            <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{pendingDeletionRecoveryNeeded
              ? t("threads.deleteQueueRecoveryDetail") : t("threads.deletePendingDetail")}</p>
          </div>
          <Button type="button" size="sm" variant="outline" disabled={cancelPendingBusy} onClick={() => void cancelPendingDeletions()}>
            {t(pendingDeletionRecoveryNeeded ? "threads.clearDeletionQueue" : "threads.cancelPendingDeletion")}
          </Button>
        </div>
      )}

      {missingRolloutCount > 0 && (
        <div role="status" className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-sm">
          <AlertTriangle className="h-4 w-4 shrink-0 text-destructive" />
          <p className="font-medium text-destructive">
            {t("threads.rolloutMissingSummary", { count: missingRolloutCount })}
          </p>
        </div>
      )}
    </>
  );
}
