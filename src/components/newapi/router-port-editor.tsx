import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export const DEFAULT_ROUTER_PORT = 43127;
/** Below it ports are privileged; the backend refuses them too. */
const MIN_PORT = 1024;

/**
 * The local router's port. Moving it withdraws every routed client, moves the
 * listener and applies the clients again on the new port; a port that cannot
 * be taken changes nothing.
 */
export function RouterPortEditor() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState });
  const current = state.data?.router.port;
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (current !== undefined) setDraft(String(current));
  }, [current]);

  const value = Number(draft);
  const valid = /^\d{1,5}$/.test(draft) && value >= MIN_PORT && value <= 65535;

  const apply = async (port: number) => {
    setBusy(true);
    try {
      await api.providers.setPort(port);
      toast({ title: t("settings.routerPort.changed", { port }), description: t("settings.routerPort.changedDesc"), variant: "success" });
    } catch (error) {
      toast({ title: t("settings.routerPort.failed"), description: formatInvokeError(error), variant: "destructive" });
      if (current !== undefined) setDraft(String(current));
    } finally {
      setBusy(false);
      await Promise.all(
        [queryKeys.providers, queryKeys.routerPortOwner, queryKeys.cursorStatus, queryKeys.clientConfigs, queryKeys.gateway]
          .map(queryKey => queryClient.invalidateQueries({ queryKey })),
      );
    }
  };

  return (
    <form
      className="flex items-center gap-2"
      onSubmit={event => {
        event.preventDefault();
        if (valid && value !== current) void apply(value);
      }}
    >
      <Input
        aria-label={t("settings.routerPort.label")}
        inputMode="numeric"
        value={draft}
        onChange={event => setDraft(event.target.value.trim())}
        disabled={busy || current === undefined}
        aria-invalid={draft !== "" && !valid}
        className="h-8 w-24 font-mono text-xs"
      />
      <Button type="submit" size="sm" variant="outline" className="h-8 gap-1.5" disabled={busy || !valid || value === current}>
        {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
        {t("settings.routerPort.apply")}
      </Button>
      {current !== undefined && current !== DEFAULT_ROUTER_PORT && (
        <Button type="button" size="sm" variant="ghost" className="h-8" disabled={busy} onClick={() => void apply(DEFAULT_ROUTER_PORT)}>
          {t("settings.routerPort.reset")}
        </Button>
      )}
    </form>
  );
}
