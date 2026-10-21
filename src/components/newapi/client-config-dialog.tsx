import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { aggregationQueryKey, type ModelTargetRef } from "@/types/aggregation";
import { useTranslation } from "react-i18next";
import { BookmarkPlus, Check, ChevronDown, Eye, Loader2, Search, Star, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { claudeRoles as roles, emptyClaudeRoles as emptyRoles, claudeOneMDefaults, clientDefaults, clientModels, clientProtocols, conflictScopes, exclusionOf, knownSmallContext, effectiveContext, reloadHint } from "@/lib/client-config";
import { ClaudeModelConfig } from "./claude-model-config";
import { DirectProxyNote } from "./direct-proxy-note";
import { ModelSourceSwitches, type SourceKind } from "./model-source-switches";
import type { ModelSources } from "@/types/gateway";
import { providerModelProtocol } from "@/lib/provider-models";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import type { ClaudeModels, ClientAccessMode, ClientConfigApplied, ClientConfigPreview, ClientConfigRequest, ClientStatus } from "@/types/clients";
import type { NewApiProvider } from "@/types/newapi";
import { storageKeys } from "@/lib/storage-keys";
import { errorKey } from "@/lib/invoke-error";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";

const clientErrorKey = (cause: unknown) =>
  errorKey(cause, ["clients", "newapi", "aggregation", "modelTargets"], "clients.errors.storage");

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
      && (typeof item.request.providerId === "string" || item.request.modelSelection?.defaultTarget?.kind === "aggregate")
      && typeof item.request.model === "string");
  } catch { return []; }
}

function selectedProviderModels(provider: NewApiProvider, protocols: ReturnType<typeof clientProtocols>, saved?: string[]) {
  const available = clientModels(provider, protocols);
  return saved === undefined ? available : available.filter(id => saved.includes(id));
}

const sourceKind = (provider: NewApiProvider): SourceKind => provider.accountSource ? "account" : "relay";

/** One model of Claude Code's combined menu, by its gateway id. */
interface MixedEntry { id: string; label: string; target: ModelTargetRef; small: boolean }

/** A client on a remote host: configured direct only, through the host's own commands. */
export interface RemoteConfigTarget {
  hostName: string;
  preview: (request: ClientConfigRequest, replaceConflicts: boolean) => Promise<ClientConfigPreview>;
  apply: (request: ClientConfigRequest, revision: string, replaceConflicts: boolean) => Promise<ClientConfigApplied>;
}

export function ConfigDialog({ client, providers, onClose, onApplied, remote }: {
  client: ClientStatus; providers: NewApiProvider[]; onClose: () => void; onApplied: () => Promise<void>;
  remote?: RemoteConfigTarget;
}) {
  const { t } = useTranslation();
  const previewConfig = remote ? remote.preview : api.targets.previewConfig;
  const applyConfig = remote ? remote.apply : api.targets.applyConfig;
  const isClaude = client.targetId === "claudeCode";
  const isHarness = client.targetId === "harness";
  const harnessProfiles = client.harness?.profiles ?? [];
  const aggregateQuery = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const aggregates = aggregateQuery.data?.models ?? [];
  const [request, setRequest] = useState<ClientConfigRequest>(() => {
    const saved = client.request;
    if (saved?.modelSelection) return { ...saved, providerIds: saved.providerIds ?? [], claudeModels: saved.claudeModels ?? emptyRoles() };
    const accessMode = remote ? "native" : saved?.accessMode ?? (client.configured ? "router" : "native");
    const protocols = clientProtocols(client, accessMode);
    const available = providers.filter(provider => !exclusionOf(provider, protocols, accessMode));
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
        ? (saved?.providerId === initial?.id && saved?.claudeModelIds != null ? saved.claudeModelIds : undefined)
        : undefined,
      claudeOneM: isClaude ? claudeOneMDefaults(saved?.claudeOneM) : undefined,
      authField: saved?.authField ?? "authToken",
      ...(isHarness ? { harnessProfile: saved?.harnessProfile ?? harnessProfiles[0]?.name ?? null,
        harnessDefaultModel: saved?.harnessDefaultModel ?? true,
        harnessReasoningEffort: canRestore ? saved?.harnessReasoningEffort ?? null : null } : {}),
    };
  });
  // A configured client shows the kinds it uses; a new one offers every kind.
  const [sources, setSources] = useState<ModelSources>(() => {
    if (!client.configured || request.accessMode !== "router") return { relay: true, account: true, aggregate: true };
    const chosen = providers.filter(provider => request.providerIds.includes(provider.id));
    const pooled = (request.modelSelection?.availableTargets ?? []).some(target => target.kind === "aggregate");
    return { relay: chosen.some(provider => !provider.accountSource), account: chosen.some(provider => !!provider.accountSource), aggregate: pooled };
  });
  const [preview, setPreview] = useState<ClientConfigPreview | null>(null);
  useEffect(() => { setPreview(null); }, [client.conflict]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [claudeProfiles, setClaudeProfiles] = useState<ClaudeConfigProfile[]>(() => isClaude ? readClaudeProfiles() : []);
  const [profileName, setProfileName] = useState("");
  const [editingProfile, setEditingProfile] = useState(false);
  const [managingProfiles, setManagingProfiles] = useState(false);
  const [confirmRequest, setConfirmRequest] = useState<ConfirmRequest | null>(null);
  const [sourceQuery, setSourceQuery] = useState("");
  const previewRef = useRef<HTMLDivElement>(null);
  // A fresh preview scrolls into view beside the choices it describes.
  useEffect(() => { if (preview) previewRef.current?.scrollIntoView({ block: "nearest" }); }, [preview]);
  const protocols = clientProtocols(client, request.accessMode);
  const routerMode = request.accessMode === "router";
  const defaultTarget = request.modelSelection?.defaultTarget;
  const aggregateIds = (request.modelSelection?.availableTargets ?? []).flatMap(target => target.kind === "aggregate" ? [target.aggregateId] : []);
  const selectedAggregates = aggregates.filter(model => aggregateIds.includes(model.id));
  const aggregateUnavailable = aggregateIds.some(id => !aggregates.some(model => model.id === id && model.available));
  const compatible = providers.filter(provider => !exclusionOf(provider, protocols, request.accessMode));
  const selected = compatible.filter(provider => request.providerIds.includes(provider.id));
  // Claude Code combining sources: several providers, or providers and aggregates.
  const claudeMixed = isClaude && routerMode && (selected.length > 1 || (selected.length > 0 && aggregateIds.length > 0));
  const aggregateMode = !claudeMixed && defaultTarget?.kind === "aggregate";
  const defaultAggregate = aggregateMode && defaultTarget?.kind === "aggregate" ? aggregates.find(model => model.id === defaultTarget.aggregateId) : undefined;
  const modelLabel = (p: NewApiProvider, id: string) => p.models.find(model => model.id === id)?.displayName || id;
  const poolOneM = (model: (typeof aggregates)[number]) => model.memberStates.some(member =>
    member.available && member.projection && !member.projection.contextIsFallback && member.projection.contextWindow >= 1_000_000);
  const aggregateOneM = (id: string) => {
    const model = selectedAggregates.find(entry => entry.publicModelId === id);
    return !!model && poolOneM(model);
  };
  const mixedEntriesFor = (chosen: NewApiProvider[], pools: typeof aggregates): MixedEntry[] => [
    ...chosen.flatMap(entry => clientModels(entry, protocols).map(id => ({
      id: `${entry.gatewaySlug ?? entry.id}/${id}`, label: `${entry.name} / ${modelLabel(entry, id)}`,
      target: { kind: "provider", providerId: entry.id, modelId: id } as ModelTargetRef, small: knownSmallContext(effectiveContext(entry, id)),
    }))),
    ...pools.map(model => ({ id: model.gatewayId ?? model.publicModelId, label: model.displayName,
      target: { kind: "aggregate", aggregateId: model.id } as ModelTargetRef, small: !poolOneM(model) })),
  ];
  const mixedEntries = claudeMixed ? mixedEntriesFor(selected, selectedAggregates) : [];
  const mixedEntry = (id: string) => mixedEntries.find(entry => entry.id === id);
  const provider = claudeMixed ? undefined : selected.find(provider => provider.id === request.providerId);
  const models = claudeMixed ? mixedEntries.map(entry => entry.id)
    : isClaude && aggregateMode ? selectedAggregates.map(model => model.publicModelId) : provider ? clientModels(provider, protocols) : [];
  const modelIdsForProvider = (entry: NewApiProvider) => {
    return selectedProviderModels(entry, protocols, request.providerModelIds?.[entry.id]);
  };
  const configuredModels = isClaude ? models.filter(id => request.claudeModelIds?.includes(id) ?? true) : models;
  const modelCompatible = aggregateMode ? !!defaultAggregate?.available && defaultAggregate.publicModelId === request.model
    : claudeMixed ? configuredModels.includes(request.model)
    : !!provider && (isClaude ? configuredModels : modelIdsForProvider(provider)).includes(request.model);
  const staleClaudeModelSelection = isClaude && !!provider
    && (request.claudeModelIds ?? []).some(id => !models.includes(id));
  const staleOpenCodeModelSelection = !isClaude && selected.some(entry =>
    (request.providerModelIds?.[entry.id] ?? []).some(id => !clientModels(entry, protocols).includes(id)));
  const selectedProtocol = provider ? providerModelProtocol(provider, request.model) : null;
  const startupOneMUnavailable = isClaude && (claudeMixed ? !!mixedEntry(request.model)?.small
    : aggregateMode ? !aggregateOneM(request.model) : knownSmallContext(effectiveContext(provider, request.model)));
  /** The request Claude Code's combined menu is applied with: every model it may hold, by target. */
  const mixedRequest = (): ClientConfigRequest => {
    const target = mixedEntry(request.model)?.target;
    return { ...request, providerId: target?.kind === "provider" ? target.providerId : "", claudeModelIds: configuredModels,
      providerIds: selected.map(entry => entry.id), providerModelIds: undefined, aggregateIngressProtocol: "anthropicMessages",
      modelSelection: target ? { defaultTarget: target, availableTargets: mixedEntries.map(entry => entry.target) } : undefined };
  };
  /** The request as it is previewed and applied: every target the client may reach. */
  const buildConfiguredRequest = (): ClientConfigRequest => {
    if (claudeMixed) return mixedRequest();
    if (isClaude) return { ...request, claudeModelIds: configuredModels };
    const availableTargets: ModelTargetRef[] = [...selected.flatMap(entry => modelIdsForProvider(entry).map(modelId => ({ kind: "provider" as const, providerId: entry.id, modelId }))),
      ...aggregateIds.map(aggregateId => ({ kind: "aggregate" as const, aggregateId }))];
    return { ...request,
      modelSelection: aggregateIds.length ? { defaultTarget: aggregateMode ? defaultTarget : { kind: "provider" as const, providerId: request.providerId, modelId: request.model }, availableTargets } : undefined,
      providerModelIds: Object.fromEntries(selected.map(entry => [entry.id, modelIdsForProvider(entry)])) };
  };
  // Harness's thinking levels for the default model, resolved by the backend like the routes it writes.
  const reasoningRequest = isHarness && modelCompatible && request.harnessDefaultModel !== false
    ? { ...buildConfiguredRequest(), harnessReasoningEffort: null } : null;
  const reasoningQuery = useQuery({
    queryKey: ["harness-reasoning", reasoningRequest],
    queryFn: () => api.targets.harnessReasoning(reasoningRequest!),
    enabled: !!reasoningRequest,
    retry: false,
  });
  const reasoning = reasoningRequest ? reasoningQuery.data : undefined;
  const profileTargetLabel = (profile: ClaudeConfigProfile) => {
    const target = profile.request.modelSelection?.defaultTarget;
    return target?.kind === "aggregate"
      ? aggregates.find(model => model.id === target.aggregateId)?.displayName ?? t("aggregation.unavailable")
      : providers.find(p => p.id === profile.request.providerId)?.name ?? profile.request.providerId;
  };
  const update = (patch: Partial<ClientConfigRequest>) => {
    if (busy) return;
    setRequest(current => {
      const next = { ...current, ...patch };
      // A chosen thinking level belongs to the model it was picked for.
      return isHarness && (next.model !== current.model || next.providerId !== current.providerId)
        ? { ...next, harnessReasoningEffort: null } : next;
    });
    setPreview(null);
    setError(null);
  };
  const chooseAggregate = (aggregateId: string, ids = aggregateIds) => {
    const patch = aggregatePatch(aggregateId, ids);
    if (patch) update(patch);
  };
  const aggregatePatch = (aggregateId: string, ids = aggregateIds): Partial<ClientConfigRequest> | null => {
    const entry = aggregates.find(model => model.id === aggregateId);
    if (!entry) return null;
    return { accessMode: "router", providerId: "", model: entry.publicModelId, contextWindow: null, maxOutputTokens: null,
      ...(isClaude ? { providerIds: [], providerModelIds: undefined,
        claudeModelIds: ids.flatMap(id => aggregates.find(model => model.id === id)?.publicModelId ?? []),
        claudeModels: roles.reduce<ClaudeModels>((mapping, role) => {
          const value = aggregateMode ? request.claudeModels[role] : "";
          if (ids.some(id => aggregates.find(model => model.id === id)?.publicModelId === value)) mapping[role] = value;
          return mapping;
        }, emptyRoles()) } : {}),
      aggregateIngressProtocol: isClaude ? "anthropicMessages" : request.aggregateIngressProtocol ?? "responses",
      modelSelection: { defaultTarget: { kind: "aggregate", aggregateId }, availableTargets: ids.map(id => ({ kind: "aggregate", aggregateId: id })) } };
  };
  /** A provider's first model as Claude Code's start, with nothing kept from another selection. */
  const freshClaudeDefault = (next: NewApiProvider | undefined): Partial<ClientConfigRequest> => {
    const model = next ? clientModels(next, protocols)[0] ?? "" : "";
    return { providerId: next?.id ?? "", model, ...clientDefaults(next, model), modelSelection: undefined,
      claudeModelIds: undefined, claudeModels: emptyRoles() };
  };
  /** Claude Code's providers and aggregates: one relay, aggregates alone, or a combined menu. */
  const claudeReselect = (providerIds: string[], ids: string[]) => {
    const chosen = compatible.filter(entry => providerIds.includes(entry.id));
    if (chosen.length === 0 && ids.length > 0) {
      const keep = defaultTarget?.kind === "aggregate" && ids.includes(defaultTarget.aggregateId) ? defaultTarget.aggregateId : ids[0];
      chooseAggregate(keep, ids);
    } else if (chosen.length === 0) {
      update({ ...freshClaudeDefault(undefined), providerIds: [] });
    } else if (chosen.length === 1 && ids.length === 0) {
      update({ ...(claudeMixed || aggregateMode ? freshClaudeDefault(chosen[0]) : chooseDefault(chosen[0])), providerIds: [chosen[0].id] });
    } else {
      const entries = mixedEntriesFor(chosen, aggregates.filter(model => ids.includes(model.id)));
      const has = (id: string) => entries.some(entry => entry.id === id);
      const model = claudeMixed && has(request.model) ? request.model : entries[0]?.id ?? "";
      const target = entries.find(entry => entry.id === model)?.target;
      update({ providerIds: chosen.map(entry => entry.id), providerId: target?.kind === "provider" ? target.providerId : "", model,
        contextWindow: null, maxOutputTokens: null, aggregateIngressProtocol: "anthropicMessages", providerModelIds: undefined,
        claudeModelIds: claudeMixed ? request.claudeModelIds?.filter(has) : undefined,
        claudeModels: roles.reduce<ClaudeModels>((mapping, role) => {
          const value = claudeMixed ? request.claudeModels[role] : "";
          if (value && has(value)) mapping[role] = value;
          return mapping;
        }, emptyRoles()),
        modelSelection: { defaultTarget: target ?? { kind: "aggregate", aggregateId: ids[0] },
          availableTargets: ids.map(aggregateId => ({ kind: "aggregate", aggregateId })) } });
    }
  };
  /** OpenCode and Pi: the providers and aggregates kept, the default moved off one dropped. */
  const otherReselect = (providerIds: string[], ids: string[]) => {
    const availableTargets = ids.map(aggregateId => ({ kind: "aggregate", aggregateId }) as ModelTargetRef);
    if (defaultTarget?.kind === "aggregate" && ids.includes(defaultTarget.aggregateId)) {
      update({ providerIds, modelSelection: { defaultTarget, availableTargets } });
      return;
    }
    const primary = compatible.find(entry => entry.id === request.providerId && providerIds.includes(entry.id))
      ?? compatible.find(entry => providerIds.includes(entry.id));
    if (primary) {
      const patch: Partial<ClientConfigRequest> = defaultTarget?.kind !== "aggregate" && primary.id === request.providerId ? {} : chooseDefault(primary);
      update({ providerIds, ...patch, modelSelection: { defaultTarget: { kind: "provider", providerId: primary.id,
        modelId: patch.model ?? request.model }, availableTargets } });
    } else if (ids.length) {
      const patch = aggregatePatch(ids[0], ids);
      if (patch) update({ ...patch, providerIds });
    } else update({ providerIds: [], providerId: "", model: "", modelSelection: undefined });
  };
  /** Turning a kind off drops what of it was chosen and hides it. */
  const changeSources = (next: ModelSources) => {
    if (busy) return;
    setSources(next);
    const providerIds = request.providerIds.filter(id => {
      const entry = providers.find(item => item.id === id);
      return !!entry && next[sourceKind(entry)];
    });
    const ids = next.aggregate ? aggregateIds : [];
    if (providerIds.length === request.providerIds.length && ids.length === aggregateIds.length) return;
    if (isClaude) claudeReselect(providerIds, ids);
    else otherReselect(providerIds, ids);
  };
  const toggleAggregate = (id: string) => {
    const ids = aggregateIds.includes(id) ? aggregateIds.filter(value => value !== id) : [...aggregateIds, id];
    if (isClaude) { claudeReselect(selected.map(entry => entry.id), ids); return; }
    if (aggregateMode && defaultTarget?.kind === "aggregate" && !ids.includes(defaultTarget.aggregateId)) {
      if (ids.length) chooseAggregate(defaultTarget?.kind === "aggregate" && ids.includes(defaultTarget.aggregateId) ? defaultTarget.aggregateId : ids[0], ids);
      else update({ modelSelection: undefined, providerId: "", model: "", claudeModelIds: undefined });
    } else update({ modelSelection: { defaultTarget: defaultTarget ?? { kind: "provider", providerId: request.providerId, modelId: request.model },
      availableTargets: ids.map(aggregateId => ({ kind: "aggregate", aggregateId })) } });
  };
  const chooseDefault = (next: NewApiProvider | undefined, model?: string) => {
    const sameProvider = next?.id === request.providerId;
    const nextModels = next ? clientModels(next, protocols) : [];
    const available = isClaude && sameProvider ? nextModels.filter(id => request.claudeModelIds?.includes(id) ?? true)
      : isClaude ? nextModels : next ? modelIdsForProvider(next) : [];
    const id = model ?? (sameProvider && available.includes(request.model) ? request.model : available[0]) ?? "";
    return { providerId: next?.id ?? "", model: id, ...clientDefaults(next, id),
      modelSelection: isClaude ? undefined : { defaultTarget: { kind: "provider", providerId: next?.id ?? "", modelId: id } as ModelTargetRef,
        availableTargets: aggregateIds.map(aggregateId => ({ kind: "aggregate", aggregateId }) as ModelTargetRef) },
      claudeModelIds: isClaude ? (sameProvider ? request.claudeModelIds : undefined) : undefined,
      claudeModels: next?.id === request.providerId ? request.claudeModels : emptyRoles() };
  };
  const selectClaudeModels = (next: string[]) => {
    const ids = models.filter(id => next.includes(id));
    const model = ids.includes(request.model) ? request.model : ids[0] ?? "";
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
    const providerModelIds = { ...request.providerModelIds, [entry.id]: ids };
    if (entry.id === request.providerId && !ids.includes(request.model)) {
      const nextModel = ids[0] ?? "";
      update({ providerModelIds, ...chooseDefault(entry, nextModel) });
    } else update({ providerModelIds });
  };
  const changeMode = (accessMode: ClientAccessMode) => {
    const nextProtocols = clientProtocols(client, accessMode);
    const eligible = providers.filter(p => !exclusionOf(p, nextProtocols, accessMode));
    const next = eligible.find(p => p.id === request.providerId) ?? eligible[0];
    if (claudeMixed) {
      // A combined menu is router-only: start over on one provider.
      const model = next ? clientModels(next, nextProtocols)[0] ?? "" : "";
      update({ accessMode, providerId: next?.id ?? "", model, providerIds: next ? [next.id] : [], modelSelection: undefined,
        claudeModelIds: undefined, claudeModels: emptyRoles(), providerModelIds: undefined, ...clientDefaults(next, model) });
      return;
    }
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
    update({ accessMode, modelSelection: undefined, providerId: next?.id ?? "", model,
      providerIds: Array.from(new Set([...ids, ...(next ? [next.id] : [])])),
      providerModelIds: isClaude ? undefined : Object.fromEntries(eligible.map(p => [
        p.id, selectedProviderModels(p, nextProtocols, request.providerModelIds?.[p.id]),
      ])),
      claudeModelIds: isClaude && request.claudeModelIds != null && !changedProvider ? nextModels : undefined,
      claudeModels, ...clientDefaults(next, model) });
    if (droppedSelections) setError("clients.errors.modeDroppedModels");
  };
  const toggleProvider = (next: NewApiProvider) => {
    if (isClaude && !routerMode) { update({ ...chooseDefault(next), providerIds: [next.id] }); return; }
    const ids = request.providerIds.includes(next.id)
      ? request.providerIds.filter(id => id !== next.id) : [...request.providerIds, next.id];
    if (isClaude) { claudeReselect(ids.filter(id => compatible.some(entry => entry.id === id)), aggregateIds); return; }
    const providerModelIds = { ...request.providerModelIds };
    if (!request.providerIds.includes(next.id)) {
      providerModelIds[next.id] = selectedProviderModels(next, protocols, providerModelIds[next.id]);
    }
    const primary = compatible.find(p => p.id === request.providerId && ids.includes(p.id))
      ?? compatible.find(p => ids.includes(p.id));
    // An aggregate selection is an explicit default. Adding/removing ordinary
    // provider subsets must not silently replace it with the first provider.
    if (aggregateMode) {
      update({ providerIds: ids, providerModelIds });
    } else {
      update({ providerIds: ids, providerModelIds, ...(primary?.id !== request.providerId ? chooseDefault(primary) : {}) });
    }
  };
  const saveClaudeProfile = () => {
    const name = profileName.trim() || defaultAggregate?.displayName || provider?.name || request.providerId;
    if (!modelCompatible) return;
    const profile: ClaudeConfigProfile = {
      id: crypto.randomUUID(), name,
      request: claudeMixed ? mixedRequest()
        : { ...request, claudeModelIds: configuredModels, providerIds: aggregateMode ? [] : [request.providerId], providerModelIds: undefined },
    };
    const next = [...claudeProfiles.filter(item => !(item.name === name && item.request.providerId === request.providerId
      && JSON.stringify(item.request.modelSelection?.defaultTarget) === JSON.stringify(request.modelSelection?.defaultTarget))), profile];
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
    const savedTargets = profile.request.modelSelection?.availableTargets ?? [];
    if (savedTargets.some(target => target.kind === "provider")) {
      // A combined menu: every target must still be offered.
      const providerIds = [...new Set(savedTargets.flatMap(target => target.kind === "provider" ? [target.providerId] : []))];
      const ids = savedTargets.flatMap(target => target.kind === "aggregate" ? [target.aggregateId] : []);
      const routerProtocols = clientProtocols(client, "router");
      const missing = providerIds.some(pid => !providers.some(entry => entry.id === pid && !exclusionOf(entry, routerProtocols, "router")))
        || ids.some(aid => !aggregates.some(model => model.id === aid && model.available));
      if (missing) { setError("clients.errors.profileModelsUnavailable"); return; }
      const restored: ClientConfigRequest = { ...profile.request, targetId: client.targetId, accessMode: "router", providerIds,
        modelSelection: { defaultTarget: profile.request.modelSelection!.defaultTarget,
          availableTargets: ids.map(aggregateId => ({ kind: "aggregate", aggregateId })) } };
      setRequest(restored); setProfileName(profile.name); setPreview(null); setBusy(true); setError(null);
      setSources({ relay: true, account: true, aggregate: true });
      try { setPreview(await previewConfig(profile.request, client.conflict)); }
      catch (cause) { setError(clientErrorKey(cause)); }
      finally { setBusy(false); }
      return;
    }
    if (profile.request.modelSelection?.defaultTarget?.kind === "aggregate") {
      const selection = profile.request.modelSelection;
      const chosen = selection.availableTargets.flatMap(target => target.kind === "aggregate" ? [aggregates.find(model => model.id === target.aggregateId && model.available)] : []);
      const ids = chosen.flatMap(model => model ? [model.publicModelId] : []);
      if (chosen.some(model => !model) || !ids.includes(profile.request.model) || roles.some(role => profile.request.claudeModels[role] && !ids.includes(profile.request.claudeModels[role]))) {
        setError("clients.errors.profileModelsUnavailable"); return;
      }
      const restored = { ...profile.request, accessMode: "router" as const, providerId: "", providerIds: [], claudeModelIds: ids };
      setRequest(restored); setProfileName(profile.name); setPreview(null); setBusy(true); setError(null);
      try { setPreview(await previewConfig(restored, client.conflict)); }
      catch (cause) { setError(clientErrorKey(cause)); }
      finally { setBusy(false); }
      return;
    }
    const accessMode = profile.request.accessMode === "router" ? "router" : "native";
    const profileProtocols = clientProtocols(client, accessMode);
    const nextProvider = providers.find(item => item.id === profile.request.providerId && !exclusionOf(item, profileProtocols, accessMode));
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
      setPreview(await previewConfig(profileRequest, client.conflict));
    } catch (cause) {
      setError(clientErrorKey(cause));
    } finally { setBusy(false); }
  };
  const run = async () => {
    if (busy) return;
    if (aggregateUnavailable || aggregateMode && request.accessMode !== "router") { setError("clients.errors.provider"); return; }
    if ((isClaude && configuredModels.length === 0)
      || (!isClaude && selected.some(entry => modelIdsForProvider(entry).length === 0))) {
      setError("clients.errors.modelSelection"); return;
    }
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
      const configuredRequest = buildConfiguredRequest();
      // A saved level the model no longer offers falls back to its default.
      if (reasoning && configuredRequest.harnessReasoningEffort && !reasoning.levels.includes(configuredRequest.harnessReasoningEffort)) {
        configuredRequest.harnessReasoningEffort = null;
      }
      if (preview) {
        const applied = await applyConfig(configuredRequest, preview.revision, client.conflict);
        await onApplied();
        toast({
          title: t("clients.applied"),
          description: applied.warnings?.length
            ? applied.warnings.map(key => t(key)).join(" ")
            : t(reloadHint(client.targetId)),
          variant: applied.warnings?.length ? "destructive" : "success",
        });
        onClose();
      } else { setPreview(await previewConfig(configuredRequest, client.conflict)); }
    } catch (cause) {
      setError(clientErrorKey(cause)); setPreview(null); await onApplied();
    } finally { setBusy(false); }
  };

  // ---- layout: header bar, sources on the left, models on the right, a fixed footer ----
  const query = sourceQuery.trim().toLowerCase();
  const matches = (text: string) => !query || text.toLowerCase().includes(query);
  const shownProviders = providers.filter(p => (!routerMode || sources[sourceKind(p)])
    && matches(`${p.name} ${p.id} ${clientModels(p, protocols).join(" ")}`));
  const shownAggregates = routerMode && sources.aggregate
    ? aggregates.filter(entry => matches(`${entry.displayName} ${entry.publicModelId}`)) : [];
  const providerGroups = (["relay", "account"] as const)
    .map(kind => ({ kind, rows: shownProviders.filter(p => sourceKind(p) === kind) }))
    .filter(group => group.rows.length > 0);
  const defaultLabel = !modelCompatible ? null
    : claudeMixed ? mixedEntry(request.model)?.label ?? request.model
    : aggregateMode ? defaultAggregate?.displayName ?? request.model
    : provider ? `${provider.name} / ${modelLabel(provider, request.model)}` : null;
  const modelTotal = isClaude ? configuredModels.length
    : selected.reduce((sum, entry) => sum + modelIdsForProvider(entry).length, 0) + selectedAggregates.length;
  const harnessDotenv = isHarness && harnessProfiles.find(profile => profile.name === request.harnessProfile)?.dotenv;
  const groupHeading = (label: string, count: number, chosen: number) => (
    <p className="flex items-center justify-between gap-2 px-1 pb-1.5 pt-1 text-[11px] font-medium text-muted-foreground">
      <span className="poster-kicker text-[10px]">{label}</span>
      <span className="tabular-nums">{chosen ? t("clients.dialog.chosen", { count: chosen }) + " · " : ""}{count}</span>
    </p>
  );

  return (
    <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
      <DialogContent data-client-config-dialog
        className="flex max-h-[92vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-5xl md:h-[min(88vh,820px)]">
        <DialogHeader className="shrink-0 space-y-3 border-b border-border/70 px-5 pb-4 pt-5 sm:px-6">
          <div className="space-y-1 pr-8">
            <DialogTitle className="font-display text-xl tracking-tight">{remote
              ? t("remote.configureTitle", { name: client.name, host: remote.hostName })
              : t("clients.configureTitle", { name: client.name })}</DialogTitle>
            <DialogDescription>{t(remote ? "remote.directHint" : request.accessMode === "native" ? "clients.nativeHint" : "clients.localHint")}</DialogDescription>
          </div>
          <fieldset disabled={busy} className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2.5">
            {!remote && <div className="flex w-fit gap-1 rounded-lg border bg-muted/40 p-1" aria-label={t("clients.accessMode")}>
              {(["native", "router"] as const).map(mode => <Button key={mode} type="button" size="sm" className="h-7"
                variant={request.accessMode === mode ? "secondary" : "ghost"} aria-pressed={request.accessMode === mode}
                disabled={busy} onClick={() => changeMode(mode)}>{t("clients.access" + (mode === "native" ? "Native" : "Router"))}</Button>)}
            </div>}
            {routerMode && <section className="flex min-w-0 flex-wrap items-center gap-2" aria-labelledby="client-sources-label" data-client-sources>
              <p id="client-sources-label" className="text-xs font-medium text-muted-foreground">{t("clients.sources")}</p>
              <ModelSourceSwitches value={sources} disabled={busy} label={t("clients.sources")} onChange={changeSources}
                counts={{ relay: compatible.filter(entry => !entry.accountSource).length,
                  account: compatible.filter(entry => !!entry.accountSource).length, aggregate: aggregates.length }} />
            </section>}
            {isHarness && <section className="flex min-w-0 items-center gap-2 sm:ml-auto" data-harness-profile>
              <Label htmlFor="harness-profile" className="shrink-0 text-xs text-muted-foreground">{t("clients.harnessProfile")}</Label>
              <Select disabled={busy || client.configured} value={request.harnessProfile ?? ""}
                onValueChange={value => update({ harnessProfile: value })}>
                <SelectTrigger id="harness-profile" className="h-8 w-48"><SelectValue placeholder={t("clients.harnessProfilePick")} /></SelectTrigger>
                <SelectContent>{harnessProfiles.map(profile => <SelectItem key={profile.name} value={profile.name}>
                  {profile.name}{profile.name === "desktop" ? ` · ${t("clients.harnessDesktop")}` : profile.name === "web" ? " · dsh web" : profile.dotenv ? " · .env" : ""}
                </SelectItem>)}</SelectContent>
              </Select>
            </section>}
          </fieldset>
          {isHarness && <p className="text-xs text-muted-foreground">
            {t(client.configured ? "clients.harnessProfileLocked" : "clients.harnessProfileHint")}
            {harnessDotenv && <span className="mt-0.5 block text-amber-700 dark:text-amber-300" data-harness-dotenv>{t("clients.warnings.harnessDotenv")}</span>}
          </p>}
          {isClaude && !remote && <section className="space-y-2" aria-label={t("clients.savedProfiles")}>
            <div className="flex flex-wrap items-center gap-2">
              {claudeProfiles.length > 0
                ? <Label htmlFor="claude-saved-profile" className="min-w-0 text-xs font-medium text-muted-foreground">{t("clients.savedProfiles")}</Label>
                : <span className="min-w-0 flex-1 text-xs text-muted-foreground">{t("clients.noSavedProfiles")}</span>}
              {claudeProfiles.length > 0 ? <Select disabled={busy} value="" onValueChange={value => void loadClaudeProfile(value)}>
                <SelectTrigger id="claude-saved-profile" className="h-8 min-w-44 flex-1"><SelectValue placeholder={t("clients.selectSavedProfile")} /></SelectTrigger>
                <SelectContent>{claudeProfiles.map(item => <SelectItem key={item.id} value={item.id}>{item.name} · {profileTargetLabel(item)}</SelectItem>)}</SelectContent>
              </Select> : null}
              <Button type="button" size="sm" variant="ghost" className="h-8" disabled={busy || (!editingProfile && !modelCompatible)}
                aria-expanded={editingProfile} aria-controls="claude-profile-editor" onClick={() => setEditingProfile(value => !value)}>
                <BookmarkPlus className="h-4 w-4" />{editingProfile ? t("common.cancel") : t("clients.saveProfile")}
              </Button>
              {claudeProfiles.length > 0 && <Button type="button" size="sm" variant="ghost" className="h-8" disabled={busy}
                aria-expanded={managingProfiles} aria-controls="claude-profile-management" onClick={() => setManagingProfiles(value => !value)}>
                {t(managingProfiles ? "common.cancel" : "clients.manageProfiles")}
              </Button>}
            </div>
            {editingProfile && <div id="claude-profile-editor" className="flex flex-wrap gap-2 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-150">
              <Input aria-label={t("clients.profileName")} placeholder={defaultAggregate?.displayName ?? provider?.name ?? t("clients.profileName")} value={profileName}
                onChange={event => setProfileName(event.target.value)} disabled={busy || !modelCompatible} className="h-9 min-w-40 flex-1" />
              <Button type="button" size="sm" variant="outline" disabled={busy || !modelCompatible} onClick={saveClaudeProfile}>
                {t("clients.confirmSaveProfile")}
              </Button>
            </div>}
            {managingProfiles && <ul id="claude-profile-management" className="max-h-40 space-y-1 overflow-y-auto rounded-lg border bg-muted/20 p-2 motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-150">
              {claudeProfiles.map(item => <li key={item.id} className="flex min-w-0 items-center gap-2 px-1 py-1">
                <span className="min-w-0 flex-1 truncate text-sm" title={item.name}>{item.name}</span>
                <span className="max-w-[40%] truncate text-xs text-muted-foreground" title={profileTargetLabel(item)}>
                  {profileTargetLabel(item)}
                </span>
                <Button type="button" size="icon-sm" variant="ghost" disabled={busy}
                  aria-label={t("clients.deleteProfile", { name: item.name })} onClick={() => deleteClaudeProfile(item.id)}>
                  <Trash2 className="h-4 w-4" />
                </Button>
              </li>)}
            </ul>}
          </section>}
          {client.conflict && <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-sm">
            {t("clients.updateConflictDescription")}
          </p>}
        </DialogHeader>

        <div data-client-config-scroll className="min-h-0 flex-1 overflow-y-auto md:grid md:grid-cols-[minmax(0,5fr)_minmax(0,7fr)] md:overflow-hidden">
          {/* Sources: relays, account sources and aggregates, searchable. */}
          <fieldset disabled={busy} aria-labelledby="client-provider-label" data-client-config-sources
            className="min-w-0 space-y-3 border-b border-border/70 bg-muted/[0.18] p-4 md:min-h-0 md:overflow-y-auto md:border-b-0 md:border-r sm:px-5">
            <div className="flex items-center justify-between gap-2">
              <p id="client-provider-label" className="text-sm font-semibold">{t(isClaude && !routerMode ? "clients.provider" : "clients.providers")}</p>
              <span className="text-[11px] tabular-nums text-muted-foreground">{t("clients.dialog.chosen", { count: selected.length + selectedAggregates.length })}</span>
            </div>
            <div className="relative">
              <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input value={sourceQuery} onChange={event => setSourceQuery(event.target.value)} className="h-8 pl-8 text-xs"
                placeholder={t("clients.dialog.searchSources")} aria-label={t("clients.dialog.searchSources")} />
            </div>
            {providerGroups.map(group => <div key={group.kind} data-client-source-group={group.kind}>
              {routerMode && groupHeading(t(`gateway.kind.${group.kind}`), group.rows.length,
                group.rows.filter(p => request.providerIds.includes(p.id) && !exclusionOf(p, protocols, request.accessMode)).length)}
              <div className="space-y-1.5">{group.rows.map(p => {
                const reason = exclusionOf(p, protocols, request.accessMode);
                const active = request.providerIds.includes(p.id) && !reason;
                const availableCount = clientModels(p, protocols).length;
                const isDefaultSource = !aggregateMode && !claudeMixed && p.id === request.providerId && active;
                return <button key={p.id} type="button" disabled={busy || !!reason} aria-pressed={active} onClick={() => toggleProvider(p)}
                  className={"flex w-full min-w-0 items-center gap-3 rounded-lg border px-3 py-2.5 text-left transition-[border-color,background-color,transform] duration-150 motion-safe:active:scale-[0.99] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45 "
                    + (active ? "border-primary/50 bg-primary/[0.06]" : "border-border/70 bg-card/70 hover:bg-muted/60")}>
                  <span className={"flex h-4 w-4 shrink-0 items-center justify-center border " + (isClaude && !routerMode ? "rounded-full " : "rounded ")
                    + (active ? "border-primary bg-primary text-primary-foreground" : "border-input")}>
                    {active && <Check className="h-3 w-3" />}</span>
                  <span className="min-w-0 flex-1"><span className="flex min-w-0 items-center gap-1.5">
                      <span className="truncate text-sm font-medium" title={p.name}>{p.name}</span>
                      {p.accountSource && <span data-account-source-badge className="shrink-0 rounded-full border border-primary/25 bg-primary/5 px-1.5 text-[10px] font-medium text-primary">{t("accountSources.badge")}</span>}
                      {isDefaultSource && <Star aria-hidden className="h-3 w-3 shrink-0 fill-amber-400 text-amber-500" />}
                    </span>
                    <span className="mt-0.5 block text-xs text-muted-foreground">{reason ? t("clients.excluded." + reason, { count: 1 })
                      : active && !isClaude ? t("clients.selectedOfAvailable", { selected: modelIdsForProvider(p).length, total: availableCount })
                      : t("clients.modelsCount", { count: availableCount })}</span>
                    {/* WorkBuddy's channel check is met by rewording the phrases it flags; Antigravity's is not. */}
                    {!reason && isClaude && p.accountSource === "antigravity" && (
                      <span className="mt-1 block text-[11px] leading-snug text-amber-700 dark:text-amber-300" data-account-source-refusal>
                        {t("accountSources.refusesClaudeCode", { name: "Antigravity" })}</span>
                    )}</span>
                </button>;
              })}</div>
            </div>)}
            {shownAggregates.length > 0 && <section aria-label={t("aggregation.title")} data-client-source-group="aggregate">
              {groupHeading(t("gateway.kind.aggregate"), shownAggregates.length, aggregateIds.length)}
              <div className="space-y-1.5">{shownAggregates.map(entry => <label key={entry.id}
                className={"flex min-w-0 cursor-pointer items-center gap-3 rounded-lg border px-3 py-2.5 transition-colors "
                  + (aggregateIds.includes(entry.id) ? "border-primary/50 bg-primary/[0.06]" : "border-border/70 bg-card/70 hover:bg-muted/60")}>
                <input type="checkbox" checked={aggregateIds.includes(entry.id)} disabled={busy || !entry.available && !aggregateIds.includes(entry.id)}
                  aria-label={t("clients.includeModel", { model: entry.displayName })} onChange={() => toggleAggregate(entry.id)} className="h-4 w-4 shrink-0 accent-primary" />
                <span className="min-w-0 flex-1"><span className="flex min-w-0 items-center gap-1.5">
                    <span className="truncate text-sm font-medium">{entry.displayName}</span>
                    {aggregateMode && defaultTarget?.kind === "aggregate" && defaultTarget.aggregateId === entry.id
                      && <Star aria-hidden className="h-3 w-3 shrink-0 fill-amber-400 text-amber-500" />}
                  </span>
                  <span className="block break-all font-mono text-[11px] text-muted-foreground">{entry.publicModelId}</span></span>
              </label>)}</div>
              {aggregateUnavailable && <p role="alert" className="mt-1.5 text-xs text-destructive">{t("aggregation.targetUnavailable")}</p>}
              {!isClaude && aggregateIds.length > 0 && <div className="mt-2.5 space-y-1.5"><Label htmlFor="client-aggregate-protocol" className="text-xs">{t("aggregation.ingress")}</Label>
                <Select value={request.aggregateIngressProtocol ?? "responses"} onValueChange={value => update({ aggregateIngressProtocol: value as ClientConfigRequest["aggregateIngressProtocol"] })}>
                  <SelectTrigger id="client-aggregate-protocol" className="h-8"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="responses">Responses</SelectItem>
                    <SelectItem value="chatCompletions">Chat</SelectItem>
                    <SelectItem value="anthropicMessages">Messages</SelectItem></SelectContent></Select>
              </div>}
            </section>}
            {providerGroups.length === 0 && shownAggregates.length === 0 && <p className="rounded-lg border border-dashed px-3 py-4 text-center text-xs text-muted-foreground">
              {t(query ? "clients.dialog.noSourceMatch" : isClaude && request.accessMode === "native" ? "clients.noNativeProviders" : "clients.noProviders")}
            </p>}
            {compatible.length === 0 && providerGroups.length > 0 && <p className="text-xs text-muted-foreground">{t(isClaude && request.accessMode === "native" ? "clients.noNativeProviders" : "clients.noProviders")}</p>}
          </fieldset>

          {/* Models: the default first, then which models each source writes. */}
          <div className="min-w-0 space-y-5 p-4 md:min-h-0 md:overflow-y-auto sm:px-5" data-client-config-models>
            <fieldset disabled={busy} className="min-w-0 space-y-5">
              {request.accessMode === "native" && !remote && <DirectProxyNote name={client.name} isClaude={isClaude} provider={provider} />}
              <section className="space-y-1.5 rounded-xl border border-primary/25 bg-primary/[0.04] p-3.5" aria-labelledby="client-model-label">
                <div className="flex items-center justify-between gap-2"><Label id="client-model-label" htmlFor="client-model" className="flex items-center gap-1.5">
                  <Star aria-hidden className="h-3.5 w-3.5 fill-amber-400 text-amber-500" />
                  {t(isClaude ? "clients.startupModel" : client.targetId === "pi" ? "clients.piModel" : "clients.model")}</Label>
                  {selectedProtocol && <Badge variant="outline" className="text-[10px]">{t("newapi.protocol" + (selectedProtocol === "responses" ? "Responses" : selectedProtocol === "chatCompletions" ? "ChatCompletions" : "AnthropicMessages"))}</Badge>}
                </div>
                <Select disabled={busy || selected.length + selectedAggregates.length === 0} value={!modelCompatible ? "" : claudeMixed ? JSON.stringify(["mixed:", request.model])
                  : JSON.stringify([aggregateMode && defaultTarget?.kind === "aggregate" ? "aggregate:" + defaultTarget.aggregateId : request.providerId, request.model])}
                  onValueChange={value => { const [id, model] = JSON.parse(value) as [string, string];
                    if (id === "mixed:") {
                      const target = mixedEntry(model)?.target;
                      update({ model, providerId: target?.kind === "provider" ? target.providerId : "" });
                    } else if (id.startsWith("aggregate:")) chooseAggregate(id.slice("aggregate:".length));
                    else update(chooseDefault(selected.find(p => p.id === id), model)); }}>
                  <SelectTrigger id="client-model" className="bg-card"><SelectValue placeholder={t("clients.selectModel")} /></SelectTrigger>
                  <SelectContent>{claudeMixed ? configuredModels.map(id => <SelectItem key={id} value={JSON.stringify(["mixed:", id])}>
                    {mixedEntry(id)?.label ?? id}
                  </SelectItem>) : selected.flatMap(p => (isClaude ? configuredModels : modelIdsForProvider(p)).map(id => <SelectItem key={JSON.stringify([p.id, id])} value={JSON.stringify([p.id, id])}>
                    {(!isClaude && selected.length > 1 ? p.name + " / " : "") + modelLabel(p, id)}
                  </SelectItem>))}
                    {!claudeMixed && selectedAggregates.map(model => <SelectItem key={model.id} value={JSON.stringify(["aggregate:" + model.id, model.publicModelId])}>
                      {t("aggregation.target")} / {model.displayName}
                    </SelectItem>)}
                  </SelectContent>
                </Select>
                {isHarness && <label className="flex items-center gap-2 pt-1 text-xs text-muted-foreground" data-harness-default-model>
                  <input type="checkbox" checked={request.harnessDefaultModel !== false} disabled={busy}
                    className="h-4 w-4 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                    onChange={event => update({ harnessDefaultModel: event.target.checked })} />
                  {t("clients.harnessDefaultModel")}
                </label>}
                {isHarness && !!reasoning?.levels.length && <div className="flex items-center justify-between gap-3 pt-1" data-harness-reasoning>
                  <Label htmlFor="client-harness-reasoning" className="text-xs font-normal text-muted-foreground">{t("clients.harnessReasoning")}</Label>
                  <Select disabled={busy} value={request.harnessReasoningEffort && reasoning.levels.includes(request.harnessReasoningEffort) ? request.harnessReasoningEffort : "auto"}
                    onValueChange={value => update({ harnessReasoningEffort: value === "auto" ? null : value })}>
                    <SelectTrigger id="client-harness-reasoning" className="h-8 w-44 bg-card"><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">{reasoning.modelDefault
                        ? t("clients.harnessReasoningDefault", { level: reasoning.modelDefault }) : t("clients.harnessReasoningProvider")}</SelectItem>
                      {reasoning.levels.map(level => <SelectItem key={level} value={level}>{level}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>}
                {isClaude && <label className="flex items-center gap-2 pt-1 text-xs text-muted-foreground"
                  title={startupOneMUnavailable ? t("clients.oneMUnavailable") : undefined}>
                  <input type="checkbox" checked={!startupOneMUnavailable && (request.claudeOneM?.startup ?? false)}
                    disabled={busy || !request.model || startupOneMUnavailable}
                    className="h-4 w-4 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50"
                    onChange={event => update({ claudeOneM: { ...claudeOneMDefaults(request.claudeOneM), startup: event.target.checked } })} />
                  {t("clients.oneMStartup")}
                </label>}
              </section>

              {!isClaude && selected.length > 0 && <section className="space-y-2" aria-label={t("clients.modelSubset")}>
                <p className="text-sm font-semibold">{t("clients.modelSubset")}</p>
                <div className="space-y-2">{selected.map(entry => {
                  const allModels = clientModels(entry, protocols);
                  const checkedModels = modelIdsForProvider(entry);
                  return <details key={entry.id} className="group overflow-hidden rounded-lg border bg-card/60">
                    <summary className="flex min-h-10 cursor-pointer list-none items-center justify-between gap-3 px-3 py-2 text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
                      <span className="min-w-0 truncate">{entry.name}</span>
                      <span className="ml-auto shrink-0 text-xs font-normal tabular-nums text-muted-foreground">{t("clients.selectedOfAvailable", { selected: checkedModels.length, total: allModels.length })}</span>
                      <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200 group-open:rotate-180 motion-reduce:transition-none" />
                    </summary>
                    <div className="flex flex-wrap items-center justify-end gap-1 border-t px-2 py-1.5">
                      <Button type="button" size="sm" variant="ghost" disabled={busy || checkedModels.length === allModels.length}
                        onClick={() => selectOpenCodeModels(entry, allModels)}>{t("clients.selectAllModels")}</Button>
                      <Button type="button" size="sm" variant="ghost" disabled={busy || checkedModels.length === 0}
                        onClick={() => selectOpenCodeModels(entry, [])}>{t("clients.deselectAllModels")}</Button>
                    </div>
                    <div className="max-h-56 divide-y overflow-y-auto border-t motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-150">
                      {allModels.map(id => {
                        const checked = checkedModels.includes(id);
                        const isDefault = !aggregateMode && entry.id === request.providerId && id === request.model;
                        return <div key={id} className="flex min-w-0 items-center gap-3 px-3 py-1.5 transition-colors hover:bg-muted/40">
                          <label className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 py-0.5">
                            <input type="checkbox" aria-label={t("clients.includeModel", { model: modelLabel(entry, id) })}
                              checked={checked} disabled={busy}
                              className="h-4 w-4 shrink-0 accent-primary focus-visible:ring-2 focus-visible:ring-ring" onChange={() => selectOpenCodeModels(entry,
                                checked ? checkedModels.filter(model => model !== id) : [...checkedModels, id])} />
                            <span className="min-w-0 flex-1 break-words text-sm">{modelLabel(entry, id)}</span>
                            {id === entry.model && <Badge variant="outline" className="shrink-0 text-[10px]">{t("clients.providerDefault")}</Badge>}
                          </label>
                          <button type="button" aria-pressed={isDefault} disabled={busy || !checked}
                            aria-label={t("clients.dialog.setDefault", { model: modelLabel(entry, id) })} title={t("clients.dialog.setDefault", { model: modelLabel(entry, id) })}
                            onClick={() => update(chooseDefault(entry, id))}
                            className="rounded-md p-1 text-muted-foreground/60 transition-colors hover:text-amber-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30 disabled:hover:text-muted-foreground/60">
                            <Star className={"h-3.5 w-3.5 " + (isDefault ? "fill-amber-400 text-amber-500" : "")} />
                          </button>
                        </div>;
                      })}
                    </div>
                  </details>;
                })}</div>
              </section>}
              {isClaude && !aggregateMode && <ClaudeModelConfig key={claudeMixed ? "mixed" : request.providerId + ":" + request.accessMode}
                provider={provider} models={models} selected={configuredModels} mapping={request.claudeModels}
                labelOf={claudeMixed ? id => mixedEntry(id)?.label ?? id : undefined}
                smallContext={claudeMixed ? id => !!mixedEntry(id)?.small : undefined}
                defaultModel={request.model} disabled={busy} oneM={claudeOneMDefaults(request.claudeOneM)}
                clientVersion={client.clientVersion}
                onSelectedChange={selectClaudeModels} onMappingChange={claudeModels => update({ claudeModels })}
                onOneMChange={claudeOneM => update({ claudeOneM })} />}
              {isClaude && aggregateMode && <div className="grid gap-3 sm:grid-cols-2" data-aggregate-claude-roles>{roles.map(role => {
                const id = `aggregate-claude-${role}`;
                const chosen = request.claudeModels[role] || request.model;
                const oneM = claudeOneMDefaults(request.claudeOneM);
                return <div key={role} className="space-y-1.5"><Label htmlFor={id}>{role}</Label>
                  <Select value={request.claudeModels[role] || "__default__"} onValueChange={value => update({ claudeModels: { ...request.claudeModels, [role]: value === "__default__" ? "" : value } })}>
                    <SelectTrigger id={id}><SelectValue /></SelectTrigger><SelectContent><SelectItem value="__default__">{t("aggregation.followDefault")}</SelectItem>
                      {selectedAggregates.map(model => <SelectItem key={model.id} value={model.publicModelId}>{model.displayName}</SelectItem>)}
                    </SelectContent></Select>
                  <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={oneM[role]} disabled={!oneM[role] && !aggregateOneM(chosen)}
                    onChange={event => update({ claudeOneM: { ...oneM, [role]: event.target.checked } })} />1M</label>
                </div>;
              })}</div>}
              {!isClaude && staleOpenCodeModelSelection &&
                <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">
                  {t("clients.staleOpenCodeModels")}
                </p>}
              {isClaude && staleClaudeModelSelection && (() => {
                const stale = (request.claudeModelIds ?? []).filter(id => !models.includes(id));
                return <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">
                  {t("clients.staleClaudeModels", { models: stale.map(id => provider ? modelLabel(provider, id) : id).join(" · ") })}
                </p>;
              })()}
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
            {preview && <div ref={previewRef} className="space-y-2" data-client-config-preview>
              <p className="text-sm font-semibold">{t("clients.previewTitle")}</p>
              {!!preview.warnings?.length && <ul className="space-y-1">
                {preview.warnings.map(key => <li key={key} role="status"
                  className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">{t(key)}</li>)}
              </ul>}
              <pre className="max-h-72 overflow-auto rounded-md bg-muted p-3 text-xs">{preview.content}</pre>
              {!!preview.removedFields?.length && <p className="break-all text-xs text-muted-foreground">{t("clients.removedFields", { fields: preview.removedFields.join(", ") })}</p>}
              {!!preview.conflictFields?.length && <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
                {t("clients.conflictScope", { fields: conflictScopes(preview.conflictFields).map(scope => t("clients.conflictScopes." + scope)).join(" · ") })}
              </p>}
              <p className="text-xs text-muted-foreground">{t("clients.backupHint")}</p>
            </div>}
          </div>
        </div>

        {error && <p role="alert" className="shrink-0 border-t border-destructive/20 bg-destructive/5 px-5 py-2 text-sm text-destructive sm:px-6">{t(error)}</p>}
        <DialogFooter className="shrink-0 flex-row flex-wrap items-center gap-2 border-t border-border/70 bg-muted/[0.18] px-5 py-3 sm:justify-between sm:space-x-0 sm:px-6">
          <p className="flex min-w-0 flex-1 basis-56 items-center gap-2 text-xs text-muted-foreground" data-client-config-summary>
            <Star aria-hidden className="h-3.5 w-3.5 shrink-0 fill-amber-400 text-amber-500" />
            <span className="min-w-0 truncate"><span className="font-medium text-foreground">{defaultLabel ?? t("clients.dialog.noDefault")}</span>
              <span className="ml-2 tabular-nums">{t("clients.dialog.summaryCount", { models: modelTotal, sources: selected.length + selectedAggregates.length })}</span></span>
          </p>
          <div className="ml-auto flex shrink-0 gap-2">
            <Button variant="outline" onClick={onClose} disabled={busy}>{t("common.cancel")}</Button>
            <Button disabled={busy || !modelCompatible || aggregateUnavailable || selected.length + selectedAggregates.length === 0
              || (isHarness && !request.harnessProfile)} onClick={() => void run()}>
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}{t(preview ? client.conflict ? "clients.applyConflict" : "clients.apply" : "clients.preview")}
            </Button>
          </div>
        </DialogFooter>
        <ConfirmDialog request={confirmRequest} onClose={() => setConfirmRequest(null)} />
      </DialogContent>
    </Dialog>
  );
}
