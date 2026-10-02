import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { RefreshCw, RotateCcw, Settings2, Terminal, Unlink } from "lucide-react";
import { api } from "@/lib/api";
import { clientModels, clientProtocols, conflictScopes, exclusionOf, type Exclusion } from "@/lib/client-config";
import { ConfigDialog } from "./client-config-dialog";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import type { ClientStatus } from "@/types/clients";
import type { ModelProtocol, NewApiProvider } from "@/types/newapi";
import { queryKeys } from "@/lib/query-keys";
import { errorKey } from "@/lib/invoke-error";

const CLIENT_KEY = queryKeys.clientConfigs;
const clientErrorKey = (error: unknown) =>
  errorKey(error, ["clients", "newapi"], "clients.errors.storage");

const summarizeExclusions = (
  providers: NewApiProvider[],
  protocols: ModelProtocol[],
  t: (key: string, options?: Record<string, unknown>) => string,
) => {
  const counts: Record<Exclusion, number> = { protocol: 0, disabled: 0, noApiKey: 0, noModel: 0 };
  for (const provider of providers) {
    const reason = exclusionOf(provider, protocols);
    if (reason) counts[reason] += 1;
  }
  const order: Exclusion[] = ["protocol", "disabled", "noApiKey", "noModel"];
  return order
    .filter(reason => counts[reason] > 0)
    .map(reason => t(`clients.excluded.${reason}`, { count: counts[reason] }));
};

export function ClientConfigs({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const clients = useQuery({ queryKey: CLIENT_KEY, queryFn: api.targets.configs });
  const [selected, setSelected] = useState<ClientStatus | null>(null);
  const [restoring, setRestoring] = useState<ClientStatus | null>(null);
  const [releasing, setReleasing] = useState<ClientStatus | null>(null);
  const [reapplying, setReapplying] = useState<{ client: ClientStatus; rebaseline: boolean } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: CLIENT_KEY }),
      queryClient.invalidateQueries({ queryKey: queryKeys.providers }),
    ]);
  };
  const restore = async () => {
    if (!restoring || busy) return;
    setBusy(true);
    setError(null);
    try {
      const restored = await api.targets.restoreConfig(restoring.targetId);
      toast({
        title: t("clients.restored"),
        description: restored.warnings?.length ? restored.warnings.map(key => t(key)).join(" ") : undefined,
        variant: restored.warnings?.length ? "destructive" : "success",
      });
      setRestoring(null);
    } catch (cause) { setError(clientErrorKey(cause)); }
    finally { setBusy(false); await refresh(); }
  };
  const reapply = async () => {
    if (!reapplying || busy) return;
    setBusy(true);
    setError(null);
    try {
      const applied = await api.targets.reapplyConfig(reapplying.client.targetId, reapplying.rebaseline);
      toast({
        title: t(reapplying.rebaseline ? "clients.reappliedRebaseline" : "clients.reapplied"),
        description: applied.warnings?.length
          ? applied.warnings.map(key => t(key)).join(" ")
          : t("clients.restartHint"),
        variant: applied.warnings?.length ? "destructive" : "success",
      });
      setReapplying(null);
    } catch (cause) { setError(clientErrorKey(cause)); }
    finally { setBusy(false); await refresh(); }
  };
  const release = async () => {
    if (!releasing || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.targets.releaseConfig(releasing.targetId);
      toast({ title: t("clients.released"), variant: "success" });
      setReleasing(null);
    } catch (cause) { setError(clientErrorKey(cause)); }
    finally { setBusy(false); await refresh(); }
  };

  return (
    <section className="space-y-3" aria-labelledby="client-configs-title">
      <div>
        <h2 id="client-configs-title" className="text-lg font-semibold">{t("clients.title")}</h2>

      </div>
      {clients.isPending && <p className="text-sm text-muted-foreground">{t("common.loading")}</p>}
      {(error || clients.error) && <p role="alert" className="text-sm text-destructive">{t(error ?? clientErrorKey(clients.error))}</p>}
      <div className="grid gap-3 lg:grid-cols-2">
        {clients.data?.map(client => {
          const provider = providers.find(p => p.id === client.providerId);
          const mode = client.request?.accessMode ?? (client.configured ? "router" : "native");
          const protocols = clientProtocols(client, mode);
          const modelIds = client.targetId === "claudeCode" ? client.request?.claudeModelIds : null;
          const modelName = provider?.models.find(model => model.id === client.model)?.displayName || client.model;
          const configuredSummary = client.targetId === "claudeCode"
            ? t(modelIds?.length ? "clients.configuredModelsSummary" : "clients.configuredDefaultSummary", {
              provider: provider?.name ?? client.providerId, count: modelIds ? new Set(modelIds).size : 0, model: modelName })
            : `${provider?.name ?? client.providerId} / ${client.model}`;
          const unavailable = client.configured && (!provider || !!exclusionOf(provider, protocols)
            || !clientModels(provider, protocols).includes(client.model ?? "")
            || client.request?.providerIds.some(id => !providers.some(p => p.id === id && !exclusionOf(p, protocols))));
          const eligible = providers.filter(p => !exclusionOf(p, protocols));
          const reasons = eligible.length === 0 ? summarizeExclusions(providers, protocols, t) : [];
          return (
            <div key={client.targetId} className="space-y-3 rounded-xl border bg-card/70 p-4">
              <div className="flex items-center gap-2">
                <Terminal className="h-4 w-4 text-muted-foreground" />
                <h3 className="font-semibold">{client.name}</h3>
                {client.configured && <span className="text-xs text-muted-foreground">{t(mode === "native" ? "clients.accessNative" : "clients.accessRouter")}</span>}
                <Badge variant="outline" className="ml-auto">{t(client.conflict ? "clients.conflict" : client.configured ? "clients.configured" : "clients.notConfigured")}</Badge>
              </div>
              <p className="break-all font-mono text-xs text-muted-foreground">{client.path}</p>
              <p className="break-words text-sm">{client.configured ? configuredSummary : t(`clients.${client.targetId}Hint`)}</p>
              {(client.conflict || client.error || unavailable) && <p role="alert" className="text-xs text-destructive">{t(client.error ?? (client.conflict ? "clients.errors.conflict" : "clients.errors.provider"))}</p>}
              {client.conflict && !!client.conflictFields?.length && <p role="status" className="text-xs text-muted-foreground">
                {t("clients.conflictScope", {
                  fields: conflictScopes(client.conflictFields).map(scope => t("clients.conflictScopes." + scope)).join(" · "),
                })}
              </p>}
              {reasons.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  {t("clients.noEligibleProviders")} {reasons.join(" · ")}
                </p>
              )}
              <div className="flex gap-2">
                <Button size="sm" variant="outline" disabled={busy || !!client.error || client.conflict} onClick={() => setSelected(client)}>
                  <Settings2 className="h-3.5 w-3.5" />{t(client.configured ? "clients.change" : "clients.configure")}
                </Button>
                {client.configured && <Button size="sm" variant="ghost" disabled={busy || client.conflict || !!client.error} onClick={() => setRestoring(client)}>
                  <RotateCcw className="h-3.5 w-3.5" />{t("clients.restore")}
                </Button>}
                {client.configured && !client.conflict && !client.error && !unavailable && <Button size="sm" variant="ghost" disabled={busy} onClick={() => setReapplying({ client, rebaseline: false })}>
                  <RefreshCw className="h-3.5 w-3.5" />{t("clients.reapply")}
                </Button>}
                {client.conflict && <Button size="sm" variant="ghost" disabled={busy} onClick={() => setReapplying({ client, rebaseline: true })}>
                  <RefreshCw className="h-3.5 w-3.5" />{t("clients.rebaseline")}
                </Button>}
                {(client.conflict || (client.error && client.error !== "clients.errors.invalidConfig")) && (
                  <Button size="sm" variant="ghost" disabled={busy} onClick={() => setReleasing(client)}>
                    <Unlink className="h-3.5 w-3.5" />{t("clients.release")}
                  </Button>
                )}
              </div>
            </div>
          );
        })}
      </div>
      {selected && <ConfigDialog client={selected} providers={providers} onClose={() => setSelected(null)} onApplied={refresh} />}
      <AlertDialog open={!!restoring} onOpenChange={open => { if (!open && !busy) setRestoring(null); }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("clients.restoreTitle", { name: restoring?.name })}</AlertDialogTitle>
            <AlertDialogDescription>{t("clients.restoreDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={event => { event.preventDefault(); void restore(); }}>{t("clients.restore")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={!!reapplying} onOpenChange={open => { if (!open && !busy) setReapplying(null); }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t(reapplying?.rebaseline ? "clients.rebaselineTitle" : "clients.reapplyTitle", { name: reapplying?.client.name })}</AlertDialogTitle>
            <AlertDialogDescription>{t(reapplying?.rebaseline ? "clients.rebaselineDescription" : "clients.reapplyDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={event => { event.preventDefault(); void reapply(); }}>
              {t(reapplying?.rebaseline ? "clients.rebaseline" : "clients.reapply")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={!!releasing} onOpenChange={open => { if (!open && !busy) setReleasing(null); }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("clients.releaseTitle", { name: releasing?.name })}</AlertDialogTitle>
            <AlertDialogDescription>{t("clients.releaseDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction disabled={busy} onClick={event => { event.preventDefault(); void release(); }}>{t("clients.release")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
