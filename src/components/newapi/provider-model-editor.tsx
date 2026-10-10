import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Brain, Library, Maximize2, Plus, RefreshCw, Search, Star, Trash2, Upload } from "lucide-react";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { effectiveModelProtocol, supportsProtocol } from "@/lib/provider-models";
import { cn } from "@/lib/utils";
import { compactTokens, ModelResolutionFields, resolvedLimitPlaceholder } from "./model-resolution-fields";
import type { ModelResolutionState } from "./use-model-resolutions";
import type { ModelProtocol, NewApiModel, ProviderModelResolution } from "@/types/newapi";

interface Props {
  models: NewApiModel[];
  providerName: string;
  protocol: ModelProtocol;
  defaultModel: string;
  disabled: boolean;
  fetching: boolean;
  fetchError: string | null;
  resolutions: ModelResolutionState;
  onFetch: () => void;
  onModelsChange: (models: NewApiModel[]) => void;
  onDefaultModelChange: (model: string) => void;
}

const protocolKey: Record<ModelProtocol, string> = {
  responses: "newapi.protocolResponses",
  chatCompletions: "newapi.protocolChatCompletions",
  anthropicMessages: "newapi.protocolAnthropicMessages",
};
const protocolShort: Record<ModelProtocol, string> = {
  responses: "newapi.protocolShortResponses",
  chatCompletions: "newapi.protocolShortChat",
  anthropicMessages: "newapi.protocolShortAnthropic",
};
const fact = "inline-flex h-6 min-w-0 items-center gap-1 rounded-full px-2 text-[11px] font-medium";
const matchTone: Record<string, string> = {
  explicit: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  exact: "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
  normalized: "bg-sky-500/10 text-sky-700 dark:text-sky-400",
  consensus: "bg-sky-500/10 text-sky-700 dark:text-sky-400",
  ambiguous: "bg-amber-500/15 text-amber-700 dark:text-amber-400",
  invalid: "bg-red-500/10 text-red-600 dark:text-red-400",
  unmatched: "bg-muted text-muted-foreground",
};

/** What the router will use for a model, read at a glance. */
function ModelFacts({ model, resolution, protocol, loading }: {
  model: NewApiModel; resolution?: ProviderModelResolution; protocol: ModelProtocol; loading: boolean;
}) {
  const { t } = useTranslation();
  const effective = effectiveModelProtocol(model, protocol);
  const context = model.overrides?.contextWindow ?? resolution?.context.value ?? null;
  const output = model.overrides?.maxOutputTokens ?? resolution?.output.value ?? null;
  const reasoning = model.overrides?.reasoning ?? model.supportedReasoningLevels ?? null;
  const kind = resolution?.matchKind ?? "unmatched";
  return (
    <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5" data-model-facts>
      <span className={cn(fact, "bg-primary/10 text-primary")} title={t(protocolKey[effective])}>{t(protocolShort[effective])}</span>
      {loading && !resolution ? (
        <span className={cn(fact, "w-28 animate-pulse bg-muted")} aria-hidden />
      ) : (
        <>
          <span className={cn(fact, "bg-muted/80 text-foreground/80")}>
            <Maximize2 aria-hidden className="h-3 w-3 opacity-70" />
            {context != null ? compactTokens(context) : "—"}
            {model.overrides?.contextWindow != null && <small className="text-[10px] text-primary">{t("newapi.factManual")}</small>}
          </span>
          {output != null && (
            <span className={cn(fact, "bg-muted/80 text-foreground/80")} title={t("newapi.capabilityOutput")}>
              <Upload aria-hidden className="h-3 w-3 opacity-70" />{compactTokens(output)}
            </span>
          )}
          <span className={cn(fact, matchTone[kind] ?? matchTone.unmatched)}
            title={resolution?.matched ? `${resolution.matched.providerId}/${resolution.matched.modelId}` : undefined}>
            <Library aria-hidden className="h-3 w-3 opacity-70" />
            <span className="truncate">{t(`newapi.matchKind.${kind}`)}{resolution?.matched ? ` · ${resolution.matched.providerId}` : ""}</span>
          </span>
          {reasoning && reasoning.length > 0 && (
            <span className={cn(fact, "bg-violet-500/10 text-violet-700 dark:text-violet-300")} title={t("newapi.capabilityReasoning")}>
              <Brain aria-hidden className="h-3 w-3 opacity-70" />
              <span className="truncate">{reasoning.map(level => level.effort).join(" / ")}</span>
            </span>
          )}
        </>
      )}
    </div>
  );
}

export function ProviderModelEditor({ models, providerName, protocol, defaultModel, disabled, fetching, fetchError, resolutions, onFetch, onModelsChange, onDefaultModelChange }: Props) {
  const { t, i18n } = useTranslation();
  const [newModel, setNewModel] = useState("");
  const [pendingLimit, setPendingLimit] = useState(50);
  const [reasoningDrafts, setReasoningDrafts] = useState<Record<string, string>>({});
  const selected = models.filter(model => model.included).length;
  const pending = models.filter(model => !model.included);
  const matchingPending = pending.filter(model => `${model.id} ${model.displayName}`.toLowerCase().includes(newModel.trim().toLowerCase()));

  const patch = (id: string, change: Partial<NewApiModel>) => {
    onModelsChange(models.map(model => model.id === id ? { ...model, ...change } : model));
  };
  const setIncluded = (model: NewApiModel, included: boolean) => {
    patch(model.id, { included });
    if (!included && defaultModel === model.id) {
      onDefaultModelChange("");
    }
  };
  const addModel = () => {
    const id = newModel.trim();
    if (!id) return;
    const existing = models.find(model => model.id === id);
    if (existing) {
      patch(id, { included: true });
    } else {
      onModelsChange([...models, {
        id,
        displayName: id,
        contextWindow: null,
        maxOutputTokens: null,
        endpointTypes: [],
        included: true,
        metadataSource: "local",
      }]);
    }
    setNewModel("");
  };
  const removeModel = (id: string) => {
    const next = models.filter(model => model.id !== id);
    onModelsChange(next);
    if (defaultModel === id) {
      onDefaultModelChange("");
    }
  };
  const setAll = (included: boolean) => {
    const next = models.map(model => ({ ...model, included }));
    onModelsChange(next);
    if (!included) onDefaultModelChange("");
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-baseline gap-2.5">
          <h3 className="font-display text-base font-semibold tracking-tight">{t("newapi.modelLibrary")}</h3>
          <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-semibold tabular-nums text-primary">
            {t("newapi.modelsSelected", { selected, total: models.length })}
          </span>
        </div>
        <div className="ml-auto flex items-center gap-1">
          <Button type="button" variant="ghost" size="sm" className="h-8 rounded-lg px-2 text-xs" disabled={disabled || selected === 0} onClick={() => setAll(false)}>
            {t("newapi.clearModels")}
          </Button>
          <Button type="button" variant="outline" size="sm" className="h-8 rounded-lg px-2.5 text-xs" disabled={disabled} onClick={onFetch}>
            <RefreshCw className={cn("h-3.5 w-3.5", fetching && "animate-spin")} />
            {t("newapi.fetchModels")}
          </Button>
        </div>
      </div>

      <div className="flex gap-2">
        <label className="relative flex-1">
          <Search aria-hidden className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={newModel}
            disabled={disabled}
            autoComplete="off"
            spellCheck={false}
            className="h-9 rounded-xl pl-9"
            placeholder={t("newapi.addModelPlaceholder")}
            onChange={event => { setNewModel(event.target.value); setPendingLimit(50); }}
            onKeyDown={event => {
              if (event.key === "Enter") {
                event.preventDefault();
                addModel();
              }
            }}
          />
        </label>
        <Button type="button" variant="outline" className="h-9 rounded-xl" disabled={disabled || !newModel.trim()} onClick={addModel}>
          <Plus className="h-4 w-4" />
          {t("newapi.addModel")}
        </Button>
      </div>

      {pending.length > 0 && <details open className="group/pending rounded-2xl border border-dashed border-primary/30 bg-primary/[0.03] p-3" data-pending-models>
        <summary className="flex cursor-pointer list-none items-center justify-between gap-2 text-xs font-semibold">
          <span>{t("newapi.pendingModels", { count: pending.length })}</span>
          <span className="flex gap-1">
            <Button type="button" size="sm" variant="ghost" className="h-7 rounded-lg px-2 text-xs" disabled={disabled || matchingPending.length === 0} onClick={event => {
              event.preventDefault();
              const matchingIds = new Set(matchingPending.map(model => model.id));
              onModelsChange(models.map(model => matchingIds.has(model.id) ? { ...model, included: true } : model));
            }}>{t(newModel.trim() ? "newapi.selectMatchingModels" : "newapi.selectAllModels")}</Button>
            <Button type="button" size="sm" variant="ghost" className="h-7 rounded-lg px-2 text-xs" disabled={disabled} onClick={event => {
              event.preventDefault();
              onModelsChange(models.filter(model => model.included));
            }}>{t("newapi.clearPendingModels")}</Button>
          </span>
        </summary>
        <div className="mt-2 grid max-h-44 gap-1 overflow-y-auto sm:grid-cols-2">
          {matchingPending.slice(0, pendingLimit).map(model => <label key={model.id} className="flex min-w-0 cursor-pointer items-center gap-2 rounded-lg border border-transparent bg-background/70 px-2 py-1.5 text-xs transition-colors hover:border-primary/30 hover:bg-background">
            <Checkbox checked={false} disabled={disabled} onCheckedChange={value => { if (value === true) setIncluded(model, true); }} aria-label={t("newapi.includeModelAria", { model: model.id })} />
            <span className="min-w-0 truncate font-mono" title={model.id}>{model.id}</span>
          </label>)}
          {matchingPending.length === 0 && <p className="text-xs text-muted-foreground">{t("newapi.noMatchingPendingModels")}</p>}
        </div>
        {matchingPending.length > pendingLimit && <Button type="button" variant="ghost" size="sm" className="mt-1" onClick={() => setPendingLimit(limit => limit + 50)}>{t("newapi.morePendingModels")}</Button>}
      </details>}
      {!defaultModel && <p className="text-xs text-muted-foreground" data-model-default-empty>{t("newapi.noDefaultModel")}</p>}
      {models.some(model => model.included) && (resolutions.loading || resolutions.error) && <div className="flex items-center gap-2 text-xs text-muted-foreground" role="status">
        {resolutions.loading ? t("newapi.modelResolution.loading") : <>
          <span>{t("newapi.modelResolution.unavailable")}</span>
          <button type="button" disabled={disabled} onClick={resolutions.retry} className="rounded text-primary hover:text-primary/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40">{t("newapi.modelResolution.retry")}</button>
        </>}
      </div>}
      {fetchError && <p role="alert" className="text-xs text-destructive">{t(fetchError)}</p>}
      {selected === 0 ? (
        <div className="route-surface rounded-2xl border border-dashed px-4 py-10 text-center text-sm text-muted-foreground">
          {t(fetching ? "newapi.loadingModels" : pending.length ? "newapi.noIncludedModels" : "newapi.emptyModelLibrary")}
        </div>
      ) : (
        <div className="max-h-[440px] space-y-2 overflow-y-auto pr-0.5">
          {models.filter(model => model.included).map(model => {
            const resolution = resolutions.models.get(model.id);
            const supported = supportsProtocol(model, effectiveModelProtocol(model, protocol));
            const isDefault = defaultModel === model.id;
            return (
              <div key={model.id} data-model-row={model.id} data-default={isDefault || undefined}
                className={cn("model-card rounded-2xl border bg-card p-3 transition-[border-color,box-shadow] duration-200",
                  isDefault ? "border-primary/45 shadow-[0_10px_28px_-20px_hsl(var(--primary)/0.9)]" : "border-border/80 hover:border-primary/25")}>
                <div className="flex items-start gap-2.5">
                  <Checkbox className="mt-2" checked={model.included} disabled={disabled} onCheckedChange={value => setIncluded(model, value === true)} aria-label={t("newapi.includeModelAria", { model: model.id })} />
                  <button
                    type="button"
                    disabled={disabled || !model.included}
                    onClick={() => onDefaultModelChange(isDefault ? "" : model.id)}
                    title={t("newapi.setDefaultModel", { model: model.id })}
                    aria-label={t("newapi.setDefaultModel", { model: model.id })}
                    aria-pressed={isDefault}
                    className={cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-xl transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30",
                      isDefault ? "bg-primary text-primary-foreground shadow-sm" : "bg-muted/70 text-muted-foreground hover:bg-primary/10 hover:text-primary")}
                  >
                    <Star className={cn("h-4 w-4", isDefault && "fill-current")} />
                  </button>
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate font-mono text-[13px] font-semibold" title={model.id}>{model.id}</span>
                      {isDefault && <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-px text-[10px] font-semibold text-primary">{t("newapi.defaultBadge")}</span>}
                    </div>
                    <ModelFacts model={model} resolution={resolution} protocol={protocol} loading={resolutions.loading} />
                  </div>
                  <button type="button" disabled={disabled} onClick={() => removeModel(model.id)} className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:opacity-30" aria-label={t("newapi.removeModelAria", { model: model.id })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                </div>
                <div className="mt-3 grid gap-2.5 border-t border-dashed border-border/70 pt-3 md:grid-cols-[minmax(0,1fr)_minmax(0,170px)_minmax(0,1.25fr)]">
                  <label className="min-w-0 space-y-1">
                    <span className="block text-[10.5px] font-medium text-muted-foreground">{t("newapi.menuDisplayName")}</span>
                    <Input className="h-8 rounded-lg text-xs" value={model.displayName === model.id && providerName.trim() ? `${providerName.trim()}/${model.id}` : model.displayName} disabled={disabled} aria-label={t("newapi.menuDisplayNameFor", { model: model.id })} onChange={event => patch(model.id, { displayName: event.target.value })} />
                  </label>
                  <div className="min-w-0 space-y-1">
                    <span className="block text-[10.5px] font-medium text-muted-foreground">{t("newapi.protocol")}</span>
                    <Select value={model.protocol ?? "inherit"} disabled={disabled} onValueChange={value => patch(model.id, { protocol: value === "inherit" ? null : value as ModelProtocol })}>
                      <SelectTrigger className="h-8 min-w-0 rounded-lg text-xs" aria-label={t("newapi.protocolFor", { model: model.id })}><SelectValue /></SelectTrigger>
                      <SelectContent>
                        <SelectItem value="inherit">{t("newapi.inheritProtocol", { protocol: t(protocolKey[effectiveModelProtocol({ ...model, protocol: null }, protocol)]) })}</SelectItem>
                        <SelectItem value="responses">{t("newapi.protocolResponses")}</SelectItem>
                        <SelectItem value="chatCompletions">{t("newapi.protocolChatCompletions")}</SelectItem>
                        <SelectItem value="anthropicMessages">{t("newapi.protocolAnthropicMessages")}</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="min-w-0 space-y-1">
                    <span className="block text-[10.5px] font-medium text-muted-foreground" title={t("newapi.modelLimitsColumnHint")}>{t("newapi.modelLimitsColumn")}</span>
                    <ModelResolutionFields model={model} resolution={resolution} loading={resolutions.loading} error={resolutions.error} disabled={disabled} onChange={change => patch(model.id, change)} />
                  </div>
                </div>
                {!supported && <p className="mt-2 text-[11px] text-amber-700 dark:text-amber-400">{t("newapi.protocolMismatchModel")}</p>}
                <details className="mt-2 text-xs">
                  <summary className="cursor-pointer text-muted-foreground transition-colors hover:text-foreground">{t("newapi.modelFieldOverrides")}</summary>
                  <div className="mt-2 grid gap-2 sm:grid-cols-2">
                    {(["inputLimit", "maxOutputTokens"] as const).map(field => (
                      <label key={field} className="space-y-1 text-muted-foreground">
                        <span>{t(field === "inputLimit" ? "newapi.capabilityInput" : "newapi.capabilityOutput")}</span>
                        <Input type="number" min={field === "inputLimit" ? 1024 : 1} max={100000000}
                          className="h-7 text-xs placeholder:text-foreground/75" disabled={disabled} placeholder={resolvedLimitPlaceholder(field === "inputLimit" ? resolution?.inputLimit : resolution?.output, i18n.language, t("newapi.matchKind.unmatched"))}
                          aria-label={`${t(field === "inputLimit" ? "newapi.capabilityInput" : "newapi.capabilityOutput")} · ${model.id}`}
                          value={model.overrides?.[field] ?? ""}
                          onChange={event => patch(model.id, { overrides: { ...model.overrides, [field]: event.target.value ? Number(event.target.value) : null } })} />
                      </label>
                    ))}
                    <label className="space-y-1 text-muted-foreground">
                      <span>{t("newapi.capabilityReasoning")}</span>
                      <Input className="h-7 text-xs" disabled={disabled} placeholder={t("newapi.reasoningOverridePlaceholder")}
                        pattern="(?:-|[A-Za-z0-9_]{1,32}(?:\s*,\s*[A-Za-z0-9_]{1,32})*)"
                        title={t("newapi.modelFieldOverridesHint")}
                        aria-label={`${t("newapi.capabilityReasoning")} · ${model.id}`}
                        value={reasoningDrafts[model.id] ?? (model.overrides?.reasoning == null ? "" : model.overrides.reasoning.length === 0 ? "-" : model.overrides.reasoning.map(level => level.effort).join(", "))}
                        onChange={event => setReasoningDrafts(current => ({ ...current, [model.id]: event.target.value }))}
                        onBlur={event => {
                          const text = event.target.value.trim();
                          const efforts = [...new Set(text.split(",").map(value => value.trim()).filter(Boolean))];
                          if (text && text !== "-" && (efforts.length > 32 || efforts.some(value => !/^[a-zA-Z0-9_]{1,32}$/.test(value)))) return;
                          patch(model.id, { overrides: { ...model.overrides, reasoning: !text ? null : text === "-" ? [] : efforts.map(effort => ({ effort, description: effort })) } });
                          setReasoningDrafts(current => { const next = { ...current }; delete next[model.id]; return next; });
                        }} />
                    </label>
                    <label className="space-y-1 text-muted-foreground">
                      <span>{t("newapi.capabilityDefaultEffort")}</span>
                      <Input className="h-7 text-xs" disabled={disabled} placeholder={t("clients.limitsAuto")}
                        pattern="[A-Za-z0-9_]{1,32}"
                        aria-label={`${t("newapi.capabilityDefaultEffort")} · ${model.id}`}
                        value={model.overrides?.defaultEffort ?? ""}
                        onChange={event => patch(model.id, { overrides: { ...model.overrides, defaultEffort: event.target.value.trim() || null } })} />
                    </label>
                  </div>
                </details>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
