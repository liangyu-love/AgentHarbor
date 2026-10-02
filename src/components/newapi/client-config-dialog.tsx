import { useState } from "react";
import { useTranslation } from "react-i18next";
import { BookmarkPlus, Check, ChevronDown, Eye, Loader2, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { claudeRoles as roles, emptyClaudeRoles as emptyRoles, claudeOneMDefaults, clientDefaults, clientModels, clientProtocols, exclusionOf, knownSmallContext, effectiveContext } from "@/lib/client-config";
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
import { storageKeys } from "@/lib/storage-keys";
import { errorKey } from "@/lib/invoke-error";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";

const clientErrorKey = (cause: unknown) =>
  errorKey(cause, ["clients", "newapi"], "clients.errors.storage");

interface ClaudeConfigProfile {
  id: string;
  name: string;
  request: ClientConfigRequest;
}
const CLAUDE_PROFILES_KEY = storageKeys.claudeCodeProfiles;

function readClaudeProfiles(): ClaudeConfigProfile[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(CLAUDE_PROFILES_KEY) ?? "[]");
    if (!Array.isArray(value)) return [];
    return value.filter((item): item is ClaudeConfigProfile =>
      !!item && typeof item === "object" && typeof item.id === "string" && typeof item.name === "string"
      && !!item.request && typeof item.request === "object" && item.request.targetId === "claudeCode"
      && typeof item.request.providerId === "string" && typeof item.request.model === "string");
  } catch { return []; }
}

function selectedProviderModels(provider: NewApiProvider, protocols: ReturnType<typeof clientProtocols>, saved?: string[]) {
  const available = clientModels(provider, protocols);
  return saved === undefined ? available : available.filter(id => saved.includes(id));
}

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
    const hasSavedModelSelection = isClaude && saved?.providerId === initial?.id && saved?.claudeModelIds != null;
    const selectedModels = hasSavedModelSelection ? restoredModels : models;
    const model = initial?.id === client.providerId && selectedModels.includes(client.model ?? "")
      ? client.model! : selectedModels[0] ?? "";
    const selectedIds = saved?.providerIds?.length ? saved.providerIds.filter(id => available.some(provider => provider.id === id))
      : !isClaude && client.configured && accessMode === "router" ? available.map(provider => provider.id) : [];
    const canRestore = saved?.providerId === initial?.id && saved?.model === model;
    return {
      targetId: client.targetId, providerId: initial?.id ?? "", model, accessMode,
      providerIds: Array.from(new Set([...selectedIds, ...(initial ? [initial.id] : [])])),
      providerModelIds: isClaude ? undefined : Object.fromEntries(available.map(provider => [
        provider.id, saved?.providerModelIds?.[provider.id] ?? clientModels(provider, protocols),
      ])),
      ...clientDefaults(initial, model),
      ...(canRestore && saved ? { contextWindow: saved.contextWindow, maxOutputTokens: saved.maxOutputTokens } : {}),
      claudeModels: roles.reduce<ClaudeModels>((mapping, role) => {
        const value = canRestore ? saved?.claudeModels?.[role] : "";
        if (value && selectedModels.includes(value)) mapping[role] = value;
        return mapping;
      }, emptyRoles()),
      claudeModelIds: isClaude
        ? (saved?.providerId === initial?.id && saved.claudeModelIds != null ? saved.claudeModelIds : undefined)
        : undefined,
      claudeOneM: isClaude ? claudeOneMDefaults(saved?.claudeOneM) : undefined,
      authField: saved?.authField ?? "authToken",
    };
  });
  const [preview, setPreview] = useState<ClientConfigPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [claudeProfiles, setClaudeProfiles] = useState<ClaudeConfigProfile[]>(() => isClaude ? readClaudeProfiles() : []);
  const [profileName, setProfileName] = useState("");
  const [editingProfile, setEditingProfile] = useState(false);
  const [managingProfiles, setManagingProfiles] = useState(false);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const protocols = clientProtocols(client, request.accessMode);
  const compatible = providers.filter(provider => !exclusionOf(provider, protocols));
  const selected = compatible.filter(provider => request.providerIds.includes(provider.id));
  const provider = selected.find(provider => provider.id === request.providerId);
  const models = provider ? clientModels(provider, protocols) : [];
  const modelIdsForProvider = (entry: NewApiProvider) => {
    return selectedProviderModels(entry, protocols, request.providerModelIds?.[entry.id]);
  };
  const configuredModels = isClaude ? models.filter(id => request.claudeModelIds?.includes(id) ?? true) : models;
  const modelCompatible = !!provider && (isClaude ? configuredModels : modelIdsForProvider(provider)).includes(request.model);
  const staleClaudeModelSelection = isClaude && !!provider
    && (request.claudeModelIds ?? []).some(id => !models.includes(id));
  const staleOpenCodeModelSelection = client.targetId === "opencode" && selected.some(entry =>
    (request.providerModelIds?.[entry.id] ?? []).some(id => !clientModels(entry, protocols).includes(id)));
  const selectedProtocol = provider ? providerModelProtocol(provider, request.model) : null;
  const startupOneMUnavailable = isClaude && knownSmallContext(effectiveContext(provider, request.model));
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
    const available = isClaude && sameProvider ? nextModels.filter(id => request.claudeModelIds?.includes(id) ?? true)
      : isClaude ? nextModels : next ? modelIdsForProvider(next) : [];
    const id = model ?? (sameProvider && available.includes(request.model) ? request.model : available[0]) ?? "";
    return { providerId: next?.id ?? "", model: id, ...clientDefaults(next, id),
      claudeModelIds: isClaude ? (sameProvider ? request.claudeModelIds : undefined) : undefined,
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
  const selectOpenCodeModels = (entry: NewApiProvider, next: string[]) => {
    const ids = clientModels(entry, protocols).filter(id => next.includes(id));
    if (ids.length === 0) return;
    const providerModelIds = { ...request.providerModelIds, [entry.id]: ids };
    if (entry.id === request.providerId && !ids.includes(request.model)) {
      const nextModel = ids[0];
      update({ providerModelIds, ...chooseDefault(entry, nextModel) });
    } else update({ providerModelIds });
  };
  const changeMode = (accessMode: ClientAccessMode) => {
    const nextProtocols = clientProtocols(client, accessMode);
    const eligible = providers.filter(p => !exclusionOf(p, nextProtocols));
    const next = eligible.find(p => p.id === request.providerId) ?? eligible[0];
    const availableModels = next ? (isClaude ? clientModels(next, nextProtocols)
      : selectedProviderModels(next, nextProtocols, request.providerModelIds?.[next.id])) : [];
    const nextModels = isClaude && next?.id === request.providerId && request.claudeModelIds != null
      ? availableModels.filter(id => request.claudeModelIds?.includes(id) ?? true) : availableModels;
    const model = nextModels.includes(request.model) ? request.model : nextModels[0] ?? "";
    const changedProvider = next?.id !== request.providerId;
    const claudeModels = !isClaude || changedProvider ? emptyRoles() : roles.reduce<ClaudeModels>((mapping, role) => {
      const value = request.claudeModels[role];
      if (value && nextModels.includes(value)) mapping[role] = value;
      return mapping;
    }, emptyRoles());
    const droppedSelections = isClaude && (request.claudeModelIds ?? []).some(id => !nextModels.includes(id));
    const ids = isClaude ? [] : request.providerIds.filter(id => eligible.some(p => p.id === id));
    update({ accessMode, providerId: next?.id ?? "", model,
      providerIds: Array.from(new Set([...ids, ...(next ? [next.id] : [])])),
      providerModelIds: isClaude ? undefined : Object.fromEntries(eligible.map(p => [
        p.id, selectedProviderModels(p, nextProtocols, request.providerModelIds?.[p.id]),
      ])),
      claudeModelIds: isClaude && request.claudeModelIds != null && !changedProvider ? nextModels : undefined,
      claudeModels, ...clientDefaults(next, model) });
    if (droppedSelections) setError("clients.errors.modeDroppedModels");
  };
  const toggleProvider = (next: NewApiProvider) => {
    if (isClaude) { update({ ...chooseDefault(next), providerIds: [next.id] }); return; }
    const ids = request.providerIds.includes(next.id)
      ? request.providerIds.filter(id => id !== next.id) : [...request.providerIds, next.id];
    const providerModelIds = { ...request.providerModelIds };
    if (!request.providerIds.includes(next.id)) {
      providerModelIds[next.id] = selectedProviderModels(next, protocols, providerModelIds[next.id]);
    }
    const primary = compatible.find(p => p.id === request.providerId && ids.includes(p.id))
      ?? compatible.find(p => ids.includes(p.id));
    update({ providerIds: ids, providerModelIds, ...(primary?.id !== request.providerId ? chooseDefault(primary) : {}) });
  };
  const saveClaudeProfile = () => {
    const name = profileName.trim() || provider?.name || request.providerId;
    if (!provider || !modelCompatible) return;
    const profile: ClaudeConfigProfile = {
      id: crypto.randomUUID(), name,
      request: { ...request, claudeModelIds: configuredModels, providerIds: [request.providerId], providerModelIds: undefined },
    };
    const next = [...claudeProfiles.filter(item => !(item.name === name && item.request.providerId === request.providerId)), profile];
    try {
      localStorage.setItem(CLAUDE_PROFILES_KEY, JSON.stringify(next));
      setClaudeProfiles(next);
      setProfileName(name);
      setEditingProfile(false);
      setError(null);
    } catch { setError("clients.errors.storage"); }
  };
  const deleteClaudeProfile = (id: string) => {
    const profile = claudeProfiles.find(item => item.id === id);
    if (!profile) return;
    setConfirmRequest({
      title: t("clients.deleteProfile", { name: profile.name }),
      description: t("clients.deleteProfileConfirm", { name: profile.name }),
      confirmLabel: t("common.delete"),
      destructive: true,
      onConfirm: () => removeClaudeProfile(id),
    });
  };
  const removeClaudeProfile = (id: string) => {
    const next = claudeProfiles.filter(item => item.id !== id);
    try {
      localStorage.setItem(CLAUDE_PROFILES_KEY, JSON.stringify(next));
      setClaudeProfiles(next);
      setError(null);
    } catch { setError("clients.errors.storage"); }
  };
  const loadClaudeProfile = async (id: string) => {
    if (busy) return;
    const profile = claudeProfiles.find(item => item.id === id);
    if (!profile) return;
    const accessMode = profile.request.accessMode === "router" ? "router" : "native";
    const profileProtocols = clientProtocols(client, accessMode);
    const nextProvider = providers.find(item => item.id === profile.request.providerId && !exclusionOf(item, profileProtocols));
    if (!nextProvider) { setError("clients.errors.provider"); return; }
    const availableModels = clientModels(nextProvider, profileProtocols);
    const savedModelIds = profile.request.claudeModelIds;
    const modelIds = savedModelIds == null ? availableModels : availableModels.filter(model => savedModelIds.includes(model));
    if (modelIds.length === 0 || (savedModelIds != null && modelIds.length !== new Set(savedModelIds).size)) {
      setError("clients.errors.profileModelsUnavailable"); return;
    }
    const model = profile.request.model;
    if (!modelIds.includes(model)) { setError("clients.errors.profileModelsUnavailable"); return; }
    const claudeModels = roles.reduce<ClaudeModels>((mapping, role) => {
      const value = profile.request.claudeModels?.[role];
      if (value && !modelIds.includes(value)) return mapping;
      if (value) mapping[role] = value;
      return mapping;
    }, emptyRoles());
    if (roles.some(role => profile.request.claudeModels?.[role] && !modelIds.includes(profile.request.claudeModels[role]))) {
      setError("clients.errors.profileModelsUnavailable"); return;
    }
    const oneM = claudeOneMDefaults(profile.request.claudeOneM);
    const defaults = clientDefaults(nextProvider, model);
    const savedContext = profile.request.contextWindow;
    const contextWindow = model === profile.request.model && Number.isSafeInteger(savedContext) && (savedContext ?? 0) >= 1024
      ? savedContext : defaults.contextWindow;
    const savedOutput = profile.request.maxOutputTokens;
    const maxOutputTokens = model === profile.request.model && Number.isSafeInteger(savedOutput)
      && (savedOutput ?? 0) >= 1 && (contextWindow == null || (savedOutput ?? 0) <= contextWindow)
      ? savedOutput
      : defaults.maxOutputTokens === null ? null : Math.min(defaults.maxOutputTokens, contextWindow ?? defaults.maxOutputTokens);
    const profileRequest: ClientConfigRequest = { ...profile.request, targetId: client.targetId, accessMode, providerId: nextProvider.id,
      providerIds: [nextProvider.id], providerModelIds: undefined, claudeModelIds: modelIds, model,
      claudeModels, claudeOneM: oneM, contextWindow, maxOutputTokens,
      authField: profile.request.authField === "apiKey" ? "apiKey" : "authToken" };
    setRequest(profileRequest);
    setPreview(null);
    setProfileName(profile.name);
    setBusy(true);
    setError(null);
    try {
      // Show the change first; the user applies it with the regular Apply button,
      // just like a manual edit.
      setPreview(await api.targets.previewConfig(profileRequest));
    } catch (cause) {
      setError(clientErrorKey(cause));
    } finally { setBusy(false); }
  };
  const run = async () => {
    if (busy) return;
    if (staleOpenCodeModelSelection) {
      setError("clients.errors.openCodeModelsUnavailable"); return;
    }
    if (!modelCompatible || request.providerIds.some(id => !compatible.some(p => p.id === id))) {
      setError("clients.errors.provider"); return;
    }
    if (isClaude && request.claudeModelIds?.some(id => !models.includes(id))) {
      setError("clients.errors.modelSelection"); return;
    }
    if (isClaude && roles.some(role => request.claudeModels[role] && !configuredModels.includes(request.claudeModels[role]))) {
      setError("clients.errors.protocol"); return;
    }
    // Only user-entered values are explicit; empty fields follow the backend
    // resolver, so a catalog refresh can move them.
    const context = request.contextWindow ?? null;
    const output = request.maxOutputTokens ?? null;
    if ((context !== null && (!Number.isSafeInteger(context) || context < 1024))
      || (output !== null && (!Number.isSafeInteger(output) || output < 1))
      || (context !== null && output !== null && output > context)) {
      setError("clients.errors.limits"); return;
    }
    setBusy(true);
    setError(null);
    try {
      const configuredRequest = isClaude ? { ...request, claudeModelIds: configuredModels } : { ...request, providerModelIds: Object.fromEntries(
        selected.map(entry => [entry.id, modelIdsForProvider(entry)]),
      ) };
      if (preview) {
        const applied = await api.targets.applyConfig(configuredRequest, preview.revision);
        await onApplied();
        toast({
          title: t("clients.applied"),
          description: applied.warnings?.length
            ? applied.warnings.map(key => t(key)).join(" ")
            : t("clients.restartHint"),
          variant: applied.warnings?.length ? "destructive" : "success",
        });
        onClose();
      } else { setPreview(await api.targets.previewConfig(configuredRequest)); }
    } catch (cause) {
      setError(clientErrorKey(cause)); setPreview(null); await onApplied();
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
          {isClaude && <section className="space-y-2 border-b border-border/60 pb-4" aria-label={t("clients.savedProfiles")}>
            <div className="flex flex-wrap items-center gap-2">
              {claudeProfiles.length > 0
                ? <Label htmlFor="claude-saved-profile" className="min-w-0 text-sm font-medium">{t("clients.savedProfiles")}</Label>
                : <span className="min-w-0 flex-1 text-sm text-muted-foreground">{t("clients.noSavedProfiles")}</span>}
              {claudeProfiles.length > 0 ? <Select disabled={busy} value="" onValueChange={value => void loadClaudeProfile(value)}>
                <SelectTrigger id="claude-saved-profile" className="h-9 min-w-44 flex-1"><SelectValue placeholder={t("clients.selectSavedProfile")} /></SelectTrigger>
                <SelectContent>{claudeProfiles.map(item => <SelectItem key={item.id} value={item.id}>{item.name} · {providers.find(p => p.id === item.request.providerId)?.name ?? item.request.providerId}</SelectItem>)}</SelectContent>
              </Select> : null}
              <Button type="button" size="sm" variant="ghost" disabled={busy || (!editingProfile && !modelCompatible)}
                aria-expanded={editingProfile} aria-controls="claude-profile-editor" onClick={() => setEditingProfile(value => !value)}>
                <BookmarkPlus className="h-4 w-4" />{editingProfile ? t("common.cancel") : t("clients.saveProfile")}
              </Button>
              {claudeProfiles.length > 0 && <Button type="button" size="sm" variant="ghost" disabled={busy}
                aria-expanded={managingProfiles} aria-controls="claude-profile-management" onClick={() => setManagingProfiles(value => !value)}>
                {t(managingProfiles ? "common.cancel" : "clients.manageProfiles")}
              </Button>}
            </div>
            {editingProfile && <div id="claude-profile-editor" className="flex flex-wrap gap-2 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-150">
              <Input aria-label={t("clients.profileName")} placeholder={provider?.name ?? t("clients.profileName")} value={profileName}
                onChange={event => setProfileName(event.target.value)} disabled={busy || !provider} className="h-9 min-w-40 flex-1" />
              <Button type="button" size="sm" variant="outline" disabled={busy || !modelCompatible} onClick={saveClaudeProfile}>
                {t("clients.confirmSaveProfile")}
              </Button>
            </div>}
            {managingProfiles && <ul id="claude-profile-management" className="space-y-1 rounded-lg border bg-muted/20 p-2 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-150">
              {claudeProfiles.map(item => <li key={item.id} className="flex min-w-0 items-center gap-2 px-1 py-1">
                <span className="min-w-0 flex-1 truncate text-sm" title={item.name}>{item.name}</span>
                <span className="max-w-[40%] truncate text-xs text-muted-foreground" title={providers.find(p => p.id === item.request.providerId)?.name ?? item.request.providerId}>
                  {providers.find(p => p.id === item.request.providerId)?.name ?? item.request.providerId}
                </span>
                <Button type="button" size="icon-sm" variant="ghost" disabled={busy}
                  aria-label={t("clients.deleteProfile", { name: item.name })} onClick={() => deleteClaudeProfile(item.id)}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              </li>)}
            </ul>}
          </section>}
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
                const availableCount = clientModels(p, protocols).length;
                return <button key={p.id} type="button" disabled={busy || !!reason} aria-pressed={active} onClick={() => toggleProvider(p)}
                  className={"flex min-w-0 items-center gap-3 rounded-lg border p-3 text-left transition-[border-color,background-color,transform] duration-150 motion-safe:active:scale-[0.99] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45 "
                    + (active ? "border-primary/50 bg-primary/5" : "bg-card/60 hover:bg-muted/60")}>
                  <span className="min-w-0 flex-1"><span className="block truncate text-sm font-medium" title={p.name}>{p.name}</span>
                    <span className="mt-1 block text-xs text-muted-foreground">{reason ? t("clients.excluded." + reason, { count: 1 })
                      : t("clients.modelsCount", { count: availableCount })}</span></span>
                  <span className={"flex h-4 w-4 shrink-0 items-center justify-center border " + (isClaude ? "rounded-full " : "rounded ")
                    + (active ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
                    {active && <Check className="h-3 w-3" />}</span>
                </button>;
              })}
            </div>
            {compatible.length === 0 && <p className="text-xs text-muted-foreground">{t(isClaude && request.accessMode === "native" ? "clients.noNativeProviders" : "clients.noProviders")}</p>}
          </div>
          {client.targetId === "opencode" && selected.length > 0 && <section className="space-y-2" aria-label={t("clients.modelSubset")}>
            <p className="text-sm font-medium">{t("clients.modelSubset")}</p>
            <div className="space-y-2">{selected.map(entry => {
              const allModels = clientModels(entry, protocols);
              const checkedModels = modelIdsForProvider(entry);
              return <details key={entry.id} className="group overflow-hidden rounded-lg border bg-card/60">
                <summary className="flex min-h-10 cursor-pointer list-none items-center justify-between gap-3 px-3 py-2 text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
                  <span className="min-w-0 truncate">{entry.name}</span>
                  <span className="ml-auto shrink-0 text-xs font-normal tabular-nums text-muted-foreground">{t("clients.selectedOfAvailable", { selected: checkedModels.length, total: allModels.length })}</span>
                  <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200 group-open:rotate-180 motion-reduce:transition-none" />
                </summary>
                <div className="max-h-48 divide-y overflow-y-auto border-t motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-150">
                  {allModels.map(id => <label key={id} className="flex min-w-0 items-center gap-3 px-3 py-2 transition-colors hover:bg-muted/40">
                    <input type="checkbox" aria-label={t("clients.includeModel", { model: modelLabel(entry, id) })}
                      checked={checkedModels.includes(id)} disabled={busy || (checkedModels.includes(id) && checkedModels.length === 1)}
                      className="h-4 w-4 shrink-0 accent-primary focus-visible:ring-2 focus-visible:ring-ring" onChange={() => selectOpenCodeModels(entry,
                        checkedModels.includes(id) ? checkedModels.filter(model => model !== id) : [...checkedModels, id])} />
                    <span className="min-w-0 flex-1 break-words text-sm">{modelLabel(entry, id)}</span>
                    {id === entry.model && <Badge variant="outline" className="shrink-0 text-[10px]">{t("clients.providerDefault")}</Badge>}
                  </label>)}
                </div>
              </details>;
            })}</div>
          </section>}
          {isClaude && <ClaudeModelConfig key={request.providerId + ":" + request.accessMode}
            provider={provider} models={models} selected={configuredModels} mapping={request.claudeModels}
            defaultModel={request.model} disabled={busy} oneM={claudeOneMDefaults(request.claudeOneM)}
            onSelectedChange={selectClaudeModels} onMappingChange={claudeModels => update({ claudeModels })}
            onOneMChange={claudeOneM => update({ claudeOneM })} />}
          {client.targetId === "opencode" && staleOpenCodeModelSelection &&
            <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">
              {t("clients.staleOpenCodeModels")}
            </p>}
          {isClaude && staleClaudeModelSelection && <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">
            {t("clients.errors.profileModelsUnavailable")}
          </p>}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2"><Label htmlFor="client-model">{t(isClaude ? "clients.startupModel" : "clients.model")}</Label>
              {selectedProtocol && <Badge variant="outline" className="text-[10px]">{t("newapi.protocol" + (selectedProtocol === "responses" ? "Responses" : selectedProtocol === "chatCompletions" ? "ChatCompletions" : "AnthropicMessages"))}</Badge>}
            </div>
            <Select disabled={busy || selected.length === 0} value={modelCompatible ? JSON.stringify([request.providerId, request.model]) : ""}
              onValueChange={value => { const [id, model] = JSON.parse(value) as [string, string]; update(chooseDefault(selected.find(p => p.id === id), model)); }}>
              <SelectTrigger id="client-model"><SelectValue placeholder={t("clients.selectModel")} /></SelectTrigger>
              <SelectContent>{selected.flatMap(p => (isClaude ? configuredModels : modelIdsForProvider(p)).map(id => <SelectItem key={JSON.stringify([p.id, id])} value={JSON.stringify([p.id, id])}>
                {(!isClaude && selected.length > 1 ? p.name + " / " : "") + modelLabel(p, id)}
              </SelectItem>))}</SelectContent>
            </Select>
            {isClaude && <label className="flex items-center gap-2 pt-1 text-xs text-muted-foreground"
              title={startupOneMUnavailable ? t("clients.oneMUnavailable") : undefined}>
              <input type="checkbox" checked={!startupOneMUnavailable && (request.claudeOneM?.startup ?? false)}
                disabled={busy || !request.model || startupOneMUnavailable}
                className="h-4 w-4 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50"
                onChange={event => update({ claudeOneM: { ...claudeOneMDefaults(request.claudeOneM), startup: event.target.checked } })} />
              {t("clients.oneMStartup")}
            </label>}
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
                <div className="space-y-1.5"><Label htmlFor="client-context">{t("clients.context")}</Label>
                  <Input id="client-context" type="number" min={1024} placeholder={t("clients.limitsAuto")} value={request.contextWindow ?? ""}
                    onChange={event => update({ contextWindow: event.target.value === "" ? null : Number(event.target.value) })} />
                </div>
                <div className="space-y-1.5"><Label htmlFor="client-output">{t("clients.output")}</Label>
                  <Input id="client-output" type="number" min={1} placeholder={t("clients.limitsAuto")} value={request.maxOutputTokens ?? ""}
                    onChange={event => update({ maxOutputTokens: event.target.value === "" ? null : Number(event.target.value) })} />
                </div>
                <p className="col-span-2 text-[11px] leading-relaxed text-muted-foreground">{t("clients.limitsHint")}</p>
              </div>}
              <p className="break-all font-mono text-xs text-muted-foreground">{client.path}</p>
            </div>
          </details>
        </fieldset>
        {preview && <div className="space-y-2"><p className="text-sm font-medium">{t("clients.previewTitle")}</p>
          {!!preview.warnings?.length && <ul className="space-y-1">
            {preview.warnings.map(key => <li key={key} role="status"
              className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">{t(key)}</li>)}
          </ul>}
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
        <ConfirmDialog request={confirmRequest} onClose={() => setConfirmRequest(null)} />
      </DialogContent>
    </Dialog>
  );
}
