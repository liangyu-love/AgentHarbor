import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Eye, Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { claudeRoles as roles, emptyClaudeRoles as emptyRoles, clientDefaults, clientModels, clientProtocols, exclusionOf } from "@/lib/client-config";
import { ClaudeModelConfig } from "./claude-model-config";
import { providerModelProtocol } from "@/lib/provider-models";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import type { ClaudeModels, ClientAccessMode, ClientConfigPreview, ClientConfigRequest, ClientStatus } from "@/types/clients";
import type { NewApiProvider } from "@/types/newapi";

const errorKey = (cause: unknown) => {
  const message = cause instanceof Error ? cause.message : String(cause);
  return /^(clients|newapi)\./.test(message) ? message : "clients.errors.storage";
};

export function ConfigDialog({ client, providers, onClose, onApplied }: {
  client: ClientStatus; providers: NewApiProvider[]; onClose: () => void; onApplied: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const isClaude = client.targetId === "claudeCode";
  const [request, setRequest] = useState<ClientConfigRequest>(() => {
    const saved = client.request;
    const accessMode = saved?.accessMode ?? (client.configured ? "router" : "native");
    const protocols = clientProtocols(client, accessMode);
    const available = providers.filter(provider => !exclusionOf(provider, protocols));
    const initial = available.find(provider => provider.id === client.providerId) ?? available[0];
    const models = initial ? clientModels(initial, protocols) : [];
    const restoredModels = isClaude && saved?.providerId === initial?.id && saved?.claudeModelIds
      ? models.filter(id => saved.claudeModelIds!.includes(id)) : models;
    const selectedModels = restoredModels.length ? restoredModels : models;
    const model = initial?.id === client.providerId && selectedModels.includes(client.model ?? "")
      ? client.model! : selectedModels[0] ?? "";
    const selectedIds = saved?.providerIds?.length ? saved.providerIds.filter(id => available.some(provider => provider.id === id))
      : !isClaude && client.configured && accessMode === "router" ? available.map(provider => provider.id) : [];
    const canRestore = saved?.providerId === initial?.id && saved?.model === model;
    return {
      targetId: client.targetId, providerId: initial?.id ?? "", model, accessMode,
      providerIds: Array.from(new Set([...selectedIds, ...(initial ? [initial.id] : [])])),
      ...clientDefaults(initial, model),
      ...(canRestore && saved ? { contextWindow: saved.contextWindow, maxOutputTokens: saved.maxOutputTokens } : {}),
      claudeModels: roles.reduce<ClaudeModels>((mapping, role) => {
        const value = canRestore ? saved?.claudeModels?.[role] : "";
        if (value && selectedModels.includes(value)) mapping[role] = value;
        return mapping;
      }, emptyRoles()),
      claudeModelIds: isClaude ? selectedModels : undefined,
      authField: saved?.authField ?? "authToken",
    };
  });
  const [preview, setPreview] = useState<ClientConfigPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const protocols = clientProtocols(client, request.accessMode);
  const compatible = providers.filter(provider => !exclusionOf(provider, protocols));
  const selected = compatible.filter(provider => request.providerIds.includes(provider.id));
  const provider = selected.find(provider => provider.id === request.providerId);
  const models = provider ? clientModels(provider, protocols) : [];
  const configuredModels = isClaude ? models.filter(id => request.claudeModelIds?.includes(id) ?? true) : models;
  const modelCompatible = configuredModels.includes(request.model);
  const selectedProtocol = provider ? providerModelProtocol(provider, request.model) : null;
  const modelLabel = (p: NewApiProvider, id: string) => p.models.find(model => model.id === id)?.displayName || id;
  const update = (patch: Partial<ClientConfigRequest>) => {
    if (busy) return;
    setRequest(current => ({ ...current, ...patch }));
    setPreview(null);
    setError(null);
  };
  const chooseDefault = (next: NewApiProvider | undefined, model?: string) => {
    const sameProvider = next?.id === request.providerId;
    const nextModels = next ? clientModels(next, protocols) : [];
    const available = isClaude && sameProvider ? nextModels.filter(id => request.claudeModelIds?.includes(id) ?? true) : nextModels;
    const id = model ?? (sameProvider && available.includes(request.model) ? request.model : available[0]) ?? "";
    return { providerId: next?.id ?? "", model: id, ...clientDefaults(next, id),
      claudeModelIds: isClaude ? available : undefined,
      claudeModels: next?.id === request.providerId ? request.claudeModels : emptyRoles() };
  };
  const selectClaudeModels = (next: string[]) => {
    const ids = models.filter(id => next.includes(id));
    if (ids.length === 0) return;
    const model = ids.includes(request.model) ? request.model : ids[0];
    update({ claudeModelIds: ids, model,
      ...(model !== request.model ? clientDefaults(provider, model) : {}),
      claudeModels: roles.reduce<ClaudeModels>((mapping, role) => {
        const value = request.claudeModels[role];
        if (ids.includes(value)) mapping[role] = value;
        return mapping;
      }, emptyRoles()) });
  };
  const changeMode = (accessMode: ClientAccessMode) => {
    const nextProtocols = clientProtocols(client, accessMode);
    const eligible = providers.filter(p => !exclusionOf(p, nextProtocols));
    const next = eligible.find(p => p.id === request.providerId) ?? eligible[0];
    const nextModels = next ? clientModels(next, nextProtocols) : [];
    const model = nextModels.includes(request.model) ? request.model : nextModels[0] ?? "";
    const ids = isClaude ? [] : request.providerIds.filter(id => eligible.some(p => p.id === id));
    update({ accessMode, providerId: next?.id ?? "", model,
      providerIds: Array.from(new Set([...ids, ...(next ? [next.id] : [])])),
      claudeModelIds: isClaude ? nextModels : undefined,
      claudeModels: emptyRoles(), ...clientDefaults(next, model) });
  };
  const toggleProvider = (next: NewApiProvider) => {
    if (isClaude) { update({ ...chooseDefault(next), providerIds: [next.id] }); return; }
    const ids = request.providerIds.includes(next.id)
      ? request.providerIds.filter(id => id !== next.id) : [...request.providerIds, next.id];
    const primary = compatible.find(p => p.id === request.providerId && ids.includes(p.id))
      ?? compatible.find(p => ids.includes(p.id));
    update({ providerIds: ids, ...(primary?.id !== request.providerId ? chooseDefault(primary) : {}) });
  };
  const run = async () => {
    if (busy) return;
    if (!modelCompatible || request.providerIds.some(id => !compatible.some(p => p.id === id))) {
      setError("clients.errors.provider"); return;
    }
    if (isClaude && request.claudeModelIds?.some(id => !models.includes(id))) {
      setError("clients.errors.modelSelection"); return;
    }
    if (isClaude && roles.some(role => request.claudeModels[role] && !configuredModels.includes(request.claudeModels[role]))) {
      setError("clients.errors.protocol"); return;
    }
    if (!Number.isSafeInteger(request.contextWindow) || request.contextWindow < 1024
      || !Number.isSafeInteger(request.maxOutputTokens) || request.maxOutputTokens < 1
      || request.maxOutputTokens > request.contextWindow) {
      setError("clients.errors.limits"); return;
    }
    setBusy(true);
    setError(null);
    try {
      if (preview) {
        const applied = await api.targets.applyConfig(request, preview.revision);
        await onApplied();
        toast({ title: t("clients.applied"), description: t(applied.warning ?? "clients.restartHint"), variant: applied.warning ? "destructive" : "success" });
        onClose();
      } else { setPreview(await api.targets.previewConfig(request)); }
    } catch (cause) {
      setError(errorKey(cause)); setPreview(null); await onApplied();
    } finally { setBusy(false); }
  };

  return (
    <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
      <DialogContent className={"flex max-h-[90vh] flex-col overflow-hidden " + (isClaude ? "sm:max-w-3xl" : "sm:max-w-2xl")}>
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("clients.configureTitle", { name: client.name })}</DialogTitle>
          <DialogDescription>{t(request.accessMode === "native" ? "clients.nativeHint" : "clients.localHint")}</DialogDescription>
        </DialogHeader>
        <div data-client-config-scroll className="-mx-1 min-h-0 space-y-4 overflow-y-auto px-1">
        <fieldset disabled={busy} className="min-w-0 space-y-4">
          <div className="flex w-fit gap-1 rounded-lg border bg-muted/40 p-1" aria-label={t("clients.accessMode")}>
            {(["native", "router"] as const).map(mode => <Button key={mode} type="button" size="sm"
              variant={request.accessMode === mode ? "secondary" : "ghost"} aria-pressed={request.accessMode === mode}
              disabled={busy} onClick={() => changeMode(mode)}>{t("clients.access" + (mode === "native" ? "Native" : "Router"))}</Button>)}
          </div>
          <div className="space-y-2">
            <p id="client-provider-label" className="text-sm font-medium">{t(isClaude ? "clients.provider" : "clients.providers")}</p>
            <div className="grid max-h-56 gap-2 overflow-y-auto sm:grid-cols-2" aria-labelledby="client-provider-label">
              {providers.map(p => {
                const reason = exclusionOf(p, protocols);
                const active = request.providerIds.includes(p.id) && !reason;
                return <button key={p.id} type="button" disabled={busy || !!reason} aria-pressed={active} onClick={() => toggleProvider(p)}
                  className={"flex min-w-0 items-center gap-3 rounded-lg border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45 "
                    + (active ? "border-primary/50 bg-primary/5" : "bg-card/60 hover:bg-muted/60")}>
                  <span className="min-w-0 flex-1"><span className="block truncate text-sm font-medium" title={p.name}>{p.name}</span>
                    <span className="mt-1 block text-xs text-muted-foreground">{reason ? t("clients.excluded." + reason, { count: 1 })
                      : t("clients.modelsCount", { count: clientModels(p, protocols).length })}</span></span>
                  <span className={"flex h-4 w-4 shrink-0 items-center justify-center border " + (isClaude ? "rounded-full " : "rounded ")
                    + (active ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
                    {active && <Check className="h-3 w-3" />}</span>
                </button>;
              })}
            </div>
            {compatible.length === 0 && <p className="text-xs text-muted-foreground">{t(isClaude && request.accessMode === "native" ? "clients.noNativeProviders" : "clients.noProviders")}</p>}
          </div>
          {isClaude && <ClaudeModelConfig key={request.providerId + ":" + request.accessMode}
            provider={provider} models={models} selected={configuredModels} mapping={request.claudeModels}
            defaultModel={request.model} disabled={busy} onSelectedChange={selectClaudeModels}
            onMappingChange={claudeModels => update({ claudeModels })} />}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2"><Label htmlFor="client-model">{t(isClaude ? "clients.startupModel" : "clients.model")}</Label>
              {selectedProtocol && <Badge variant="outline" className="text-[10px]">{t("newapi.protocol" + (selectedProtocol === "responses" ? "Responses" : selectedProtocol === "chatCompletions" ? "ChatCompletions" : "AnthropicMessages"))}</Badge>}
            </div>
            <Select disabled={busy || selected.length === 0} value={modelCompatible ? JSON.stringify([request.providerId, request.model]) : ""}
              onValueChange={value => { const [id, model] = JSON.parse(value) as [string, string]; update(chooseDefault(selected.find(p => p.id === id), model)); }}>
              <SelectTrigger id="client-model"><SelectValue placeholder={t("clients.selectModel")} /></SelectTrigger>
              <SelectContent>{selected.flatMap(p => (isClaude ? configuredModels : clientModels(p, protocols)).map(id => <SelectItem key={JSON.stringify([p.id, id])} value={JSON.stringify([p.id, id])}>
                {(!isClaude && selected.length > 1 ? p.name + " / " : "") + modelLabel(p, id)}
              </SelectItem>))}</SelectContent>
            </Select>
          </div>
          <details className="group rounded-lg border bg-muted/20">
            <summary className="flex cursor-pointer list-none items-center justify-between px-3 py-2.5 text-sm font-medium">
              {t("clients.advanced")}<ChevronDown className="h-4 w-4 text-muted-foreground transition-transform group-open:rotate-180" />
            </summary>
            <div className="space-y-4 border-t p-3">
              {isClaude ? <>
                <div className="space-y-1.5"><Label htmlFor="client-auth">{t("clients.authField")}</Label>
                  <Select disabled={busy} value={request.authField} onValueChange={value => update({ authField: value as ClientConfigRequest["authField"] })}>
                    <SelectTrigger id="client-auth"><SelectValue /></SelectTrigger><SelectContent>
                      <SelectItem value="authToken">ANTHROPIC_AUTH_TOKEN</SelectItem><SelectItem value="apiKey">ANTHROPIC_API_KEY</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </> : <div className="grid grid-cols-2 gap-3">
                <div className="space-y-1.5"><Label htmlFor="client-context">{t("clients.context")}</Label><Input id="client-context" type="number" min={1024} value={request.contextWindow} onChange={event => update({ contextWindow: Number(event.target.value) })} /></div>
                <div className="space-y-1.5"><Label htmlFor="client-output">{t("clients.output")}</Label><Input id="client-output" type="number" min={1} max={request.contextWindow} value={request.maxOutputTokens} onChange={event => update({ maxOutputTokens: Number(event.target.value) })} /></div>
              </div>}
              <p className="break-all font-mono text-xs text-muted-foreground">{client.path}</p>
            </div>
          </details>
        </fieldset>
        {preview && <div className="space-y-2"><p className="text-sm font-medium">{t("clients.previewTitle")}</p>
          <pre className="max-h-64 overflow-auto rounded-md bg-muted p-3 text-xs">{preview.content}</pre>
          {!!preview.removedFields?.length && <p className="break-all text-xs text-muted-foreground">{t("clients.removedFields", { fields: preview.removedFields.join(", ") })}</p>}
          <p className="text-xs text-muted-foreground">{t("clients.backupHint")}</p>
        </div>}
        {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
        </div>
        <DialogFooter className="shrink-0 gap-2 sm:gap-0"><Button variant="outline" onClick={onClose} disabled={busy}>{t("common.cancel")}</Button>
          <Button disabled={busy || !modelCompatible || selected.length === 0} onClick={() => void run()}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}{t(preview ? "clients.apply" : "clients.preview")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
