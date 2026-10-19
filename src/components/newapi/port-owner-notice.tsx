import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2, Unplug } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { errorKey } from "@/lib/invoke-error";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ui/confirm-dialog";
import { RouterPortEditor } from "./router-port-editor";

// While the router is down, says what keeps it off its port: a program it can
// end, or the system's reservation. Either way another port can be taken here;
// once the router holds a port, it and Cursor start again.
export function PortOwnerNotice({ routerRunning }: { routerRunning: boolean | undefined }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [confirming, setConfirming] = useState(false);
  const [ending, setEnding] = useState(false);
  const owner = useQuery({
    queryKey: queryKeys.routerPortOwner,
    queryFn: api.providers.portOwner,
    enabled: routerRunning === false,
    refetchInterval: 15_000,
  });
  const holder = routerRunning === false ? owner.data : null;
  if (!holder) return null;

  const end = async (pid: number) => {
    setEnding(true);
    try {
      await api.providers.endPortOwner(pid);
      toast({ title: t("newapi.portOwner.ended"), variant: "success" });
    } catch (error) {
      toast({ title: t(errorKey(error, ["newapi", "cursor"], "newapi.errors.portBusy")), variant: "destructive" });
    } finally {
      setEnding(false);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: queryKeys.providers }),
        queryClient.invalidateQueries({ queryKey: queryKeys.cursorStatus }),
        queryClient.invalidateQueries({ queryKey: queryKeys.clientConfigs }),
      ]);
    }
  };

  const names = { port: holder.port, pid: holder.pid, name: holder.name };
  const pid = holder.pid ?? null;
  const kind = pid !== null ? "" : holder.reserved ? "reserved" : "unknown";
  const title = kind ? t(`newapi.portOwner.${kind}Title`, names) : t("newapi.portOwner.title", names);
  const hint = kind ? t(`newapi.portOwner.${kind}Hint`) : t("newapi.portOwner.hint");
  return (
    <div role="alert" data-port-owner
      className="flex flex-col gap-3 rounded-xl border border-amber-500/35 bg-amber-500/[0.07] p-4 sm:flex-row sm:items-start">
      <Unplug aria-hidden className="mt-0.5 hidden h-5 w-5 shrink-0 text-amber-600 sm:block dark:text-amber-400" />
      <div className="min-w-0 flex-1 space-y-0.5">
        <p className="text-sm font-semibold text-amber-900 dark:text-amber-100">{title}</p>
        <p className="text-xs leading-relaxed text-amber-800/90 dark:text-amber-200/90">{hint}</p>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2 pt-2.5 text-xs font-medium text-amber-900 dark:text-amber-100">
          <span>{t(pid !== null ? "newapi.portOwner.changeInstead" : "newapi.portOwner.change")}</span>
          <RouterPortEditor replacing />
        </div>
      </div>
      {pid !== null && (
        <Button type="button" size="sm" variant="outline" disabled={ending} onClick={() => setConfirming(true)}
          className="h-8 shrink-0 gap-1.5 border-amber-500/50 bg-background/70 text-xs text-amber-800 hover:bg-amber-500/10 sm:mt-0.5 dark:text-amber-200">
          {ending && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          {t("newapi.portOwner.end")}
        </Button>
      )}
      <ConfirmDialog
        request={confirming && pid !== null ? {
          title: t("newapi.portOwner.confirmTitle", names),
          description: t("newapi.portOwner.confirmDesc", names),
          confirmLabel: t("newapi.portOwner.end"),
          destructive: true,
          onConfirm: () => void end(pid),
        } : null}
        onClose={() => setConfirming(false)}
      />
    </div>
  );
}
