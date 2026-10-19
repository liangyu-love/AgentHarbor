import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";

export const DEFAULT_ROUTER_PORT = 43127;
/** Below it ports are privileged; the backend refuses them too. */
const MIN_PORT = 1024;

/**
 * The local router's port, shown as the address clients use. Moving it
 * withdraws every routed client, moves the listener and applies the clients
 * again on the new port; a port that cannot be taken changes nothing.
 * `replacing`: the current port cannot be used, so the field starts empty.
 */
export function RouterPortEditor({ replacing = false }: { replacing?: boolean }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState });
  const current = state.data?.router.port;
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (current !== undefined && !replacing) setDraft(String(current));
  }, [current, replacing]);

  const value = Number(draft);
  const valid = /^\d{1,5}$/.test(draft) && value >= MIN_PORT && value <= 65535;
  const invalid = draft !== "" && !valid;

  const apply = async (port: number) => {
    setBusy(true);
    try {
      await api.providers.setPort(port);
      toast({ title: t("settings.routerPort.changed", { port }), description: t("settings.routerPort.changedDesc"), variant: "success" });
    } catch (error) {
      toast({ title: t("settings.routerPort.failed"), description: formatInvokeError(error), variant: "destructive" });
      setDraft(replacing || current === undefined ? "" : String(current));
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
      <label
        className={cn(
          "flex h-8 items-center rounded-md border bg-background/80 pl-2.5 font-mono text-xs shadow-xs transition-[border-color,box-shadow]",
          "focus-within:border-ring focus-within:ring-[3px] focus-within:ring-ring/50",
          invalid ? "border-destructive/60" : "border-input",
          (busy || current === undefined) && "opacity-60",
        )}
        title={invalid ? t("newapi.errors.portInvalid") : undefined}
      >
        <span className="select-none text-muted-foreground" aria-hidden>127.0.0.1:</span>
        <input
          aria-label={t("settings.routerPort.label")}
          aria-invalid={invalid}
          inputMode="numeric"
          maxLength={5}
          placeholder={replacing ? t("settings.routerPort.placeholder") : undefined}
          value={draft}
          onChange={event => setDraft(event.target.value.replace(/\D/g, ""))}
          disabled={busy || current === undefined}
          className="h-full w-[3.75rem] bg-transparent pr-2 pl-0.5 text-foreground tabular-nums outline-none placeholder:text-muted-foreground/60"
        />
      </label>
      <Button type="submit" size="sm" variant="outline" className="h-8 gap-1.5" disabled={busy || !valid || value === current}>
        {busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
        {t("settings.routerPort.apply")}
      </Button>
      {!replacing && current !== undefined && current !== DEFAULT_ROUTER_PORT && (
        <Button type="button" size="sm" variant="ghost" className="h-8 text-muted-foreground" disabled={busy} onClick={() => void apply(DEFAULT_ROUTER_PORT)}>
          {t("settings.routerPort.reset")}
        </Button>
      )}
    </form>
  );
}
