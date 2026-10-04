import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Circle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { effectiveModelProtocol, supportsProtocol } from "@/lib/provider-models";
import { cn } from "@/lib/utils";
import { ModelResolutionFields, resolvedLimitPlaceholder } from "./model-resolution-fields";
import type { ModelResolutionState } from "./use-model-resolutions";
import type { ModelProtocol, NewApiModel } from "@/types/newapi";

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
        <div className="flex items-baseline gap-2">
          <h3 className="text-sm font-semibold">{t("newapi.modelLibrary")}</h3>
          <span className="text-xs tabular-nums text-muted-foreground">{t("newapi.modelsSelected", { selected, total: models.length })}</span>
        </div>
        <div className="ml-auto flex items-center gap-1">
          <Button type="button" variant="ghost" size="sm" className="h-7 px-2 text-xs" disabled={disabled || selected === 0} onClick={() => setAll(false)}>
            {t("newapi.clearModels")}
          </Button>
          <Button type="button" variant="outline" size="sm" className="h-8 px-2.5 text-xs" disabled={disabled} onClick={onFetch}>
            <RefreshCw className={cn("h-3.5 w-3.5", fetching && "animate-spin")} />
            {t("newapi.fetchModels")}
          </Button>
        </div>
      </div>

      <div className="flex gap-2">
        <Input
          value={newModel}
          disabled={disabled}
          autoComplete="off"
          spellCheck={false}
          placeholder={t("newapi.addModelPlaceholder")}
          onChange={event => { setNewModel(event.target.value); setPendingLimit(50); }}
          onKeyDown={event => {
            if (event.key === "Enter") {
              event.preventDefault();
              addModel();
            }
          }}
        />
        <Button type="button" variant="outline" disabled={disabled || !newModel.trim()} onClick={addModel}>
          <Plus className="h-4 w-4" />
          {t("newapi.addModel")}
        </Button>
      </div>

      {pending.length > 0 && <details open className="rounded-xl border border-border/60 bg-muted/15 p-3" data-pending-models>
        <summary className="cursor-pointer text-xs font-medium">{t("newapi.pendingModels", { count: pending.length })}</summary>
        <p className="mt-1 text-[11px] text-muted-foreground">{t("newapi.pendingModelsHint")}</p>
        <div className="mt-2 flex flex-wrap gap-1.5">
          <Button type="button" size="sm" variant="ghost" disabled={disabled || matchingPending.length === 0} onClick={() => {
            const matchingIds = new Set(matchingPending.map(model => model.id));
            onModelsChange(models.map(model => matchingIds.has(model.id) ? { ...model, included: true } : model));
          }}>{t(newModel.trim() ? "newapi.selectMatchingModels" : "newapi.selectAllModels")}</Button>
          <Button type="button" size="sm" variant="ghost" disabled={disabled} onClick={() => onModelsChange(models.filter(model => model.included))}>{t("newapi.clearPendingModels")}</Button>
        </div>
        <div className="mt-2 max-h-40 space-y-1 overflow-y-auto">
          {matchingPending.slice(0, pendingLimit).map(model => <label key={model.id} className="flex min-w-0 cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted">
            <Checkbox checked={false} disabled={disabled} onCheckedChange={value => { if (value === true) setIncluded(model, true); }} aria-label={t("newapi.includeModelAria", { model: model.id })} />
            <span className="min-w-0 truncate font-mono" title={model.id}>{model.id}</span>
          </label>)}
          {matchingPending.length === 0 && <p className="text-xs text-muted-foreground">{t("newapi.noMatchingPendingModels")}</p>}
          {matchingPending.length > pendingLimit && <Button type="button" variant="ghost" size="sm" onClick={() => setPendingLimit(limit => limit + 50)}>{t("newapi.morePendingModels")}</Button>}
        </div>
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
        <div className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
          {t(fetching ? "newapi.loadingModels" : pending.length ? "newapi.noIncludedModels" : "newapi.emptyModelLibrary")}
        </div>
      ) : (
        <div className="overflow-hidden rounded-xl border">
          <div className="hidden grid-cols-[32px_minmax(130px,1.2fr)_minmax(100px,1fr)_150px_240px_36px] gap-2 bg-muted/50 px-3 py-2 text-[11px] font-medium text-muted-foreground md:grid">
            <span><span className="sr-only">{t("newapi.includeModel")}</span></span>
            <span>{t("newapi.modelId")}</span>
            <span>{t("newapi.menuDisplayName")}</span>
            <span>{t("newapi.protocol")}</span>
            <span title={t("newapi.modelLimitsColumnHint")}>{t("newapi.modelLimitsColumn")}</span>
            <span><span className="sr-only">{t("newapi.removeModel")}</span></span>
          </div>
          <div className="max-h-[310px] overflow-y-auto">
            {models.filter(model => model.included).map(model => {
              const resolution = resolutions.models.get(model.id);
              const supported = supportsProtocol(model, effectiveModelProtocol(model, protocol));
              const isDefault = defaultModel === model.id;
              return (
                <div key={model.id} data-model-row={model.id} className="grid gap-2 border-t px-3 py-2.5 first:border-t-0 md:grid-cols-[32px_minmax(130px,1.2fr)_minmax(100px,1fr)_150px_240px_36px] md:items-center">
                  <div className="flex items-center gap-2 md:block">
                    <Checkbox checked={model.included} disabled={disabled} onCheckedChange={value => setIncluded(model, value === true)} aria-label={t("newapi.includeModelAria", { model: model.id })} />
                    <span className="text-xs text-muted-foreground md:hidden">{t("newapi.includeModel")}</span>
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5">
                    <button
                      type="button"
                      disabled={disabled || !model.included}
                      onClick={() => onDefaultModelChange(isDefault ? "" : model.id)}
                      title={t("newapi.setDefaultModel", { model: model.id })}
                      aria-label={t("newapi.setDefaultModel", { model: model.id })}
                      aria-pressed={isDefault}
                      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-30"
                    >
                      {isDefault ? <CheckCircle2 className="h-4 w-4 text-primary" /> : <Circle className="h-4 w-4 text-muted-foreground" />}
                    </button>
                    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className="truncate font-mono text-xs" title={model.id}>{model.id}</span>
                      {isDefault && <span className="text-[10px] font-medium text-primary">{t("newapi.defaultBadge")}</span>}
                    </div>
                  </div>
                  <Input className="h-8 text-xs" value={model.displayName === model.id && providerName.trim() ? `${providerName.trim()}/${model.id}` : model.displayName} disabled={disabled} aria-label={t("newapi.menuDisplayNameFor", { model: model.id })} onChange={event => patch(model.id, { displayName: event.target.value })} />
                  <Select value={model.protocol ?? "inherit"} disabled={disabled} onValueChange={value => patch(model.id, { protocol: value === "inherit" ? null : value as ModelProtocol })}>
                    <SelectTrigger className="h-8 min-w-0 text-xs" aria-label={t("newapi.protocolFor", { model: model.id })}><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="inherit">{t("newapi.inheritProtocol", { protocol: t({ responses: "newapi.protocolResponses", chatCompletions: "newapi.protocolChatCompletions", anthropicMessages: "newapi.protocolAnthropicMessages" }[effectiveModelProtocol({ ...model, protocol: null }, protocol)]) })}</SelectItem>
                      <SelectItem value="responses">{t("newapi.protocolResponses")}</SelectItem>
                      <SelectItem value="chatCompletions">{t("newapi.protocolChatCompletions")}</SelectItem>
                      <SelectItem value="anthropicMessages">{t("newapi.protocolAnthropicMessages")}</SelectItem>
                    </SelectContent>
                  </Select>
                  <ModelResolutionFields model={model} resolution={resolution} loading={resolutions.loading} error={resolutions.error} disabled={disabled} onChange={change => patch(model.id, change)} />
                  <button type="button" disabled={disabled} onClick={() => removeModel(model.id)} className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:opacity-30" aria-label={t("newapi.removeModelAria", { model: model.id })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                  {!supported && <p className="text-[10px] text-muted-foreground md:col-start-2 md:col-span-4">{t("newapi.protocolMismatchModel")}</p>}
                  <details className="text-xs md:col-start-2 md:col-span-4">
                    <summary className="cursor-pointer text-muted-foreground">{t("newapi.modelFieldOverrides")}</summary>
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
                      <p className="sm:col-span-2">{t("newapi.modelFieldOverridesHint")}</p>
                    </div>
                  </details>
                </div>
              );
            })}
          </div>
        </div>
      )}
      <p className="text-xs leading-relaxed text-muted-foreground">{t("newapi.codexCatalogHint")}</p>
    </div>
  );
}
