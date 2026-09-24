import { useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, Circle, Plus, RefreshCw, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { supportsProtocol } from "@/lib/provider-models";
import { cn } from "@/lib/utils";
import type { ModelProtocol, NewApiModel } from "@/types/newapi";

const protocolKey: Record<ModelProtocol, string> = {
  responses: "newapi.protocolShortResponses",
  chatCompletions: "newapi.protocolShortChat",
  anthropicMessages: "newapi.protocolShortAnthropic",
};

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
  const compatible = models.filter(model => supportsProtocol(model, protocol));
  const selected = compatible.filter(model => model.included).length;

  const patch = (id: string, change: Partial<NewApiModel>) => {
    onModelsChange(models.map(model => model.id === id ? { ...model, ...change } : model));
  };
  const setIncluded = (model: NewApiModel, included: boolean) => {
    patch(model.id, { included });
    if (!included && defaultModel === model.id) {
      onDefaultModelChange(models.find(item => item.id !== model.id && item.included && supportsProtocol(item, protocol))?.id ?? "");
    }
  };
  const addModel = () => {
    const id = newModel.trim();
    if (!id) return;
    const existing = models.find(model => model.id === id);
    if (existing) {
      patch(id, { included: true });
      if (!defaultModel && supportsProtocol(existing, protocol)) onDefaultModelChange(id);
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
      onDefaultModelChange(next.find(model => model.included && supportsProtocol(model, protocol))?.id ?? "");
    }
  };
  const setAll = (included: boolean) => {
    const next = models.map(model => supportsProtocol(model, protocol) ? { ...model, included } : model);
    onModelsChange(next);
    if (!included) onDefaultModelChange("");
    else if (!next.some(model => model.id === defaultModel && model.included)) {
      onDefaultModelChange(next.find(model => model.included && supportsProtocol(model, protocol))?.id ?? "");
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-baseline gap-2">
          <h3 className="text-sm font-semibold">{t("newapi.modelLibrary")}</h3>
          <span className="text-xs tabular-nums text-muted-foreground">{t("newapi.modelsSelected", { selected, total: compatible.length })}</span>
        </div>
        <div className="ml-auto flex items-center gap-1">
          <Button type="button" variant="ghost" size="sm" className="h-7 px-2 text-xs" disabled={disabled || compatible.length === 0} onClick={() => setAll(selected !== compatible.length)}>
            {t(selected === compatible.length && compatible.length > 0 ? "newapi.clearModels" : "newapi.selectAllModels")}
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
          <div className="hidden grid-cols-[32px_minmax(150px,1.25fr)_minmax(130px,1fr)_90px_130px_36px] gap-2 bg-muted/50 px-3 py-2 text-[11px] font-medium text-muted-foreground md:grid">
            <span className="sr-only">{t("newapi.includeModel")}</span>
            <span>{t("newapi.modelId")}</span>
            <span>{t("newapi.menuDisplayName")}</span>
            <span>{t("newapi.protocol")}</span>
            <span>{t("newapi.contextWindowShort")}</span>
            <span className="sr-only">{t("newapi.removeModel")}</span>
          </div>
          <div className="max-h-[310px] overflow-y-auto">
            {models.map(model => {
              const supported = supportsProtocol(model, protocol);
              const isDefault = defaultModel === model.id;
              return (
                <div key={model.id} className={cn("grid gap-2 border-t px-3 py-2.5 first:border-t-0 md:grid-cols-[32px_minmax(150px,1.25fr)_minmax(130px,1fr)_90px_130px_36px] md:items-center", !supported && "bg-muted/25 opacity-65")}>
                  <div className="flex items-center gap-2 md:block">
                    <Checkbox checked={model.included && supported} disabled={disabled || !supported || isDefault} onCheckedChange={value => setIncluded(model, value === true)} aria-label={t("newapi.includeModelAria", { model: model.id })} />
                    <span className="text-xs text-muted-foreground md:hidden">{t("newapi.includeModel")}</span>
                  </div>
                  <div className="flex min-w-0 items-center gap-1.5">
                    <button
                      type="button"
                      disabled={disabled || !supported || !model.included}
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
                  <Input className="h-8 text-xs" value={model.displayName === model.id && providerName.trim() ? `${providerName.trim()}/${model.id}` : model.displayName} disabled={disabled || !supported} aria-label={t("newapi.menuDisplayNameFor", { model: model.id })} onChange={event => patch(model.id, { displayName: event.target.value })} />
                  <Badge variant="secondary" className="w-fit text-[10px]">{t(protocolKey[protocol])}</Badge>
                  <div className="space-y-0.5">
                    <Input className="h-8 text-xs tabular-nums" type="number" min={1024} max={100000000} inputMode="numeric" value={model.contextWindow ?? ""} disabled={disabled} aria-label={t("newapi.contextFor", { model: model.id })} onChange={event => patch(model.id, { contextWindow: event.target.value ? Number(event.target.value) : null, metadataSource: "manual" })} />
                  </div>
                  <button type="button" disabled={disabled || isDefault} onClick={() => removeModel(model.id)} className="flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:opacity-30" aria-label={t("newapi.removeModelAria", { model: model.id })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </button>
                  {!supported && <p className="text-[10px] text-muted-foreground md:col-start-2 md:col-span-4">{t("newapi.protocolMismatchModel")}</p>}
                </div>
              );
            })}
          </div>
        </div>
      )}
      <p className="text-xs leading-relaxed text-muted-foreground">{t(protocol === "responses" ? "newapi.codexCatalogHint" : "newapi.otherClientCatalogHint")}</p>
    </div>
  );
}
