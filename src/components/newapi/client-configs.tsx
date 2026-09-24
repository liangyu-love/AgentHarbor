import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Eye, Loader2, RotateCcw, Settings2, Terminal } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import type { ClientConfigPreview, ClientConfigRequest, ClientStatus } from "@/types/clients";
import type { ModelProtocol, NewApiProvider } from "@/types/newapi";

const CLIENT_KEY = ["client-configs"] as const;
const errorKey = (error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  return /^(clients|newapi)\./.test(message) ? message : "clients.errors.storage";
};

/// Why a relay cannot serve this client. The client's accepted protocols are the
/// hard gate: a Responses-only relay can never back Claude Code. Reporting the
/// distinct causes is what keeps an empty picker from looking like a broken button.
type Exclusion = "protocol" | "disabled" | "noApiKey" | "noModel";

const exclusionOf = (provider: NewApiProvider, protocols: ModelProtocol[]): Exclusion | null => {
  if (!protocols.includes(provider.protocol)) return "protocol";
  if (!provider.enabled) return "disabled";
  if (!provider.hasApiKey) return "noApiKey";
  if (!provider.model) return "noModel";
  return null;
};

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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: CLIENT_KEY }),
      queryClient.invalidateQueries({ queryKey: ["newapi-state"] }),
    ]);
  };
  const restore = async () => {
    if (!restoring || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.targets.restoreConfig(restoring.targetId);
      toast({ title: t("clients.restored"), variant: "success" });
      setRestoring(null);
    } catch (cause) { setError(errorKey(cause)); }
    finally { setBusy(false); await refresh(); }
  };

  return (
    <section className="space-y-3" aria-labelledby="client-configs-title">
      <div>
        <h2 id="client-configs-title" className="text-lg font-semibold">{t("clients.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("clients.description")}</p>
      </div>
      {clients.isPending && <p className="text-sm text-muted-foreground">{t("common.loading")}</p>}
      {(error || clients.error) && <p role="alert" className="text-sm text-destructive">{t(error ?? errorKey(clients.error))}</p>}
      <div className="grid gap-3 lg:grid-cols-2">
        {clients.data?.map(client => {
          const provider = providers.find(p => p.id === client.providerId);
          const unavailable = client.configured && (!provider?.enabled || !provider.hasApiKey || !client.protocols.includes(provider.protocol));
          const eligible = providers.filter(p => !exclusionOf(p, client.protocols));
          const reasons = eligible.length === 0 ? summarizeExclusions(providers, client.protocols, t) : [];
          return (
            <div key={client.targetId} className="space-y-3 rounded-xl border bg-card/70 p-4">
              <div className="flex items-center gap-2">
                <Terminal className="h-4 w-4 text-muted-foreground" />
                <h3 className="font-semibold">{client.name}</h3>
                <Badge variant="outline" className="ml-auto">{t(client.conflict ? "clients.conflict" : client.configured ? "clients.configured" : "clients.notConfigured")}</Badge>
              </div>
              <p className="break-all font-mono text-xs text-muted-foreground">{client.path}</p>
              <p className="text-sm">{client.configured ? `${provider?.name ?? client.providerId} / ${client.model}` : t(`clients.${client.targetId}Hint`)}</p>
              {(client.conflict || client.error || unavailable) && <p role="alert" className="text-xs text-destructive">{t(client.error ?? (client.conflict ? "clients.errors.conflict" : "clients.errors.provider"))}</p>}
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
    </section>
  );
}

function ConfigDialog({ client, providers, onClose, onApplied }: {
  client: ClientStatus; providers: NewApiProvider[]; onClose: () => void; onApplied: () => Promise<void>;
}) {
  const { t } = useTranslation();
  // Same gate as the card used to build its summary, so the dialog can never end
  // up narrower than the reason the card reported.
  const compatible = providers.filter(p => !exclusionOf(p, client.protocols));
  const initial = compatible.find(p => p.id === client.providerId) ?? compatible[0];
  const defaults = (provider: NewApiProvider | undefined, model: string) => {
    const metadata = provider?.models.find(m => m.id === model);
    const contextWindow = metadata?.contextWindow ?? provider?.contextWindow ?? 1_048_576;
    return { contextWindow, maxOutputTokens: Math.min(metadata?.maxOutputTokens ?? 8192, contextWindow) };
  };
  const initialModel = initial?.id === client.providerId ? client.model ?? initial.model : initial?.model ?? "";
  const [request, setRequest] = useState<ClientConfigRequest>({
    targetId: client.targetId, providerId: initial?.id ?? "", model: initialModel, ...defaults(initial, initialModel),
  });
  const [preview, setPreview] = useState<ClientConfigPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const provider = compatible.find(p => p.id === request.providerId);
  const update = (patch: Partial<ClientConfigRequest>) => {
    setRequest(current => ({ ...current, ...patch }));
    setPreview(null);
    setError(null);
  };
  const run = async (apply: boolean) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (apply && preview) {
        await api.targets.applyConfig(request, preview.revision);
        await onApplied();
        toast({ title: t("clients.applied"), description: t("clients.restartHint"), variant: "success" });
        onClose();
      } else {
        setPreview(await api.targets.previewConfig(request));
      }
    } catch (cause) {
      setError(errorKey(cause));
      setPreview(null);
      await onApplied();
    } finally { setBusy(false); }
  };

  return (
    <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t("clients.configureTitle", { name: client.name })}</DialogTitle>
          <DialogDescription>{t(`clients.${client.targetId}Hint`)}</DialogDescription>
        </DialogHeader>
        <fieldset disabled={busy} className="space-y-4">
          <div className="space-y-1.5">
            <Label htmlFor="client-provider">{t("clients.provider")}</Label>
            <select id="client-provider" value={request.providerId} onChange={event => {
              const next = compatible.find(p => p.id === event.target.value);
              update({ providerId: next?.id ?? "", model: next?.model ?? "", ...defaults(next, next?.model ?? "") });
            }} className="flex h-10 w-full rounded-md border border-input bg-background px-3 text-sm">
              {compatible.length === 0 && <option value="">{t("clients.noProviders")}</option>}
              {compatible.map(p => <option key={p.id} value={p.id}>{p.name} · {t(`newapi.protocol${p.protocol === "responses" ? "Responses" : p.protocol === "chatCompletions" ? "ChatCompletions" : "AnthropicMessages"}`)}</option>)}
            </select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="client-model">{t("clients.model")}</Label>
            <Input id="client-model" list="client-model-list" value={request.model} onChange={event => update({ model: event.target.value, ...defaults(provider, event.target.value) })} />
            <datalist id="client-model-list">{Array.from(new Set([provider?.model, ...(provider?.models.map(m => m.id) ?? [])])).filter(Boolean).map(id => <option key={id} value={id} />)}</datalist>
          </div>
          {client.targetId === "opencode" && <div className="space-y-2">
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5"><Label htmlFor="client-context">{t("clients.context")}</Label><Input id="client-context" type="number" min={1024} value={request.contextWindow} onChange={event => update({ contextWindow: Number(event.target.value) })} /></div>
              <div className="space-y-1.5"><Label htmlFor="client-output">{t("clients.output")}</Label><Input id="client-output" type="number" min={1} max={request.contextWindow} value={request.maxOutputTokens} onChange={event => update({ maxOutputTokens: Number(event.target.value) })} /></div>
            </div>
            <p className="text-xs text-muted-foreground">{t("clients.limitsHint")}</p>
          </div>}
        </fieldset>
        <p className="break-all font-mono text-xs text-muted-foreground">{preview?.path ?? client.path}</p>
        <p className="text-xs leading-relaxed text-muted-foreground">{t("clients.localHint")}</p>
        {preview && <div className="space-y-2">
          <p className="text-sm font-medium">{t("clients.previewTitle")}</p>
          <pre className="max-h-64 overflow-auto rounded-md bg-muted p-3 text-xs">{preview.content}</pre>
          <p className="text-xs text-muted-foreground">{t("clients.backupHint")}</p>
        </div>}
        {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>{t("common.cancel")}</Button>
          <Button disabled={busy || !request.providerId || !request.model.trim()} onClick={() => void run(!!preview)}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}{t(preview ? "clients.apply" : "clients.preview")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
