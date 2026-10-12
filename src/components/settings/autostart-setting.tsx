import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { toast } from "@/hooks/use-toast";
import { Switch } from "@/components/ui/switch";

const PREFIX = "settings.autostart.";

/** Start at sign-in, straight into the tray. */
export function AutostartSetting() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const query = useQuery({ queryKey: queryKeys.autostart, queryFn: api.autostart.status, retry: false });
  const save = useMutation({
    mutationFn: api.autostart.set,
    onSuccess: status => queryClient.setQueryData(queryKeys.autostart, status),
    onError: error => toast({ title: t(PREFIX + "failed"), description: formatInvokeError(error), variant: "destructive" }),
  });
  const status = query.data;
  if (!status?.supported) return null;
  const on = status.enabled && !status.blocked;
  return (
    <div className="flex items-center justify-between px-5 py-4" data-autostart={status.blocked ? "blocked" : on ? "on" : "off"}>
      <div className="min-w-0 pr-4">
        <span className="text-[13px] font-medium">{t(PREFIX + "title")}</span>
        <p className="mt-0.5 text-xs text-muted-foreground">{t(PREFIX + "description")}</p>
        {status.blocked && <p className="mt-1 text-xs text-amber-700 dark:text-amber-300" role="status">{t(PREFIX + "blocked")}</p>}
      </div>
      <Switch checked={on} disabled={save.isPending} aria-label={t(PREFIX + "title")}
        onCheckedChange={enabled => save.mutate(enabled)} />
    </div>
  );
}
