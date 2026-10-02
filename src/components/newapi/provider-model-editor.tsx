import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Circle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { effectiveModelProtocol, supportsProtocol } from "@/lib/provider-models";
import { cn } from "@/lib/utils";
import type { CompressionPolicy, ModelProtocol, NewApiModel } from "@/types/newapi";

interface Props {
  models: NewApiModel[];
  providerName: string;
  protocol: ModelProtocol;
  defaultModel: string;
  disabled: boolean;
  fetching: boolean;
  fetchError: string | null;
  onFetch: () => void;
  onModelsChange: (models: NewApiModel[]) => void;
  onDefaultModelChange: (model: string) => void;
}

export function ProviderModelEditor({ models, providerName, protocol, defaultModel, disabled, fetching, fetchError, onFetch, onModelsChange, onDefaultModelChange }: Props) {
  const { t } = useTranslation();
  const [newModel, setNewModel] = useState("");
  // Drafts keep a half-typed catalog reference visible until it is complete.
  const [mappingDrafts, setMappingDrafts] = useState<Record<string, string>>({});
  const [reasoningDrafts, setReasoningDrafts] = useState<Record<string, string>>({});
  const selected = models.filter(model => model.included).length;

  const patch = (id: string, change: Partial<NewApiModel>) => {
    onModelsChange(models.map(model => model.id === id ? { ...model, ...change } : model));
  };
  const commitMapping = (id: string, text: string) => {
    const value = text.trim();
    const reference = (() => {
      if (!value) return null;
      const [providerId, ...rest] = value.split("/");
      const modelId = rest.join("/");
      if (!providerId || !modelId) return undefined;
      return { providerId: providerId.toLowerCase(), modelId };
    })();
    setMappingDrafts(current => {
      const next = { ...current };
      delete next[id];
      return next;
    });
    // `undefined` is an incomplete reference: keep the stored value as is.
    if (reference === undefined) return;
    patch(id, { catalogRef: reference });
  };
  const setIncluded = (model: NewApiModel, included: boolean) => {
    patch(model.id, { included });
    if (!included && defaultModel === model.id) {
      onDefaultModelChange(models.find(item => item.id !== model.id && item.included)?.id ?? "");
    }
  };
  const addModel = () => {
    const id = newModel.trim();
    if (!id) return;
    const existing = models.find(model => model.id === id);
    if (existing) {
      patch(id, { included: true });
      if (!defaultModel) onDefaultModelChange(id);
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
      if (!defaultModel) onDefaultModelChange(id);
    }
    setNewModel("");
  };
  const removeModel = (id: string) => {
    const next = models.filter(model => model.id !== id);
    onModelsChange(next);
    if (defaultModel === id) {
      onDefaultModelChange(next.find(model => model.included)?.id ?? "");
    }
  };
  const setAll = (included: boolean) => {
    const next = models.map(model => ({ ...model, included }));
    onModelsChange(next);
    if (!included) onDefaultModelChange("");
    else if (!next.some(model => model.id === defaultModel && model.included)) {
      onDefaultModelChange(next.find(model => model.included)?.id ?? "");
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-baseline gap-2">
          <h3 className="text-sm font-semibold">{t("newapi.modelLibrary")}</h3>
          <span className="text-xs tabular-nums text-muted-foreground">{t("newapi.modelsSelected", { selected, total: models.length })}</span>
        </div>
        <div className="ml-auto flex items-center gap-1">
          <Button type="button" variant="ghost" size="sm" className="h-7 px-2 text-xs" disabled={disabled || models.length === 0} onClick={() => setAll(selected !== models.length)}>
            {t(selected === models.length && models.length > 0 ? "newapi.clearModels" : "newapi.selectAllModels")}
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
          onChange={event => setNewModel(event.target.value)}
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

      {fetchError && <p role="alert" className="text-xs text-destructive">{t(fetchError)}</p>}
      {models.length === 0 ? (
        <div className="rounded-xl border border-dashed px-4 py-8 text-center text-sm text-muted-foreground">
          {t(fetching ? "newapi.loadingModels" : "newapi.emptyModelLibrary")}
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
            {models.map(model => {
              const supported = supportsProtocol(model, effectiveModelProtocol(model, protocol));
              const isDefault = defaultModel === model.id;
              return (
                <div key={model.id} className="grid gap-2 border-t px-3 py-2.5 first:border-t-0 md:grid-cols-[32px_minmax(130px,1.2fr)_minmax(100px,1fr)_150px_240px_36px] md:items-center">
                  <div className="flex items-center gap-2 md:block">
                    <Checkbox checked={model.included} disabled={disabled || isDefault} onCheckedChange={value => setIncluded(model, value === true)} aria-label={t("newapi.includeModelAria", { model: model.id })} />
                    <span className="text-xs text-muted-foreground md:hidden">{t("newapi.includeModel")}</span>
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5">
                    <button
                      type="button"
                      disabled={disabled || !model.included}
                      onClick={() => onDefaultModelChange(model.id)}
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
                  <Select value={effectiveModelProtocol(model, protocol)} disabled={disabled} onValueChange={value => patch(model.id, { protocol: value as ModelProtocol })}>
                    <SelectTrigger className="h-8 min-w-0 text-xs" aria-label={t("newapi.protocolFor", { model: model.id })}><SelectValue /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="responses">{t("newapi.protocolResponses")}</SelectItem>
                      <SelectItem value="chatCompletions">{t("newapi.protocolChatCompletions")}</SelectItem>
                      <SelectItem value="anthropicMessages">{t("newapi.protocolAnthropicMessages")}</SelectItem>
                    </SelectContent>
                  </Select>
                  <div className="space-y-1">
                    <div className="flex items-center gap-1.5">
                      <span className="w-8 shrink-0 text-[9px] uppercase tracking-wider text-muted-foreground/70" title={t("newapi.contextWindowShort")}>
                        {t("newapi.limitShort")}
                      </span>
                      <Input className="h-7 text-xs tabular-nums" type="number" min={1024} max={100000000} inputMode="numeric" placeholder={t("clients.limitsAuto")} value={model.overrides?.contextWindow ?? ""} disabled={disabled} aria-label={t("newapi.contextFor", { model: model.id })} onChange={event => patch(model.id, { overrides: { ...model.overrides, contextWindow: event.target.value ? Number(event.target.value) : null } })} />
                      {model.overrides?.contextWindow != null && (
                        <button
                          type="button"
                          disabled={disabled}
                          onClick={() => patch(model.id, { overrides: { ...model.overrides, contextWindow: null } })}
                          className="shrink-0 text-[10px] font-medium text-primary transition-colors hover:text-primary/80 disabled:opacity-40"
                        >
                          {t("clients.limitsAuto")}
                        </button>
                      )}
                    </div>
                    {/* Per-model catalog reference; empty follows the provider mapping. */}
                    <div className="flex items-center gap-1.5">
                      <span className="w-8 shrink-0 text-[9px] uppercase tracking-wider text-muted-foreground/70" title={t("newapi.catalogMapping.modelLevel")}>
                        {t("newapi.mappingShort")}
                      </span>
                      <input
                        type="text"
                        spellCheck={false}
                        autoComplete="off"
                        disabled={disabled}
                        aria-label={t("newapi.catalogMapping.modelLevel")}
                        placeholder={t("newapi.catalogMapping.modelPlaceholder")}
                        className="h-7 w-full min-w-0 rounded-md border border-border/70 bg-background px-1.5 font-mono text-[10px] text-muted-foreground"
                        value={mappingDrafts[model.id] ?? (model.catalogRef ? `${model.catalogRef.providerId}/${model.catalogRef.modelId}` : "")}
                        onChange={event => {
                          const text = event.target.value;
                          setMappingDrafts(current => ({ ...current, [model.id]: text }));
                          // Empty clears immediately; a complete reference commits as typed.
                          if (!text.trim()) commitMapping(model.id, "");
                          else if (text.includes("/")) commitMapping(model.id, text);
                        }}
                        onBlur={event => commitMapping(model.id, event.target.value)}
                      />
                    </div>
                    {/* Per-model compression policy; inherit follows the provider. */}
                    <div className="flex items-center gap-1.5">
                      <span className="w-8 shrink-0 text-[9px] uppercase tracking-wider text-muted-foreground/70" title={t("newapi.compression.modelLevel")}>
                        {t("newapi.compressionShort")}
                      </span>
                      <select
                        disabled={disabled}
                        aria-label={t("newapi.compression.modelLevel")}
                        className="h-7 min-w-0 flex-1 rounded-md border border-border/70 bg-background px-1 text-[10px] text-muted-foreground"
                        value={model.compression?.mode ?? "inherit"}
                        onChange={event => {
                          const mode = event.target.value as CompressionPolicy["mode"];
                          patch(model.id, {
                            compression:
                              mode === "percent" ? { mode, percent: 90 }
                                : mode === "tokens" ? { mode, tokens: 100_000 }
                                : { mode },
                          });
                        }}
                      >
                        <option value="inherit">{t("newapi.compression.inherit")}</option>
                        <option value="percent">{t("newapi.compression.percent")}</option>
                        <option value="tokens">{t("newapi.compression.tokens")}</option>
                        <option value="clientDefault">{t("newapi.compression.clientDefault")}</option>
                      </select>
                      {(model.compression?.mode === "percent" || model.compression?.mode === "tokens") && (
                        <>
                          <input
                            type="number"
                            inputMode="numeric"
                            min={1}
                            max={model.compression.mode === "percent" ? 99 : 10_000_000}
                            disabled={disabled}
                            aria-label={t("newapi.compression.modelLevel")}
                            className="h-7 w-16 shrink-0 rounded-md border border-border/70 bg-background px-1 text-[10px] tabular-nums"
                            value={model.compression.mode === "percent" ? model.compression.percent : model.compression.tokens}
                            onChange={event => {
                              const value = Number(event.target.value);
                              const compression = model.compression;
                              if (!Number.isSafeInteger(value) || value <= 0) return;
                              if (compression?.mode === "percent") {
                                if (value > 99) return;
                                patch(model.id, { compression: { mode: "percent", percent: value } });
                              } else if (compression?.mode === "tokens") {
                                patch(model.id, { compression: { mode: "tokens", tokens: value } });
                              }
                            }}
                          />
                          <span className="shrink-0 text-[9px] text-muted-foreground/70">
                            {t(model.compression.mode === "percent" ? "newapi.percentUnit" : "newapi.tokenUnit")}
                          </span>
                        </>
                      )}
                    </div>
                  </div>
                  <button type="button" disabled={disabled || isDefault} onClick={() => removeModel(model.id)} className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:opacity-30" aria-label={t("newapi.removeModelAria", { model: model.id })}>
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
                            className="h-7 text-xs" disabled={disabled} placeholder={t("clients.limitsAuto")}
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
