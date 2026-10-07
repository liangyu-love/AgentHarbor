import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  Check,
  Copy,
  Cpu,
  Edit2,
  GitBranch,
  Layers,
  Network,
  Plus,
  RefreshCw,
  Save,
  Share2,
  Sliders,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { useFold } from "@/hooks/use-fold";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { aggregationQueryKey, type AggregateMember, type AggregateModel } from "@/types/aggregation";
import type { CompressionPolicy, ModelProtocol, NewApiProvider } from "@/types/newapi";
import { cn } from "@/lib/utils";

const initialModel = (): AggregateModel => ({
  id: "",
  publicModelId: "",
  displayName: "",
  enabled: true,
  members: [],
  strategy: "roundRobin",
  affinity: "session",
  retryPolicy: { maxAttempts: 2, failureThreshold: 3 },
  compression: { mode: "inherit" },
});

const modelsOf = (provider: NewApiProvider | undefined) =>
  provider
    ? provider.models
        .filter((model) => model.included)
        .map((model) => model.id)
        .concat(
          !provider.models.some((model) => model.id === provider.model) && provider.model ? [provider.model] : [],
        )
    : [];

export function AggregationManager({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const cache = useQueryClient();
  const query = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const [draft, setDraft] = useState<AggregateModel | null>(null);
  const capabilities = useQuery({
    queryKey: ["model-capabilities"],
    queryFn: api.routing.capabilities,
    enabled: draft !== null,
  });
  const [busy, setBusy] = useState(false);
  const mutationInFlight = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const [exportId, setExportId] = useState("");
  const [ingressProtocol, setIngressProtocol] = useState<ModelProtocol>("responses");
  const [consumerProfile, setConsumerProfile] = useState<"generic" | "anthropicSdk" | "opencode">("generic");
  const [configText, setConfigText] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const fold = useFold("aggregation");
  const gatewayFold = useFold("aggregation-gateway");

  useEffect(() => {
    if (copiedId === null) return;
    const timer = window.setTimeout(() => setCopiedId(null), 2000);
    return () => window.clearTimeout(timer);
  }, [copiedId]);

  const models = query.data?.models ?? [];
  const available = models.filter((model) => model.available);
  const selectedExport = exportId ? available.find((model) => model.id === exportId) : available[0];

  const referenceSources = [
    ...providers.flatMap((provider) =>
      provider.models.flatMap((model) =>
        model.catalogRef ? [{ providerId: provider.id, modelId: model.id, reference: model.catalogRef }] : [],
      ),
    ),
    ...(capabilities.data?.models.flatMap((model) =>
      model.matched ? [{ providerId: model.providerId, modelId: model.upstreamModel, reference: model.matched }] : [],
    ) ?? []),
  ];
  const references = referenceSources.map((source) => source.reference);
  const memberTargets = new Set(
    (draft?.members ?? []).map((member) => JSON.stringify([member.providerId, member.actualModelId])),
  );
  const memberReferences = referenceSources.filter((source) =>
    memberTargets.has(JSON.stringify([source.providerId, source.modelId])),
  );
  const suggestions = [
    ...new Set([
      ...memberReferences.map((source) => source.reference.modelId),
      ...references.map((reference) => reference.modelId),
    ]),
  ];

  const changed = async () => {
    await Promise.all([
      cache.invalidateQueries({ queryKey: aggregationQueryKey }),
      cache.invalidateQueries({ queryKey: queryKeys.providers }),
      cache.invalidateQueries({ queryKey: queryKeys.clientConfigs }),
      cache.invalidateQueries({ queryKey: queryKeys.claudeDesktopCandidates }),
      cache.invalidateQueries({ queryKey: queryKeys.claudeDesktopStatus }),
    ]);
    setConfigText(null);
  };

  const run = async (action: () => Promise<unknown>) => {
    if (mutationInFlight.current) return;
    mutationInFlight.current = true;
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (cause) {
      setError(formatInvokeError(cause));
    } finally {
      mutationInFlight.current = false;
      setBusy(false);
    }
  };

  const update = (patch: Partial<AggregateModel>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current));

  const memberUpdate = (id: string, patch: Partial<AggregateMember>) => {
    if (draft) {
      update({
        members: draft.members.map((member) => (member.id === id ? { ...member, ...patch } : member)),
      });
    }
  };

  const addMember = () => {
    if (!draft) return;
    const provider = providers.find((provider) => modelsOf(provider).length > 0);
    update({
      members: [
        ...draft.members,
        {
          id: crypto.randomUUID(),
          providerId: provider?.id ?? "",
          actualModelId: modelsOf(provider)[0] ?? "",
          enabled: true,
          priority: 0,
          weight: 1,
        },
      ],
    });
  };

  const save = () =>
    void run(async () => {
      if (!draft) return;
      if (
        !draft.publicModelId.trim() ||
        draft.members.length === 0 ||
        draft.members.some(
          (member) =>
            !member.providerId ||
            !member.actualModelId ||
            !Number.isInteger(member.priority) ||
            member.priority < 0 ||
            member.priority > 1000 ||
            !Number.isInteger(member.weight) ||
            member.weight < 1 ||
            member.weight > 1000,
        )
      ) {
        setError(t("aggregation.formInvalid"));
        return;
      }
      await api.aggregation.save(draft);
      setDraft(null);
      await changed();
      toast({ title: t("aggregation.saved"), variant: "success" });
    });

  const connection = (revealCredential: boolean) =>
    api.aggregation.exportConnection({
      ingressProtocol,
      aggregateIds: available.map((model) => model.id),
      selectedAggregateId: selectedExport?.id ?? null,
      consumerProfile,
      revealCredential,
    });

  const copyPublicId = (publicId: string) =>
    void run(async () => {
      setCopiedId(null);
      await navigator.clipboard.writeText(publicId);
      setCopiedId(publicId);
      toast({ title: t("aggregation.publicIdCopied"), description: publicId, variant: "success" });
    });

  return (
    <section
      className="space-y-5 rounded-2xl border border-border bg-card p-4 shadow-sm sm:p-5"
      aria-labelledby="aggregation-title"
    >
      <FoldHeader
        open={fold.open}
        onToggle={fold.toggle}
        titleId="aggregation-title"
        contentId="aggregation-body"
        title={t("aggregation.title")}
        leading={
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-indigo-500/10 text-indigo-600 dark:text-indigo-400">
            <Network className="h-4 w-4" />
          </div>
        }
        meta={
          <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">
            {models.length}
          </span>
        }
        actions={
          <Button
            disabled={busy || draft !== null || models.length >= 128}
            className="gap-1.5 shadow-sm transition-all motion-safe:active:scale-95"
            onClick={() => {
              if (!fold.open) fold.toggle();
              setDraft(initialModel());
              setError(null);
            }}
          >
            <Plus className="h-4 w-4" />
            {t("aggregation.add")}
          </Button>
        }
      />

      <FoldBody open={fold.open} id="aggregation-body" className="space-y-5">
      {(error || query.error) && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3.5 text-sm text-destructive"
        >
          <X className="h-4 w-4 shrink-0" />
          <span>{error ?? formatInvokeError(query.error)}</span>
        </div>
      )}

      {query.isPending && (
        <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
          <RefreshCw className="h-4 w-4 animate-spin text-primary" />
          <span>{t("common.loading")}</span>
        </div>
      )}

      {/* Draft Creation / Edit Studio Panel */}
      {draft && (
        <form
          className="relative overflow-hidden rounded-2xl border border-indigo-500/30 bg-card p-5 sm:p-6 shadow-md transition-all space-y-6"
          onSubmit={(event) => {
            event.preventDefault();
            save();
          }}
        >
          {/* Top highlight bar */}
          <div className="absolute left-0 top-0 h-1 w-full bg-gradient-to-r from-indigo-500 via-primary to-purple-500" />

          <div className="flex items-center justify-between border-b pb-4">
            <div className="flex items-center gap-2">
              <Sparkles className="h-4 w-4 text-indigo-500" />
              <h3 className="font-semibold text-foreground tracking-tight">
                {draft.id ? t("common.edit") : t("aggregation.add")} · {t("aggregation.title")}
              </h3>
            </div>
            <div className="flex items-center gap-2">
              <label className="flex cursor-pointer items-center gap-2 text-xs font-medium text-foreground select-none">
                <input
                  type="checkbox"
                  className="rounded border-border text-primary focus:ring-primary"
                  checked={draft.enabled}
                  disabled={busy}
                  onChange={(event) => update({ enabled: event.target.checked })}
                />
                {t("aggregation.enabled")}
              </label>
            </div>
          </div>

          <fieldset disabled={busy} className="min-w-0 space-y-6">
            {/* Block 1: Basic Identifiers */}
            <div className="space-y-3">
              <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <Cpu className="h-3.5 w-3.5" />
                <span>{t("aggregation.identity")}</span>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-public-id" className="text-xs font-medium">
                    {t("aggregation.publicId")}
                  </Label>
                  <Input
                    id="aggregate-public-id"
                    list="aggregate-official-models"
                    value={draft.publicModelId}
                    disabled={!!draft.id}
                    placeholder={t("aggregation.publicIdExample")}
                    className="font-mono text-sm bg-background/80"
                    onChange={(event) => {
                      const matching = references.filter((reference) => reference.modelId === event.target.value);
                      const providerSet = new Set(matching.map((reference) => reference.providerId));
                      update({
                        publicModelId: event.target.value,
                        canonicalRef: providerSet.size === 1 ? matching[0] : null,
                      });
                    }}
                    maxLength={256}
                    required
                  />
                  <datalist id="aggregate-official-models">
                    {suggestions.map((id) => (
                      <option key={id} value={id} />
                    ))}
                  </datalist>
                </div>

                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-display-name" className="text-xs font-medium">
                    {t("aggregation.displayName")}
                  </Label>
                  <Input
                    id="aggregate-display-name"
                    value={draft.displayName}
                    placeholder={t("aggregation.displayNameExample")}
                    className="text-sm bg-background/80"
                    onChange={(event) => update({ displayName: event.target.value })}
                    maxLength={200}
                  />
                </div>
              </div>
            </div>

            {/* Block 2: Scheduling & Resilience */}
            <div className="space-y-3">
              <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <Sliders className="h-3.5 w-3.5" />
                <span>{t("aggregation.scheduling")}</span>
              </div>
              <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-strategy" className="text-xs font-medium">
                    {t("aggregation.strategy")}
                  </Label>
                  <Select
                    value={draft.strategy}
                    onValueChange={(value) => update({ strategy: value as AggregateModel["strategy"] })}
                  >
                    <SelectTrigger id="aggregate-strategy" className="bg-background/80">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="roundRobin">{t("aggregation.roundRobin")}</SelectItem>
                      <SelectItem value="weightedRoundRobin">{t("aggregation.weightedRoundRobin")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-affinity" className="text-xs font-medium">
                    {t("aggregation.affinity")}
                  </Label>
                  <Select
                    value={draft.affinity}
                    onValueChange={(value) => update({ affinity: value as AggregateModel["affinity"] })}
                  >
                    <SelectTrigger id="aggregate-affinity" className="bg-background/80">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="session">{t("aggregation.session")}</SelectItem>
                      <SelectItem value="statelessRotation">{t("aggregation.statelessRotation")}</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-attempts" className="text-xs font-medium">
                    {t("aggregation.retryAttempts")}
                  </Label>
                  <Select
                    value={String(draft.retryPolicy.maxAttempts)}
                    onValueChange={(value) => update({ retryPolicy: { ...draft.retryPolicy, maxAttempts: Number(value) } })}
                  >
                    <SelectTrigger id="aggregate-attempts" className="bg-background/80">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="1">1</SelectItem>
                      <SelectItem value="2">2</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-failure-threshold" className="text-xs font-medium">
                    {t("aggregation.failureThreshold")}
                  </Label>
                  <Select
                    value={String(draft.retryPolicy.failureThreshold ?? 3)}
                    onValueChange={(value) => update({ retryPolicy: { ...draft.retryPolicy, failureThreshold: Number(value) } })}
                  >
                    <SelectTrigger id="aggregate-failure-threshold" className="bg-background/80">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {[2, 3, 4, 5, 6, 7, 8, 9, 10].map((value) => (
                        <SelectItem key={value} value={String(value)}>{value}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-1.5">
                  <Label htmlFor="aggregate-compression" className="text-xs font-medium">
                    {t("newapi.compression.modelLevel")}
                  </Label>
                  <div className="flex min-w-0 gap-2">
                    <Select
                      value={draft.compression.mode}
                      onValueChange={(value) => {
                        const mode = value as CompressionPolicy["mode"];
                        update({
                          compression:
                            mode === "percent"
                              ? { mode, percent: 90 }
                              : mode === "tokens"
                                ? { mode, tokens: 100_000 }
                                : { mode },
                        });
                      }}
                    >
                      <SelectTrigger id="aggregate-compression" className="min-w-0 flex-1 bg-background/80">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {(["inherit", "percent", "tokens", "clientDefault"] as const).map((mode) => (
                          <SelectItem key={mode} value={mode}>
                            {t("newapi.compression." + mode)}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    {(draft.compression.mode === "percent" || draft.compression.mode === "tokens") && (
                      <Input
                        type="number"
                        min={1}
                        max={draft.compression.mode === "percent" ? 99 : 10_000_000}
                        className="w-24 shrink-0 bg-background/80 font-mono text-xs"
                        aria-label={t("newapi.compression.modelLevel")}
                        value={
                          draft.compression.mode === "percent"
                            ? draft.compression.percent
                            : draft.compression.tokens
                        }
                        onChange={(event) =>
                          update({
                            compression:
                              draft.compression.mode === "percent"
                                ? { mode: "percent", percent: Number(event.target.value) }
                                : { mode: "tokens", tokens: Number(event.target.value) },
                          })
                        }
                      />
                    )}
                  </div>
                </div>
              </div>
            </div>

            {/* Block 3: Members Pool */}
            <div className="space-y-3 pt-2">
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                  <Layers className="h-3.5 w-3.5" />
                  <span>{t("aggregation.members")}</span>
                </div>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1 rounded-lg text-xs font-medium"
                  disabled={draft.members.length >= 64 || providers.length === 0}
                  onClick={addMember}
                >
                  <Plus className="h-3.5 w-3.5" />
                  {t("aggregation.addMember")}
                </Button>
              </div>

              <div className="space-y-2.5">
                {draft.members.map((member, index) => {
                  const provider = providers.find((provider) => provider.id === member.providerId);
                  const providerModels = modelsOf(provider);

                  return (
                    <div
                      key={member.id}
                      className="relative rounded-xl border border-border/60 bg-muted/20 p-3.5 shadow-sm space-y-3"
                      data-aggregate-member
                    >
                      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1.5fr)_5rem_5rem_auto] items-end">
                        <div className="min-w-0 space-y-1">
                          <Label htmlFor={`member-provider-${member.id}`} className="text-xs font-medium">
                            {t("aggregation.provider")}
                          </Label>
                          <Select
                            value={member.providerId}
                            onValueChange={(value) => {
                              const next = providers.find((provider) => provider.id === value);
                              memberUpdate(member.id, {
                                providerId: value,
                                actualModelId: modelsOf(next)[0] ?? "",
                              });
                            }}
                          >
                            <SelectTrigger id={`member-provider-${member.id}`} className="bg-background/80 text-xs">
                              <SelectValue placeholder={t("aggregation.selectProvider")} />
                            </SelectTrigger>
                            <SelectContent>
                              {!provider && member.providerId && (
                                <SelectItem value={member.providerId} disabled>
                                  {member.providerId} · {t("aggregation.unavailable")}
                                </SelectItem>
                              )}
                              {providers.map((p) => (
                                <SelectItem key={p.id} value={p.id}>
                                  {p.name}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>

                        <div className="min-w-0 space-y-1">
                          <Label htmlFor={`member-model-${member.id}`} className="text-xs font-medium">
                            {t("aggregation.actualId")}
                          </Label>
                          <Select
                            value={member.actualModelId}
                            onValueChange={(value) => memberUpdate(member.id, { actualModelId: value })}
                          >
                            <SelectTrigger id={`member-model-${member.id}`} className="bg-background/80 font-mono text-xs">
                              <SelectValue placeholder={t("aggregation.selectModel")} />
                            </SelectTrigger>
                            <SelectContent>
                              {member.actualModelId && !providerModels.includes(member.actualModelId) && (
                                <SelectItem value={member.actualModelId} disabled>
                                  {member.actualModelId} · {t("aggregation.unavailable")}
                                </SelectItem>
                              )}
                              {providerModels.map((id) => (
                                <SelectItem key={id} value={id}>
                                  {id}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </div>

                        <div className="space-y-1">
                          <Label htmlFor={`member-priority-${member.id}`} className="text-xs font-medium">
                            {t("aggregation.priority")}
                          </Label>
                          <Input
                            id={`member-priority-${member.id}`}
                            type="number"
                            min={0}
                            max={1000}
                            value={member.priority}
                            className="bg-background/80 font-mono text-xs"
                            onChange={(event) => memberUpdate(member.id, { priority: Number(event.target.value) })}
                          />
                        </div>

                        <div className="space-y-1">
                          <Label htmlFor={`member-weight-${member.id}`} className="text-xs font-medium">
                            {t("aggregation.weight")}
                          </Label>
                          <Input
                            id={`member-weight-${member.id}`}
                            type="number"
                            min={1}
                            max={1000}
                            value={member.weight}
                            className="bg-background/80 font-mono text-xs"
                            onChange={(event) => memberUpdate(member.id, { weight: Number(event.target.value) })}
                          />
                        </div>

                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-9 w-9 text-muted-foreground hover:text-destructive"
                          aria-label={t("aggregation.removeMember", { index: index + 1 })}
                          onClick={() => update({ members: draft.members.filter((entry) => entry.id !== member.id) })}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>

                      <div className="flex flex-wrap items-center justify-between gap-2 pt-1 text-xs">
                        <label className="flex cursor-pointer items-center gap-2 select-none">
                          <input
                            type="checkbox"
                            checked={member.enabled}
                            className="rounded border-border text-primary focus:ring-primary"
                            onChange={(event) => memberUpdate(member.id, { enabled: event.target.checked })}
                          />
                          <span className={cn(member.enabled ? "text-foreground" : "text-muted-foreground")}>
                            {t("aggregation.memberEnabled")}
                          </span>
                        </label>

                        <div className="flex items-center gap-2 text-[11px] text-muted-foreground">
                          <Badge variant="outline" className="text-muted-foreground text-[10px] py-0">
                            {t("aggregation.priority")} {member.priority}
                          </Badge>
                          <span>{t("aggregation.weight")} {member.weight}</span>
                        </div>
                      </div>
                    </div>
                  );
                })}

                {draft.members.length === 0 && (
                  <div className="rounded-xl border border-dashed border-border/80 p-6 text-center text-xs text-muted-foreground">
                    {t(providers.length ? "aggregation.addMembersHint" : "aggregation.noProviders")}
                  </div>
                )}
              </div>
            </div>
          </fieldset>

          <div className="flex items-center justify-end gap-2.5 pt-4 border-t">
            <Button type="button" variant="outline" disabled={busy} onClick={() => setDraft(null)}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={busy} className="gap-1.5 shadow-sm">
              <Save className="h-4 w-4" />
              {t("common.save")}
            </Button>
          </div>
        </form>
      )}

      {/* Empty State */}
      {!query.isPending && models.length === 0 && !draft && (
        <div className="rounded-2xl border border-dashed border-border/80 bg-card/50 p-8 text-center space-y-3">
          <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-2xl bg-indigo-500/10 text-indigo-500 ring-1 ring-indigo-500/20">
            <GitBranch className="h-6 w-6" />
          </div>
          <p role="status" className="text-sm text-muted-foreground">{t("aggregation.empty")}</p>
        </div>
      )}

      {/* Aggregated Models Grid Cards */}
      <div className="grid gap-4 md:grid-cols-2">
        {models.map((model) => {
          const isAvailable = model.available;
          return (
            <div
              key={model.id}
              className={cn(
                "group relative flex flex-col justify-between overflow-hidden rounded-2xl border bg-card/80 p-5 shadow-sm transition-all duration-300 hover:border-indigo-500/40 hover:shadow-md",
                isAvailable ? "border-border/80" : "border-destructive/30 bg-destructive/[0.01]",
              )}
            >
              {/* Subtle top indicator bar */}
              <div
                className={cn(
                  "absolute left-5 top-0 h-0.5 w-10 rounded-b-sm transition-all duration-300 group-hover:w-16",
                  isAvailable
                    ? "bg-gradient-to-r from-indigo-500 to-purple-500"
                    : "bg-destructive",
                )}
              />

              <div className="space-y-3.5">
                {/* Header row */}
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="flex min-w-0 max-w-full items-center gap-2.5">
                    <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-indigo-500/10 text-indigo-600 dark:text-indigo-400 shadow-inner ring-1 ring-indigo-500/20">
                      <Cpu className="h-4 w-4" />
                    </div>
                    <div className="min-w-0">
                      <h3 className="break-words font-semibold text-foreground tracking-tight">{model.displayName || model.publicModelId}</h3>
                      <div className="flex items-center gap-1.5 mt-0.5">
                        <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">{model.publicModelId}</span>
                        <button
                          type="button"
                          onClick={() => copyPublicId(model.publicModelId)}
                          disabled={busy}
                          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-muted hover:text-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
                          title={t("aggregation.copyPublicId")}
                          aria-label={t("aggregation.copyPublicId")}
                        >
                          {copiedId === model.publicModelId ? (
                            <Check className="h-3 w-3 text-emerald-500" />
                          ) : (
                            <Copy className="h-3 w-3" />
                          )}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    <Badge
                      variant="outline"
                      className={cn(
                        "gap-1 font-medium text-xs shadow-sm",
                        isAvailable
                          ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
                          : "border-destructive/40 bg-destructive/10 text-destructive",
                      )}
                    >
                      {isAvailable && <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse motion-reduce:animate-none" />}
                      {!isAvailable && <span className="h-1.5 w-1.5 rounded-full bg-destructive" />}
                      {t(isAvailable ? "aggregation.available" : "aggregation.unavailable")}
                    </Badge>
                  </div>
                </div>

                {/* Strategy and Member tags */}
                <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                  <span className="rounded-md bg-muted px-2 py-0.5 font-medium">
                    {t(model.strategy === "weightedRoundRobin" ? "aggregation.weightedRoundRobin" : "aggregation.roundRobin")}
                  </span>
                  <span className="rounded-md bg-muted px-2 py-0.5 font-medium">
                    {t(model.affinity === "statelessRotation" ? "aggregation.statelessRotation" : "aggregation.session")}
                  </span>
                  <span className="rounded-md bg-muted px-2 py-0.5 font-medium">
                    {t("aggregation.memberCount", { count: model.members.length })}
                  </span>
                </div>

                {model.reason && (
                  <p className="rounded-lg bg-destructive/5 border border-destructive/20 p-2 text-xs text-destructive">
                    {t(`aggregation.reasons.${model.reason}`, { defaultValue: t("aggregation.unavailable") })}
                  </p>
                )}

                {/* One member list serves both the route overview and channel details. */}
                <details open className="rounded-xl border border-border/50 bg-background/60 p-3 shadow-sm">
                  <summary className="cursor-pointer rounded text-xs font-semibold text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                    {t("aggregation.members")}
                  </summary>
                  <ul className="mt-2 space-y-1.5">
                    {model.members.map((member) => {
                      const status = model.memberStates.find((state) => state.memberId === member.id);
                      const availability = t(status?.available ? "aggregation.available" : "aggregation.unavailable");
                      return (
                        <li key={member.id} className="space-y-1 rounded-lg border border-border/40 bg-card/50 px-2.5 py-2 text-xs">
                          <div className="flex flex-wrap items-center justify-between gap-2">
                            <div className="flex min-w-0 max-w-full items-center gap-2 font-mono">
                              <span className={cn("h-2 w-2 shrink-0 rounded-full", status?.available ? "bg-emerald-500" : "bg-destructive")}
                                title={availability} aria-label={availability} />
                              <span className="min-w-0 break-all">
                                <span className="font-semibold text-foreground">{status?.providerName || member.providerId}</span>
                                <span className="text-muted-foreground"> / {member.actualModelId}</span>
                              </span>
                            </div>
                            <div className="flex flex-wrap items-center gap-1.5 text-[10px] text-muted-foreground">
                              <Badge variant="outline" className="px-1.5 py-0 text-[10px] font-normal">
                                {status?.protocol === "responses" ? "Responses" : status?.protocol === "chatCompletions" ? "Chat" : status?.protocol === "anthropicMessages" ? "Messages" : "—"}
                              </Badge>
                              <span className="rounded bg-muted px-1.5 py-0.5">{t("aggregation.priority")} {member.priority}</span>
                              <span className="rounded bg-muted px-1.5 py-0.5">{t("aggregation.weight")} {member.weight}</span>
                            </div>
                          </div>
                          {!status?.available && (
                            <p className="text-destructive">{t(`aggregation.reasons.${status?.reason}`, { defaultValue: t("aggregation.unavailable") })}</p>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </details>
              </div>

              {/* Action Buttons */}
              <div className="mt-4 flex items-center justify-end gap-2 pt-3 border-t border-border/50">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-8 gap-1.5 rounded-lg text-xs"
                  disabled={busy || draft !== null}
                  onClick={() =>
                    setDraft({ ...model, retryPolicy: { ...model.retryPolicy, failureThreshold: model.retryPolicy.failureThreshold ?? 3 },
                      members: model.members.map((member) => ({ ...member })) })
                  }
                >
                  <Edit2 className="h-3.5 w-3.5" />
                  {t("common.edit")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-8 gap-1.5 rounded-lg text-xs text-destructive hover:bg-destructive/10"
                  disabled={busy}
                  onClick={() =>
                    setConfirm({
                      title: t("aggregation.deleteTitle"),
                      description: t("aggregation.deleteDescription", { name: model.displayName }),
                      confirmLabel: t("common.delete"),
                      destructive: true,
                      onConfirm: async () => {
                        await run(async () => {
                          await api.aggregation.delete(model.id);
                          await changed();
                        });
                      },
                    })
                  }
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  {t("common.delete")}
                </Button>
              </div>
            </div>
          );
        })}
      </div>

      {/* Aggregation gateway: external clients connect through here */}
      <div
        className="rounded-2xl border border-border/80 bg-card/70 p-4 shadow-sm sm:p-5"
        role="group"
        aria-labelledby="aggregate-export-title"
      >
        <FoldHeader
          open={gatewayFold.open}
          onToggle={gatewayFold.toggle}
          titleId="aggregate-export-title"
          contentId="aggregate-gateway-body"
          title={t("aggregation.gateway")}
          titleClassName="text-sm"
          leading={<Share2 className="h-4 w-4 text-indigo-500" />}
          meta={
            <Badge
              variant="outline"
              className={cn(
                "gap-1 text-xs",
                query.data?.listenerReady
                  ? "border-emerald-500/40 text-emerald-600 dark:text-emerald-400"
                  : "text-muted-foreground",
              )}
            >
              {query.data?.listenerReady && <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />}
              {t(query.data?.listenerReady ? "aggregation.listenerReady" : "aggregation.listenerStopped")}
            </Badge>
          }
          actions={
            <div className="flex items-center gap-2">
              <Label htmlFor="aggregate-external-enabled" className="cursor-pointer text-xs font-medium">
                {t("aggregation.keepService")}
              </Label>
              <Switch
                id="aggregate-external-enabled"
                checked={query.data?.externalEnabled ?? false}
                disabled={busy || !query.isSuccess}
                onCheckedChange={(value) =>
                  void run(async () => {
                    await api.aggregation.setExternalEnabled(value);
                    await changed();
                  })
                }
              />
            </div>
          }
        />

        <FoldBody open={gatewayFold.open} id="aggregate-gateway-body" className="mt-4 space-y-4">
        {/* Export Parameters Grid */}
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div className="space-y-1.5">
            <Label htmlFor="aggregate-export-model" className="text-xs font-medium">
              {t("aggregation.defaultModel")}
            </Label>
            <Select
              value={selectedExport?.id ?? ""}
              onValueChange={(value) => {
                setExportId(value);
                setConfigText(null);
              }}
            >
              <SelectTrigger id="aggregate-export-model" className="bg-background/80 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {exportId && !selectedExport && (
                  <SelectItem value={exportId} disabled>
                    {t("aggregation.unavailable")}
                  </SelectItem>
                )}
                {available.map((model) => (
                  <SelectItem key={model.id} value={model.id}>
                    {model.displayName}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="aggregate-export-protocol" className="text-xs font-medium">
              {t("aggregation.ingress")}
            </Label>
            <Select
              value={ingressProtocol}
              onValueChange={(value) => {
                setIngressProtocol(value as ModelProtocol);
                setConfigText(null);
              }}
            >
              <SelectTrigger id="aggregate-export-protocol" className="bg-background/80 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="responses" disabled={consumerProfile === "anthropicSdk"}>
                  Responses
                </SelectItem>
                <SelectItem value="chatCompletions" disabled={consumerProfile === "anthropicSdk"}>
                  Chat
                </SelectItem>
                <SelectItem value="anthropicMessages">Messages</SelectItem>
              </SelectContent>
            </Select>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="aggregate-export-profile" className="text-xs font-medium">
              {t("aggregation.template")}
            </Label>
            <Select
              value={consumerProfile}
              onValueChange={(value) => {
                setConsumerProfile(value as typeof consumerProfile);
                if (value === "anthropicSdk") setIngressProtocol("anthropicMessages");
                setConfigText(null);
              }}
            >
              <SelectTrigger id="aggregate-export-profile" className="bg-background/80 text-xs">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="generic">{t("aggregation.generic")}</SelectItem>
                <SelectItem value="anthropicSdk">Anthropic SDK</SelectItem>
                <SelectItem value="opencode">OpenCode</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>

        {/* Buttons */}
        <div className="flex flex-wrap items-center gap-2 pt-2">
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-1.5 text-xs"
            disabled={busy || !selectedExport}
            onClick={() => void run(async () => setConfigText((await connection(false)).configText))}
          >
            {t("aggregation.previewConnection")}
          </Button>

          <Button
            size="sm"
            className="h-8 gap-1.5 text-xs shadow-sm"
            disabled={busy || !selectedExport}
            onClick={() =>
              void run(async () => {
                const result = await connection(true);
                await navigator.clipboard.writeText(result.configText);
                toast({ title: t("aggregation.copied"), variant: "success" });
              })
            }
          >
            <Copy className="h-3.5 w-3.5" />
            {t("aggregation.copyConnection")}
          </Button>
        </div>

        {/* Code Snippet Output Box */}
        {configText !== null && (
          <div className="relative mt-3 rounded-xl border border-border/60 bg-zinc-950 p-4 font-mono text-xs text-zinc-100 shadow-inner">
            <div className="flex items-center justify-between pb-2 mb-2 border-b border-zinc-800 text-[11px] text-zinc-400">
              <span>{t("aggregation.template")} · {consumerProfile === "generic" ? t("aggregation.generic") : consumerProfile === "anthropicSdk" ? "Anthropic SDK" : "OpenCode"}</span>
              <button
                type="button"
                disabled={busy}
                onClick={() => void run(async () => {
                  await navigator.clipboard.writeText(configText);
                  toast({ title: t("aggregation.copied"), variant: "success" });
                })}
                className="flex shrink-0 items-center gap-1 rounded px-1 py-1 hover:text-white transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
              >
                <Copy className="h-3 w-3" />
                <span>{t("aggregation.copyAll")}</span>
              </button>
            </div>
            <pre
              className="max-h-72 overflow-auto font-mono text-xs leading-relaxed"
              data-aggregate-export-preview
            >
              {configText}
            </pre>
          </div>
        )}
        </FoldBody>
      </div>
      </FoldBody>

      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </section>
  );
}
