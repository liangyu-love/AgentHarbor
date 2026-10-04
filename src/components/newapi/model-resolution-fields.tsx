import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";
import type { CompressionPlan, CompressionPolicy, NewApiModel, ProviderModelResolution, ResolvedField } from "@/types/newapi";

interface Props {
  model: NewApiModel;
  resolution?: ProviderModelResolution;
  loading: boolean;
  error: boolean;
  disabled: boolean;
  onChange: (change: Partial<NewApiModel>) => void;
}

function compactTokens(value: number): string {
  const divisor = value >= 1_000_000 ? 1_000_000 : value >= 1_000 ? 1_000 : 1;
  return `${Number((value / divisor).toFixed(3))}${divisor === 1_000_000 ? "M" : divisor === 1_000 ? "K" : ""}`;
}

export function resolvedLimitPlaceholder(field: ResolvedField<number> | undefined, locale: string, unavailable: string): string {
  return field?.value == null ? unavailable : field.value.toLocaleString(locale);
}

// This component only presents resolver output. Empty controls stay automatic;
// neither matched catalog references nor computed limits become saved overrides.
export function ModelResolutionFields({ model, resolution, loading, error, disabled, onChange }: Props) {
  const { t, i18n } = useTranslation();
  const [mappingDraft, setMappingDraft] = useState<string | null>(null);
  const unavailable = t(loading ? "newapi.modelResolution.loading" : error ? "newapi.modelResolution.unavailable" : `newapi.matchKind.${resolution?.matchKind ?? "unmatched"}`);
  const context = resolvedLimitPlaceholder(resolution?.context, i18n.language, unavailable);
  const catalog = resolution?.matched ? `${resolution.matched.providerId}/${resolution.matched.modelId}` : unavailable;
  const plan = resolution?.projection.compression;
  const compressionLabel = (value: CompressionPlan | undefined) => {
    if (!value) return unavailable;
    const source = t(`newapi.modelResolution.${value.source}`);
    if (value.state === "clientDefault") return `${source} · ${t("newapi.compression.clientDefault")}`;
    if (value.state !== "computed" || value.threshold == null) return `${source} · ${t("newapi.modelResolution.unavailable")}`;
    const threshold = `${value.budgetSource === "fallback" ? `${t("newapi.modelResolution.fallback")} ` : ""}${compactTokens(value.threshold)}`;
    return `${source} ${value.mode === "percent" ? `${value.percent}% · ` : ""}${threshold}`;
  };
  const effectiveCompression = compressionLabel(plan);
  const inheritLabel = (model.compression?.mode ?? "inherit") === "inherit" ? effectiveCompression : t("newapi.compression.inherit");
  const commitMapping = (text: string) => {
    const value = text.trim();
    if (!value) {
      setMappingDraft(null);
      onChange({ catalogRef: null });
      return;
    }
    const [provider, ...rest] = value.split("/");
    const providerId = provider.trim();
    const modelId = rest.join("/").trim();
    if (!providerId || !modelId) return;
    setMappingDraft(null);
    onChange({ catalogRef: { providerId: providerId.toLowerCase(), modelId } });
  };

  return <div className="min-w-0 space-y-1.5" data-model-resolution={loading ? "loading" : error ? "error" : resolution?.matchKind ?? "unmatched"}>
    <div className="flex items-center gap-1.5">
      <span className="w-8 shrink-0 text-[10px] text-muted-foreground" title={t("newapi.contextWindowShort")}>{t("newapi.limitShort")}</span>
      <Input className="h-7 min-w-0 text-xs tabular-nums placeholder:text-foreground/75" type="number" min={1024} max={100000000} inputMode="numeric"
        placeholder={context} value={model.overrides?.contextWindow ?? ""} disabled={disabled}
        title={resolution?.context.value != null ? `${context} · ${t(`newapi.capabilitySource.${resolution.context.source}`)}` : context}
        aria-label={t("newapi.contextFor", { model: model.id })}
        onChange={event => onChange({ overrides: { ...model.overrides, contextWindow: event.target.value ? Number(event.target.value) : null } })} />
      {model.overrides?.contextWindow != null && <button type="button" disabled={disabled}
        onClick={() => onChange({ overrides: { ...model.overrides, contextWindow: null } })}
        className="shrink-0 rounded text-[10px] font-medium text-primary hover:text-primary/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40">
        {t("clients.limitsAuto")}
      </button>}
    </div>
    <div className="flex items-center gap-1.5">
      <span className="w-8 shrink-0 text-[10px] text-muted-foreground" title={t("newapi.catalogMapping.modelLevel")}>{t("newapi.mappingShort")}</span>
      <Input type="text" spellCheck={false} autoComplete="off" disabled={disabled}
        aria-label={t("newapi.catalogMapping.modelLevel")} placeholder={catalog} title={catalog}
        className="h-7 min-w-0 px-2 font-mono text-[10px] placeholder:text-foreground/75"
        value={mappingDraft ?? (model.catalogRef ? `${model.catalogRef.providerId}/${model.catalogRef.modelId}` : "")}
        onChange={event => {
          const text = event.target.value;
          setMappingDraft(text);
          if (!text.trim() || text.includes("/")) commitMapping(text);
        }}
        onBlur={event => commitMapping(event.target.value)} />
    </div>
    <div className="flex items-center gap-1.5">
      <span className="w-8 shrink-0 text-[10px] text-muted-foreground" title={t("newapi.compression.modelLevel")}>{t("newapi.compressionShort")}</span>
      <select disabled={disabled} aria-label={t("newapi.compression.modelLevel")}
        title={plan?.threshold != null ? `${effectiveCompression} (${plan.threshold.toLocaleString(i18n.language)})` : effectiveCompression}
        className="h-7 min-w-0 flex-1 rounded-md border border-input bg-background px-1.5 text-[11px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
        value={model.compression?.mode ?? "inherit"}
        onChange={event => {
          const mode = event.target.value as CompressionPolicy["mode"];
          onChange({ compression: mode === "percent" ? { mode, percent: 90 } : mode === "tokens" ? { mode, tokens: 100_000 } : { mode } });
        }}>
        <option value="inherit">{inheritLabel}</option>
        <option value="percent">{t("newapi.compression.percent")}</option>
        <option value="tokens">{t("newapi.compression.tokens")}</option>
        <option value="clientDefault">{t("newapi.compression.clientDefault")}</option>
      </select>
      {(model.compression?.mode === "percent" || model.compression?.mode === "tokens") && <>
        <Input type="number" inputMode="numeric" min={1} max={model.compression.mode === "percent" ? 99 : 10_000_000} disabled={disabled}
          aria-label={t("newapi.compression.modelLevel")} className="h-7 w-16 shrink-0 px-1 text-[11px] tabular-nums"
          value={model.compression.mode === "percent" ? model.compression.percent : model.compression.tokens}
          onChange={event => {
            const value = Number(event.target.value);
            if (!Number.isSafeInteger(value) || value <= 0) return;
            if (model.compression?.mode === "percent" && value <= 99) onChange({ compression: { mode: "percent", percent: value } });
            else if (model.compression?.mode === "tokens" && value <= 10_000_000) onChange({ compression: { mode: "tokens", tokens: value } });
          }} />
        <span className="shrink-0 text-[10px] text-muted-foreground">{t(model.compression.mode === "percent" ? "newapi.percentUnit" : "newapi.tokenUnit")}</span>
      </>}
    </div>
  </div>;
}
