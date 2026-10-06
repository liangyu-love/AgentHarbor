import { useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Copy, Edit2, Plus, Save, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { aggregationQueryKey, type AggregateMember, type AggregateModel } from "@/types/aggregation";
import type { CompressionPolicy, ModelProtocol, NewApiProvider } from "@/types/newapi";

const initialModel = (): AggregateModel => ({ id: "", publicModelId: "", displayName: "", enabled: true,
  members: [], strategy: "roundRobin", affinity: "session", retryPolicy: { maxAttempts: 2 }, compression: { mode: "inherit" } });
const modelsOf = (provider: NewApiProvider | undefined) => provider
  ? provider.models.filter(model => model.included).map(model => model.id).concat(
    !provider.models.some(model => model.id === provider.model) && provider.model ? [provider.model] : []) : [];

export function AggregationManager({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const cache = useQueryClient();
  const query = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const [draft, setDraft] = useState<AggregateModel | null>(null);
  const capabilities = useQuery({ queryKey: ["model-capabilities"], queryFn: api.routing.capabilities, enabled: draft !== null });
  const [busy, setBusy] = useState(false);
  const mutationInFlight = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const [exportId, setExportId] = useState("");
  const [ingressProtocol, setIngressProtocol] = useState<ModelProtocol>("responses");
  const [consumerProfile, setConsumerProfile] = useState<"generic" | "anthropicSdk" | "opencode">("generic");
  const [configText, setConfigText] = useState<string | null>(null);
  const models = query.data?.models ?? [];
  const available = models.filter(model => model.available);
  const selectedExport = exportId ? available.find(model => model.id === exportId) : available[0];
  const references = [...providers.flatMap(provider => provider.models.flatMap(model => model.catalogRef ? [model.catalogRef] : [])),
    ...(capabilities.data?.models.flatMap(model => model.matched ? [model.matched] : []) ?? [])];
  const suggestions = [...new Set(references.map(reference => reference.modelId))];
  const changed = async () => {
    await Promise.all([cache.invalidateQueries({ queryKey: aggregationQueryKey }),
      cache.invalidateQueries({ queryKey: queryKeys.providers }), cache.invalidateQueries({ queryKey: queryKeys.clientConfigs }),
      cache.invalidateQueries({ queryKey: queryKeys.claudeDesktopCandidates }),
      cache.invalidateQueries({ queryKey: queryKeys.claudeDesktopStatus })]);
    setConfigText(null);
  };
  const run = async (action: () => Promise<unknown>) => {
    if (mutationInFlight.current) return;
    mutationInFlight.current = true;
    setBusy(true); setError(null);
    try { await action(); } catch (cause) { setError(formatInvokeError(cause)); }
    finally { mutationInFlight.current = false; setBusy(false); }
  };
  const update = (patch: Partial<AggregateModel>) => setDraft(current => current ? { ...current, ...patch } : current);
  const memberUpdate = (id: string, patch: Partial<AggregateMember>) => {
    if (draft) update({ members: draft.members.map(member => member.id === id ? { ...member, ...patch } : member) });
  };
  const addMember = () => {
    if (!draft) return;
    const provider = providers.find(provider => modelsOf(provider).length > 0);
    update({ members: [...draft.members, { id: crypto.randomUUID(), providerId: provider?.id ?? "",
      actualModelId: modelsOf(provider)[0] ?? "", enabled: true, priority: 0, weight: 1 }] });
  };
  const save = () => void run(async () => {
    if (!draft) return;
    if (!draft.publicModelId.trim() || draft.members.length === 0 || draft.members.some(member =>
      !member.providerId || !member.actualModelId || !Number.isInteger(member.priority) || member.priority < 0 || member.priority > 1000
      || !Number.isInteger(member.weight) || member.weight < 1 || member.weight > 1000)) {
      setError(t("aggregation.formInvalid")); return;
    }
    await api.aggregation.save(draft); setDraft(null); await changed();
    toast({ title: t("aggregation.saved"), variant: "success" });
  });
  const connection = (revealCredential: boolean) => api.aggregation.exportConnection({ ingressProtocol,
    aggregateIds: available.map(model => model.id), selectedAggregateId: selectedExport?.id ?? null, consumerProfile, revealCredential });

  return <section className="space-y-5" aria-labelledby="aggregation-title">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div><h2 id="aggregation-title" className="text-lg font-semibold">{t("aggregation.title")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("aggregation.description")}</p></div>
      <Button disabled={busy || draft !== null || models.length >= 128} onClick={() => { setDraft(initialModel()); setError(null); }}>
        <Plus className="h-4 w-4" />{t("aggregation.add")}</Button>
    </div>
    {(error || query.error) && <p role="alert" className="text-sm text-destructive">{error ?? formatInvokeError(query.error)}</p>}
    {query.isPending && <p role="status" className="text-sm text-muted-foreground">{t("common.loading")}</p>}
    {draft && <form className="space-y-4 border-y py-4" onSubmit={event => { event.preventDefault(); save(); }}>
      <fieldset disabled={busy} className="min-w-0 space-y-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="space-y-1.5"><Label htmlFor="aggregate-public-id">{t("aggregation.publicId")}</Label>
            <Input id="aggregate-public-id" list="aggregate-official-models" value={draft.publicModelId} disabled={!!draft.id}
              onChange={event => {
                const matching = references.filter(reference => reference.modelId === event.target.value);
                const providers = new Set(matching.map(reference => reference.providerId));
                update({ publicModelId: event.target.value, canonicalRef: providers.size === 1 ? matching[0] : null });
              }} maxLength={256} required />
            <datalist id="aggregate-official-models">{suggestions.map(id => <option key={id} value={id} />)}</datalist></div>
          <div className="space-y-1.5"><Label htmlFor="aggregate-display-name">{t("aggregation.displayName")}</Label>
            <Input id="aggregate-display-name" value={draft.displayName} onChange={event => update({ displayName: event.target.value })} maxLength={200} /></div>
          <div className="space-y-1.5"><Label htmlFor="aggregate-strategy">{t("aggregation.strategy")}</Label>
            <Select value={draft.strategy} onValueChange={value => update({ strategy: value as AggregateModel["strategy"] })}>
              <SelectTrigger id="aggregate-strategy"><SelectValue /></SelectTrigger><SelectContent>
                <SelectItem value="roundRobin">{t("aggregation.roundRobin")}</SelectItem>
                <SelectItem value="weightedRoundRobin">{t("aggregation.weightedRoundRobin")}</SelectItem>
              </SelectContent></Select></div>
          <div className="space-y-1.5"><Label htmlFor="aggregate-affinity">{t("aggregation.affinity")}</Label>
            <Select value={draft.affinity} onValueChange={value => update({ affinity: value as AggregateModel["affinity"] })}>
              <SelectTrigger id="aggregate-affinity"><SelectValue /></SelectTrigger><SelectContent>
                <SelectItem value="session">{t("aggregation.session")}</SelectItem>
                <SelectItem value="statelessRotation">{t("aggregation.statelessRotation")}</SelectItem>
              </SelectContent></Select></div>
          <div className="space-y-1.5"><Label htmlFor="aggregate-attempts">{t("aggregation.retryAttempts")}</Label>
            <Select value={String(draft.retryPolicy.maxAttempts)} onValueChange={value => update({ retryPolicy: { maxAttempts: Number(value) } })}>
              <SelectTrigger id="aggregate-attempts"><SelectValue /></SelectTrigger><SelectContent>
                <SelectItem value="1">1</SelectItem><SelectItem value="2">2</SelectItem>
              </SelectContent></Select></div>
          <div className="space-y-1.5"><Label htmlFor="aggregate-compression">{t("newapi.compression.modelLevel")}</Label>
            <div className="flex gap-2"><Select value={draft.compression.mode} onValueChange={value => {
              const mode = value as CompressionPolicy["mode"];
              update({ compression: mode === "percent" ? { mode, percent: 90 } : mode === "tokens" ? { mode, tokens: 100_000 } : { mode } });
            }}>
              <SelectTrigger id="aggregate-compression"><SelectValue /></SelectTrigger><SelectContent>
                {(["inherit", "percent", "tokens", "clientDefault"] as const).map(mode => <SelectItem key={mode} value={mode}>{t("newapi.compression." + mode)}</SelectItem>)}
              </SelectContent></Select>
              {(draft.compression.mode === "percent" || draft.compression.mode === "tokens") && <Input
                type="number" min={1} max={draft.compression.mode === "percent" ? 99 : 10_000_000} className="w-28 shrink-0"
                aria-label={t("newapi.compression.modelLevel")} value={draft.compression.mode === "percent" ? draft.compression.percent : draft.compression.tokens}
                onChange={event => update({ compression: draft.compression.mode === "percent"
                  ? { mode: "percent", percent: Number(event.target.value) } : { mode: "tokens", tokens: Number(event.target.value) } })} />}
            </div></div>
        </div>
        <div className="flex items-center justify-between gap-3"><h3 className="text-sm font-medium">{t("aggregation.members")}</h3>
          <Button type="button" variant="outline" size="sm" disabled={draft.members.length >= 64 || providers.length === 0} onClick={addMember}>
            <Plus className="h-4 w-4" />{t("aggregation.addMember")}</Button></div>
        {draft.members.map((member, index) => {
          const provider = providers.find(provider => provider.id === member.providerId);
          const providerModels = modelsOf(provider);
          return <div key={member.id} className="space-y-2 border-b pb-3" data-aggregate-member>
            <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_5rem_5rem_auto]">
              <div className="min-w-0 space-y-1"><Label htmlFor={`member-provider-${member.id}`}>{t("aggregation.provider")}</Label>
                <Select value={member.providerId} onValueChange={value => { const next = providers.find(provider => provider.id === value);
                  memberUpdate(member.id, { providerId: value, actualModelId: modelsOf(next)[0] ?? "" }); }}>
                  <SelectTrigger id={`member-provider-${member.id}`}><SelectValue placeholder={t("aggregation.selectProvider")} /></SelectTrigger>
                  <SelectContent>{!provider && member.providerId && <SelectItem value={member.providerId} disabled>{member.providerId} · {t("aggregation.unavailable")}</SelectItem>}
                    {providers.map(provider => <SelectItem key={provider.id} value={provider.id}>{provider.name}</SelectItem>)}</SelectContent></Select></div>
              <div className="min-w-0 space-y-1"><Label htmlFor={`member-model-${member.id}`}>{t("aggregation.actualId")}</Label>
                <Select value={member.actualModelId} onValueChange={value => memberUpdate(member.id, { actualModelId: value })}>
                  <SelectTrigger id={`member-model-${member.id}`}><SelectValue placeholder={t("aggregation.selectModel")} /></SelectTrigger><SelectContent>
                    {member.actualModelId && !providerModels.includes(member.actualModelId) && <SelectItem value={member.actualModelId} disabled>{member.actualModelId} · {t("aggregation.unavailable")}</SelectItem>}
                    {providerModels.map(id => <SelectItem key={id} value={id}>{id}</SelectItem>)}</SelectContent></Select></div>
              <div className="space-y-1"><Label htmlFor={`member-priority-${member.id}`}>{t("aggregation.priority")}</Label>
                <Input id={`member-priority-${member.id}`} type="number" min={0} max={1000} value={member.priority} onChange={event => memberUpdate(member.id, { priority: Number(event.target.value) })} /></div>
              <div className="space-y-1"><Label htmlFor={`member-weight-${member.id}`}>{t("aggregation.weight")}</Label>
                <Input id={`member-weight-${member.id}`} type="number" min={1} max={1000} value={member.weight} onChange={event => memberUpdate(member.id, { weight: Number(event.target.value) })} /></div>
              <Button type="button" variant="ghost" size="icon" className="self-end" aria-label={t("aggregation.removeMember", { index: index + 1 })}
                onClick={() => update({ members: draft.members.filter(entry => entry.id !== member.id) })}><Trash2 className="h-4 w-4" /></Button>
            </div>
            <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={member.enabled} onChange={event => memberUpdate(member.id, { enabled: event.target.checked })} />{t("aggregation.memberEnabled")}</label>
          </div>;
        })}
        {draft.members.length === 0 && <p className="text-sm text-muted-foreground">{t(providers.length ? "aggregation.addMembersHint" : "aggregation.noProviders")}</p>}
        <label className="flex items-center gap-2 text-sm"><input type="checkbox" checked={draft.enabled} onChange={event => update({ enabled: event.target.checked })} />{t("aggregation.enabled")}</label>
      </fieldset>
      <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={busy} onClick={() => setDraft(null)}>{t("common.cancel")}</Button>
        <Button type="submit" disabled={busy}><Save className="h-4 w-4" />{t("common.save")}</Button></div>
    </form>}
    {!query.isPending && models.length === 0 && !draft && <p role="status" className="py-4 text-sm text-muted-foreground">{t("aggregation.empty")}</p>}
    <div className="divide-y">{models.map(model => <div key={model.id} className="flex flex-wrap items-center justify-between gap-3 py-3">
      <div className="min-w-0 flex-1"><div className="flex flex-wrap items-center gap-2"><h3 className="font-medium">{model.displayName}</h3>
        <Badge variant="outline">{t(model.available ? "aggregation.available" : "aggregation.unavailable")}</Badge></div>
        <p className="mt-1 break-all text-sm text-muted-foreground">{model.publicModelId} · {t("aggregation.memberCount", { count: model.members.length })}</p>
        {model.reason && <p className="mt-1 text-xs text-muted-foreground">{t(`aggregation.reasons.${model.reason}`, { defaultValue: t("aggregation.unavailable") })}</p>}
        <details className="mt-2 text-xs"><summary className="cursor-pointer text-muted-foreground">{t("aggregation.members")}</summary>
          <ul className="mt-2 space-y-1">{model.members.map(member => {
            const status = model.memberStates.find(state => state.memberId === member.id);
            return <li key={member.id} className="flex flex-wrap gap-x-2 gap-y-1 break-all">
              <span>{status?.providerName || member.providerId} / {member.actualModelId}</span>
              <span className="text-muted-foreground">{status?.protocol === "responses" ? "Responses" : status?.protocol === "chatCompletions" ? "Chat" : status?.protocol === "anthropicMessages" ? "Messages" : "—"}
                {` · ${t("aggregation.priority")} ${member.priority} · ${t("aggregation.weight")} ${member.weight}`}</span>
              {!status?.available && <span className="text-destructive">{t(`aggregation.reasons.${status?.reason}`, { defaultValue: t("aggregation.unavailable") })}</span>}
            </li>;
          })}</ul>
        </details></div>
      <div className="flex gap-2"><Button variant="outline" size="sm" disabled={busy || draft !== null} onClick={() => setDraft({ ...model, members: model.members.map(member => ({ ...member })) })}>
        <Edit2 className="h-4 w-4" />{t("common.edit")}</Button>
        <Button variant="ghost" size="sm" disabled={busy} onClick={() => setConfirm({ title: t("aggregation.deleteTitle"), description: t("aggregation.deleteDescription", { name: model.displayName }),
          confirmLabel: t("common.delete"), destructive: true, onConfirm: async () => { await run(async () => { await api.aggregation.delete(model.id); await changed(); }); } })}>
          <Trash2 className="h-4 w-4" />{t("common.delete")}</Button></div>
    </div>)}</div>
    <div className="space-y-3 border-t pt-5" aria-labelledby="aggregate-export-title">
      <h3 id="aggregate-export-title" className="font-medium">{t("aggregation.connection")}</h3>
      <div className="flex flex-wrap items-center justify-between gap-3"><Label htmlFor="aggregate-external-enabled">{t("aggregation.keepService")}</Label>
        <Switch id="aggregate-external-enabled" checked={query.data?.externalEnabled ?? false} disabled={busy || !query.isSuccess}
          onCheckedChange={value => void run(async () => { await api.aggregation.setExternalEnabled(value); await changed(); })} /></div>
      <p className="text-xs text-muted-foreground">{t(query.data?.listenerReady ? "aggregation.listenerReady" : "aggregation.listenerStopped")}</p>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="space-y-1"><Label htmlFor="aggregate-export-model">{t("aggregation.defaultModel")}</Label><Select value={selectedExport?.id ?? ""} onValueChange={value => { setExportId(value); setConfigText(null); }}>
          <SelectTrigger id="aggregate-export-model"><SelectValue /></SelectTrigger><SelectContent>
            {exportId && !selectedExport && <SelectItem value={exportId} disabled>{t("aggregation.unavailable")}</SelectItem>}
            {available.map(model => <SelectItem key={model.id} value={model.id}>{model.displayName}</SelectItem>)}</SelectContent></Select></div>
        <div className="space-y-1"><Label htmlFor="aggregate-export-protocol">{t("aggregation.ingress")}</Label><Select value={ingressProtocol} onValueChange={value => { setIngressProtocol(value as ModelProtocol); setConfigText(null); }}>
          <SelectTrigger id="aggregate-export-protocol"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="responses" disabled={consumerProfile === "anthropicSdk"}>Responses</SelectItem><SelectItem value="anthropicMessages">Messages</SelectItem></SelectContent></Select></div>
        <div className="space-y-1"><Label htmlFor="aggregate-export-profile">{t("aggregation.template")}</Label><Select value={consumerProfile} onValueChange={value => {
          setConsumerProfile(value as typeof consumerProfile); if (value === "anthropicSdk") setIngressProtocol("anthropicMessages"); setConfigText(null);
        }}>
          <SelectTrigger id="aggregate-export-profile"><SelectValue /></SelectTrigger><SelectContent><SelectItem value="generic">{t("aggregation.generic")}</SelectItem><SelectItem value="anthropicSdk">Anthropic SDK</SelectItem><SelectItem value="opencode">OpenCode</SelectItem></SelectContent></Select></div>
      </div>
      <div className="flex flex-wrap gap-2"><Button variant="outline" disabled={busy || !selectedExport} onClick={() => void run(async () => setConfigText((await connection(false)).configText))}>{t("aggregation.previewConnection")}</Button>
        <Button disabled={busy || !selectedExport} onClick={() => void run(async () => { const result = await connection(true);
          await navigator.clipboard.writeText(result.configText); toast({ title: t("aggregation.copied"), variant: "success" }); })}><Copy className="h-4 w-4" />{t("aggregation.copyConnection")}</Button></div>
      {configText !== null && <pre className="max-h-72 overflow-auto rounded-md bg-muted p-3 text-xs" data-aggregate-export-preview>{configText}</pre>}
    </div>
    <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
  </section>;
}
