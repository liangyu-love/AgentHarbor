import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Terminal } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { Input } from "@/components/ui/input";
import {
  Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import type { ApiProxyConfigPayload, TerminalProxyProbe } from "@/types";

const PREFIX = "settings.terminalProxy.";

/** This owns only the terminal environment; the application's API proxy is separate. */
export function TerminalProxySettings({ currentProxy }: { currentProxy?: ApiProxyConfigPayload }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const inputId = useId();
  const query = useQuery({ queryKey: queryKeys.terminalProxy, queryFn: api.terminalProxy.load, retry: false });
  const status = query.data;
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [noProxy, setNoProxy] = useState("");
  const [probes, setProbes] = useState<TerminalProxyProbe[] | null>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  // Ignore responses after a draft edit, close/reopen or unmount.
  const draftRevision = useRef(0);
  const saveLock = useRef(false);
  useEffect(() => () => { draftRevision.current += 1; }, []);
  const clearProbe = () => {
    draftRevision.current += 1;
    setProbes(null);
    setTestError(null);
    setTesting(false);
  };
  const changeOpen = (next: boolean) => {
    if (saveLock.current) return;
    clearProbe();
    setFormError(null);
    if (next) {
      setUrl(status?.url ?? "");
      setNoProxy(status?.noProxy ?? "localhost,127.0.0.1,::1");
    }
    setOpen(next);
  };
  const save = useMutation({
    mutationFn: (enabled: boolean) => api.terminalProxy.set(enabled, url, noProxy),
    onSuccess: result => {
      queryClient.setQueryData(queryKeys.terminalProxy, result);
      clearProbe();
      setOpen(false);
      const detail = !result.notificationSent ? t(PREFIX + "notificationFailed")
        : result.preservedVariables.length ? t(PREFIX + "preserved", { variables: result.preservedVariables.join(", ") })
        : result.enabled ? t(PREFIX + "restartHint") : t(PREFIX + "restoredDesc");
      toast({ title: t(PREFIX + (result.enabled ? "saved" : "restored")), description: detail, variant: "success" });
    },
    onError: error => {
      const message = formatInvokeError(error);
      setFormError(message);
      toast({ title: t("common.error"), description: message, variant: "destructive" });
      // A partial operation or an external editor may have changed the lease.
      void queryClient.invalidateQueries({ queryKey: queryKeys.terminalProxy });
    },
    onSettled: () => { saveLock.current = false; },
  });
  const submit = (enabled: boolean) => {
    if (saveLock.current) return;
    saveLock.current = true;
    clearProbe();
    setFormError(null);
    save.mutate(enabled);
  };
  const launch = useMutation({
    mutationFn: api.terminalProxy.openTerminal,
    onError: error => {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
      void queryClient.invalidateQueries({ queryKey: queryKeys.terminalProxy });
    },
  });
  const test = async () => {
    if (testing || !url.trim() || saveLock.current) return;
    const revision = ++draftRevision.current;
    setTesting(true);
    setTestError(null);
    setProbes(null);
    try {
      const result = await api.terminalProxy.test(url.trim());
      if (draftRevision.current === revision) setProbes(result);
    } catch (error) {
      if (draftRevision.current === revision) setTestError(formatInvokeError(error));
    } finally {
      if (draftRevision.current === revision) setTesting(false);
    }
  };
  const blocked = Boolean(status?.recoveryRequired || status?.conflicts.length);
  const stateKey = query.isPending ? "loading" : query.isError ? "loadFailed"
    : !status?.supported ? "unsupported" : status.recoveryRequired ? "recoveryRequired"
    : status.conflicts.length ? "changed" : status.applied ? "enabled" : "disabled";
  const reuseUrl = currentProxy?.mode === "manual" && /^https?:\/\//i.test(currentProxy.url ?? "")
    ? currentProxy.url : null;

  return (
    <>
      <div className="space-y-3 px-5 py-4" data-terminal-proxy>
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span className="text-[13px] font-medium">{t(PREFIX + "title")}</span>
              <Badge variant="secondary" className={cn("text-[11px] font-normal", blocked && "text-amber-700 dark:text-amber-300")} data-terminal-proxy-state={stateKey}>
                {t(PREFIX + stateKey)}
              </Badge>
            </div>
            <p className="text-xs leading-relaxed text-muted-foreground">{t(PREFIX + "description")}</p>
            {status?.enabled && status.url && <p className="break-all font-mono text-[11px] text-foreground/70">{status.url}</p>}
          </div>
          {query.isError ? <Button variant="outline" size="sm" onClick={() => void query.refetch()}>{t(PREFIX + "retry")}</Button>
            : <Button variant="outline" size="sm" disabled={!status?.supported || save.isPending} onClick={() => changeOpen(true)} aria-label={t(PREFIX + "edit")}>
              {t("common.edit")}
            </Button>}
        </div>
        {query.isError && <p className="text-xs text-destructive" role="alert">{formatInvokeError(query.error)}</p>}
        {status?.supported && status.enabled && (
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" disabled={!status.applied || blocked || launch.isPending || save.isPending} onClick={() => launch.mutate()} aria-busy={launch.isPending}>
              <ButtonBusyContent busy={launch.isPending} idleIcon={<Terminal className="h-3.5 w-3.5" />} idleLabel={t(PREFIX + "openTerminal")} busyLabel={t(PREFIX + "opening")} />
            </Button>
            <Button variant="ghost" size="sm" disabled={save.isPending || launch.isPending} onClick={() => submit(false)} aria-busy={save.isPending && save.variables === false}>
              <ButtonBusyContent busy={save.isPending && save.variables === false} idleLabel={t(PREFIX + "restore")} busyLabel={t(PREFIX + "restoring")} />
            </Button>
          </div>
        )}
        {blocked && <p className="text-xs leading-relaxed text-amber-700 dark:text-amber-300">{t(PREFIX + (status?.recoveryRequired ? "recoveryHint" : "changedHint"))}</p>}
      </div>
      <Dialog open={open} onOpenChange={changeOpen}>
        <DialogContent className="max-w-md" onEscapeKeyDown={event => { if (saveLock.current) event.preventDefault(); }} onPointerDownOutside={event => { if (saveLock.current) event.preventDefault(); }}>
          <DialogHeader>
            <DialogTitle>{t(PREFIX + "title")}</DialogTitle>
            <DialogDescription>{t(PREFIX + "dialogDescription")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-1">
            <div className="space-y-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <label className="text-sm font-medium" htmlFor={inputId + "-url"}>{t(PREFIX + "url")}</label>
                {reuseUrl && <button type="button" className="rounded text-xs text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" disabled={save.isPending} onClick={() => { clearProbe(); setFormError(null); setUrl(reuseUrl); }}>{t(PREFIX + "reuseApiProxy")}</button>}
              </div>
              <Input id={inputId + "-url"} value={url} onChange={event => { clearProbe(); setFormError(null); setUrl(event.target.value); }} disabled={save.isPending} placeholder="http://127.0.0.1:7890" autoCapitalize="off" autoCorrect="off" spellCheck={false} className="text-sm" />
              <p className="text-xs leading-relaxed text-muted-foreground">{t(PREFIX + "httpOnly")}</p>
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor={inputId + "-bypass"}>{t(PREFIX + "bypass")}</label>
              <Input id={inputId + "-bypass"} value={noProxy} onChange={event => { clearProbe(); setFormError(null); setNoProxy(event.target.value); }} disabled={save.isPending} autoCapitalize="off" autoCorrect="off" spellCheck={false} className="text-sm" />
              <p className="text-xs leading-relaxed text-muted-foreground">{t(PREFIX + "bypassHint")}</p>
            </div>
            {(probes || testError) && <div className="space-y-2 border-t border-border pt-3" role="status" aria-live="polite" data-terminal-proxy-probes>
              {probes?.map(probe => <div key={probe.service} className="flex items-center justify-between gap-3 text-xs">
                <span className="font-medium">{probe.service}</span>
                <span className={cn("inline-flex items-center gap-1.5", probe.reachable ? "text-emerald-700 dark:text-emerald-300" : "text-destructive")}>
                  {probe.reachable && <Check className="h-3.5 w-3.5" />}
                  {t(PREFIX + (probe.reachable ? "reachable" : "unreachable"))}{probe.statusCode !== null && ` · HTTP ${probe.statusCode}`}
                </span>
              </div>)}
              {testError && <p className="text-xs text-destructive">{testError}</p>}
              {probes && <p className="text-xs leading-relaxed text-muted-foreground">{t(PREFIX + "testHint")}</p>}
            </div>}
            {formError && <p className="text-xs text-destructive" role="alert">{formError}</p>}
            {blocked && <p className="text-xs leading-relaxed text-amber-700 dark:text-amber-300">{t(PREFIX + (status?.recoveryRequired ? "recoveryHint" : "changedHint"))}</p>}
            <p className="text-xs leading-relaxed text-muted-foreground">{t(PREFIX + "restartHint")}</p>
          </div>
          <DialogFooter className="gap-2 sm:space-x-0">
            <Button variant="outline" onClick={() => changeOpen(false)} disabled={save.isPending}>{t("common.cancel")}</Button>
            <Button variant="outline" onClick={() => void test()} disabled={!url.trim() || testing || save.isPending} aria-busy={testing}>
              <ButtonBusyContent busy={testing} idleLabel={t(PREFIX + "test")} busyLabel={t(PREFIX + "testing")} />
            </Button>
            <Button onClick={() => submit(!blocked)} disabled={save.isPending || launch.isPending || (!blocked && !url.trim()) || !status?.supported} aria-busy={save.isPending}>
              <ButtonBusyContent busy={save.isPending} idleLabel={t(PREFIX + (blocked ? "restore" : "save"))} busyLabel={t(PREFIX + (blocked ? "restoring" : "saving"))} />
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
