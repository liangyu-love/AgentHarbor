import { useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  Brain,
  ChevronDown,
  Cpu,
  Database,
  DollarSign,
  Gauge,
  Layers,
  RefreshCw,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { invalidateClaudeDesktop } from "@/lib/claude-desktop";
import { formatInvokeError } from "@/lib/invoke-error";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { CapabilityClearControls } from "./capability-clear-controls";
import { compactTokens } from "./model-resolution-fields";
import { relayHue } from "./relay-groups";
import { cn } from "@/lib/utils";
import type {
  ClientProjection,
  CostRates,
  CompressionPlan,
  MatchKind,
  ModelCapability,
  PriceInfo,
  ResolvedField,
  ModelsDevCatalogStatus,
} from "@/types/newapi";

type Field = ResolvedField<unknown>;

function Origin({ field }: { field: Field }) {
  const { t } = useTranslation();
  const reference = field.sourceRef ? ` · ${field.sourceRef.providerId}/${field.sourceRef.modelId}` : "";
  return (
    <div className="mt-1 flex flex-wrap items-center gap-1 font-mono text-[10px] text-muted-foreground/80">
      <span>{t("newapi.capabilitySource." + field.source, { defaultValue: field.source })}</span>
      {reference && <span className="break-all">{reference}</span>}
      <span>·</span>
      <span>
        {field.updatedAt
          ? new Date(field.updatedAt).toLocaleString()
          : t("newapi.capabilityDateUnknown")}
      </span>
    </div>
  );
}

const matchKindTone: Record<MatchKind, string> = {
  explicit: "border-primary/40 text-primary",
  exact: "border-emerald-500/40 text-emerald-600 dark:text-emerald-400",
  normalized: "border-sky-500/40 text-sky-600 dark:text-sky-400",
  consensus: "border-border text-muted-foreground",
  ambiguous: "border-amber-500/50 text-amber-600 dark:text-amber-400",
  invalid: "border-destructive/40 text-destructive",
  unmatched: "border-border text-muted-foreground",
};

function number(value: number | null | undefined) {
  return value === null || value === undefined ? "—" : value.toLocaleString();
}

/** Prices keep enough decimals to stay meaningful without float noise. */
function priceNumber(value: number | null | undefined) {
  if (value === null || value === undefined) return "—";
  if (!Number.isFinite(value)) return "—";
  return Number(value.toFixed(4)).toString();
}

function CompressionLine({ plan, applied, model }: { plan: CompressionPlan; applied: boolean | null; model: ModelCapability }) {
  const { t } = useTranslation();
  const mode =
    plan.mode === "percent"
      ? `${plan.percent}%`
      : plan.mode === "tokens"
        ? number(plan.tokens)
        : plan.mode;
  const state =
    plan.state === "computed"
      ? plan.threshold !== null
        ? t("newapi.capabilityCompressionThreshold", { value: number(plan.threshold) })
        : t("newapi.capabilityCompressionUnknown")
      : plan.state === "clientDefault"
        ? t("newapi.capabilityCompressionClient")
        : plan.state === "invalid"
          ? t("newapi.capabilityCompressionInvalid")
          : t("newapi.capabilityCompressionUnknown");
  const appliedLabel =
    applied === true
      ? t("newapi.capabilityCompressionAppliedNow")
      : applied === false
        ? t("newapi.capabilityCompressionPendingSync")
        : t("newapi.capabilityCompressionSyncUnknown");
  return (
    <div className="cap-panel">
      <h4 className="cap-label">
        <Gauge className="h-3.5 w-3.5 text-primary" />
        <span>{t("newapi.capabilityCompression")}</span>
        {plan.threshold !== null && model.compressionScope === "routerModel" && (
          <span
            className={cn(
              "rounded px-1.5 py-0 text-[10px] font-semibold",
              applied === true
                ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"
                : "bg-amber-500/10 text-amber-700 dark:text-amber-400",
            )}
          >
            {appliedLabel}
          </span>
        )}
      </h4>
      <p className="cap-value">{state}</p>
      {plan.threshold !== null && plan.threshold > model.codexTotalCompactThreshold && <p className="mt-1 text-xs text-muted-foreground">
        {t("newapi.capabilityClearCodexCompression")}: {number(model.codexTotalCompactThreshold)}
      </p>}
      {model.compressionScope === "nativeShared" && (
        <p className="mt-1 text-[10px] text-muted-foreground">
          {t("newapi.capabilityCompressionShared", {
            planned: number(model.compressionSharedThreshold),
            actual: number(model.compressionAppliedThreshold),
          })}
          {(!model.compressionSharedPlan || model.compressionSharedPlan.state === "computed") && ` · ${t(model.compressionSharedApplied ? "newapi.capabilityCompressionAppliedNow" : "newapi.capabilityCompressionPendingSync")}`}
          {model.compressionLimitingModels.length > 0 && ` · ${t("newapi.capabilityCompressionLimitingModels", { models: model.compressionLimitingModels.join(", ") })}`}
          {model.compressionSharedPlan && model.compressionSharedPlan.state !== "computed" && <span className="block text-amber-600 dark:text-amber-400">
            {t("newapi.capabilityCompressionSharedState." + model.compressionSharedPlan.state)}
            {model.compressionSharedPlan.blockingModels.length > 0 && ` · ${model.compressionSharedPlan.blockingModels.join(", ")}`}
          </span>}
        </p>
      )}
      {model.compressionScope === "inactive" && <p className="mt-1 text-[10px] text-muted-foreground">{t("newapi.capabilityCompressionInactive")}</p>}
      {model.compressionScope === "inactive" && model.compressionWrittenThreshold != null && <p className="text-[10px] text-muted-foreground">
        {t("newapi.capabilityCompressionWritten", { value: number(model.compressionWrittenThreshold) })}
      </p>}
      <p className="font-mono text-[10px] text-muted-foreground">
        {mode} · {plan.source}
        {plan.budget !== null
          ? ` · ${t("newapi.capabilityCompressionHint", {
              budget: number(plan.budget),
              source: plan.budgetSource,
            })}`
          : ""}
      </p>
      {plan.note && (
        <p className="mt-0.5 text-[10px] text-muted-foreground/80">{plan.note}</p>
      )}
    </div>
  );
}

function RateDetails({ rates, includeText = false }: { rates: CostRates; includeText?: boolean }) {
  const { t } = useTranslation();
  const keys: (keyof CostRates)[] = includeText ? ["input", "output", "cacheRead", "cacheWrite", "reasoning", "inputAudio", "outputAudio"]
    : ["cacheRead", "cacheWrite", "reasoning", "inputAudio", "outputAudio"];
  return <p className="mt-1 text-[10px] text-muted-foreground">
    {keys.filter(key => rates[key] != null).map(key => `${t("newapi.priceRates." + key)} ${priceNumber(rates[key])}`).join(" · ")}
  </p>;
}

function PriceLine({
  price,
  editing,
  onEdit,
}: {
  price: PriceInfo;
  editing: boolean;
  onEdit: () => void;
}) {
  const { t } = useTranslation();
  // A real zero rate is a price, not "unknown"; only missing rates are unknown.
  const empty = price.input == null && price.output == null && price.inputAudio == null && price.outputAudio == null;
  return (
    <div className="cap-panel">
      <h4 className="cap-label justify-between">
        <span className="flex items-center gap-1.5">
          <DollarSign className="h-3.5 w-3.5 text-primary" />
          <span>{t("newapi.capabilityPrice")}</span>
        </span>
        <button
          type="button"
          onClick={onEdit}
          aria-expanded={editing}
          className="text-[10px] font-medium text-primary transition-colors hover:text-primary/80"
        >
          {t("newapi.priceEdit")}
        </button>
      </h4>
      {empty ? (
        <p className="mt-1 text-xs text-muted-foreground">
          {t("newapi.capabilitySource." + price.source, { defaultValue: price.source })}
          {price.note ? ` · ${price.note}` : ""}
        </p>
      ) : (
        <>
          <p className="cap-value">
            {priceNumber(price.input)} / {priceNumber(price.output)}
            <span className="ml-1 text-[10px] font-normal text-muted-foreground">
              {t("newapi.capabilityPricePerMillion")}
            </span>
          </p>
          {!price.complete && (
            <p className="text-[10px] text-amber-600 dark:text-amber-400">
              {t("newapi.capabilityPriceIncomplete")}
            </p>
          )}
          <RateDetails rates={price} />
          {(price.inputAudio != null || price.outputAudio != null || price.contextOver200k?.inputAudio != null || price.contextOver200k?.outputAudio != null
            || price.tiers?.some(tier => tier.inputAudio != null || tier.outputAudio != null)) && <p className="text-[10px] text-amber-600 dark:text-amber-400">
              {t("newapi.capabilityPriceAudioUnsupported")}
            </p>}
          {price.contextOver200k && <div className="mt-1 text-[10px] text-muted-foreground">
            {t("newapi.capabilityPriceOver200k")}<RateDetails rates={price.contextOver200k} includeText />
          </div>}
          {price.tiers?.map((tier, index) => <div key={index} className="mt-1 text-[10px] text-muted-foreground">
            {tier.tier.type === "context" ? t("newapi.capabilityPriceTier", { value: number(tier.tier.size) }) : tier.tier.type}
            <RateDetails rates={tier} includeText />
          </div>)}
          <div className="mt-1 flex flex-wrap items-center gap-1 font-mono text-[10px] text-muted-foreground/80">
            <span>
              {price.sourceRef
                ? `${price.sourceRef.providerId}/${price.sourceRef.modelId}`
                : "—"}
            </span>
            <span>·</span>
            <span>
              {t("newapi.capabilitySource." + price.source, { defaultValue: price.source })}
            </span>
            {price.multiplier != null && <span>· {t("newapi.priceMultiplierApplied", { value: price.multiplier })}</span>}
          </div>
        </>
      )}
    </div>
  );
}

/** Explicit price override: estimated cost only, never a billing reference. */
function PriceEditor({
  providerId,
  modelId,
  price,
  onClose,
  onSaved,
}: {
  providerId: string;
  modelId: string;
  price: PriceInfo;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useTranslation();
  const manual = price.source === "manual";
  const [rates, setRates] = useState({
    input: manual && price.input != null ? String(price.input) : "",
    output: manual && price.output != null ? String(price.output) : "",
    cacheRead: manual && price.cacheRead != null ? String(price.cacheRead) : "",
    cacheWrite: manual && price.cacheWrite != null ? String(price.cacheWrite) : "",
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const parse = (text: string) => {
    if (!text.trim()) return null;
    const value = Number(text);
    return Number.isFinite(value) && value >= 0 ? value : undefined;
  };
  const submit = async () => {
    const parsed = {
      input: parse(rates.input),
      output: parse(rates.output),
      cacheRead: parse(rates.cacheRead),
      cacheWrite: parse(rates.cacheWrite),
    };
    if (
      Object.values(parsed).some(value => value === undefined) ||
      (parsed.input == null && parsed.output == null)
    ) {
      setError(t("newapi.priceInvalid"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.routing.setModelPrice(providerId, modelId, {
        input: parsed.input,
        output: parsed.output,
        cacheRead: parsed.cacheRead,
        cacheWrite: parsed.cacheWrite,
      });
      onSaved();
      onClose();
    } catch (cause) {
      setError(formatInvokeError(cause));
    } finally {
      setBusy(false);
    }
  };
  const clear = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.routing.setModelPrice(providerId, modelId, null);
      onSaved();
      onClose();
    } catch (cause) {
      setError(formatInvokeError(cause));
    } finally {
      setBusy(false);
    }
  };
  const fields: Array<{ key: keyof typeof rates; label: string }> = [
    { key: "input", label: t("newapi.priceInput") },
    { key: "output", label: t("newapi.priceOutput") },
    { key: "cacheRead", label: t("newapi.priceCacheRead") },
    { key: "cacheWrite", label: t("newapi.priceCacheWrite") },
  ];
  return (
    <div className="rounded-xl border border-primary/30 bg-primary/[0.04] p-3">
      <p className="text-[11px] font-semibold text-foreground">{t("newapi.priceEditorTitle")}</p>
      <div className="mt-2 grid gap-2 sm:grid-cols-4">
        {fields.map(field => (
          <label key={field.key} className="space-y-1">
            <span className="block text-[10px] text-muted-foreground">{field.label}</span>
            <input
              type="number"
              min={0}
              step="0.0001"
              inputMode="decimal"
              disabled={busy}
              value={rates[field.key]}
              placeholder="0"
              onChange={event =>
                setRates(current => ({ ...current, [field.key]: event.target.value }))
              }
              className="h-7 w-full rounded-md border border-border/70 bg-background px-1.5 font-mono text-[11px] tabular-nums"
            />
          </label>
        ))}
      </div>
      <p className="mt-2 text-[10px] leading-relaxed text-muted-foreground">
        {t("newapi.priceManualNote")}
      </p>
      {error && (
        <p role="alert" className="mt-1 text-[11px] text-destructive">
          {error}
        </p>
      )}
      <div className="mt-2 flex justify-end gap-2">
        <Button
          variant="ghost"
          size="sm"
          disabled={busy || !manual}
          onClick={() => void clear()}
          className="h-7 rounded-lg px-2 text-[11px]"
        >
          {t("newapi.priceClear")}
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={busy}
          onClick={onClose}
          className="h-7 rounded-lg px-2 text-[11px]"
        >
          {t("common.cancel")}
        </Button>
        <Button
          size="sm"
          disabled={busy}
          onClick={() => void submit()}
          className="h-7 rounded-lg px-2.5 text-[11px]"
        >
          {t("newapi.save")}
        </Button>
      </div>
    </div>
  );
}

function CapabilityCard({ model, onSaved }: { model: ModelCapability; onSaved: () => void }) {
  const { t } = useTranslation();
  const [priceEditing, setPriceEditing] = useState(false);
  const projection: ClientProjection = model.projection;
  const levels = model.reasoning.value ?? [];
  const conflicts = model.conflicts;
  return (
    <details className="cap-row group bg-card" data-capability-row={model.providerId + "::" + model.upstreamModel}>
      <summary className="cap-summary">
        <span aria-hidden className="agg-chip-mark" style={{ "--relay-hue": relayHue(model.providerId) } as CSSProperties}>
          {model.providerName.slice(0, 1).toUpperCase()}
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="truncate text-[13px] font-semibold text-foreground">{model.displayName}</span>
            <span className="text-muted-foreground/60">→</span>
            <span className="min-w-0 break-all font-mono text-[12px] font-semibold text-primary">{model.matchedModel}</span>
          </span>
          <span className="block truncate text-[11px] text-muted-foreground">{model.providerName}</span>
        </span>
        <span className="cap-quick" aria-hidden>
          <span>{t("newapi.capabilityQuickContext", { value: model.context.value == null ? "—" : compactTokens(model.context.value) })}</span>
          <span>{t("newapi.capabilityQuickOutput", { value: model.output.value == null ? "—" : compactTokens(model.output.value) })}</span>
          {(model.price.input != null || model.price.output != null) && (
            <span>${priceNumber(model.price.input)} / ${priceNumber(model.price.output)}</span>
          )}
        </span>
        <Badge variant="outline" className={cn("text-[10px] font-semibold", matchKindTone[model.matchKind])}>
          {t("newapi.matchKind." + model.matchKind, { defaultValue: model.matchKind })}
        </Badge>
        {conflicts.length > 0 && (
          <Badge variant="outline" className="border-amber-500/50 text-[10px] font-semibold text-amber-600 dark:text-amber-400">
            <AlertTriangle className="mr-1 h-3 w-3" />
            {conflicts.length}
          </Badge>
        )}
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200 group-open:rotate-180" />
      </summary>

      <div className="cap-body">
        <p className="break-words font-mono text-[11px] text-muted-foreground">
          {t("newapi.upstreamModel")}:{" "}
          <span className="font-semibold text-foreground">{model.upstreamModel}</span>
        </p>

        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <div className="cap-panel">
            <h4 className="cap-label">
              <Brain className="h-3.5 w-3.5 text-primary" />
              <span>{t("newapi.capabilityReasoning")}</span>
            </h4>
            <p className="mt-1 font-mono text-xs font-semibold text-foreground">
              {levels.map((v) => v.effort).join(" / ") || "—"}
            </p>
            {model.defaultReasoning && (
              <p className="font-mono text-[10px] text-muted-foreground">
                {t("newapi.capabilityDefaultEffort")}: {model.defaultReasoning} ·{" "}
                {t("newapi.capabilitySource." + model.defaultReasoningSource, {
                  defaultValue: model.defaultReasoningSource,
                })}
              </p>
            )}
            {model.catalogHints.length === 1 && model.reasoningOptions.length > 0 && (
              <p className="font-mono text-[10px] text-muted-foreground/80">
                {t("newapi.capabilityReasoningOptions")}:{" "}
                {model.reasoningOptions
                  .map((option) =>
                    option.kind === "effort"
                      ? `effort(${option.values.join("/")})`
                      : option.kind === "budgetTokens"
                        ? `budget(${option.min ?? "?"}…)`
                        : "toggle",
                  )
                  .join(", ")}
              </p>
            )}
            <Origin field={model.reasoning} />
          </div>

          <div className="cap-panel">
            <h4 className="cap-label">
              <Layers className="h-3.5 w-3.5 text-primary" />
              <span>{t("newapi.capabilityContext")}</span>
            </h4>
            <p className="cap-value">
              {number(model.context.value)}{" "}
              <span className="text-[10px] font-normal text-muted-foreground">
                {t("newapi.tokenUnit")}
              </span>
            </p>
            {model.inputLimit.value !== null && (
              <p className="font-mono text-[10px] text-muted-foreground">
                {t("newapi.capabilityInput")}: {number(model.inputLimit.value)}
              </p>
            )}
            <Origin field={model.context} />
          </div>

          <div className="cap-panel">
            <h4 className="cap-label">
              <Sparkles className="h-3.5 w-3.5 text-primary" />
              <span>{t("newapi.capabilityOutput")}</span>
            </h4>
            <p className="cap-value">
              {number(model.output.value)}{" "}
              <span className="text-[10px] font-normal text-muted-foreground">
                {t("newapi.tokenUnit")}
              </span>
            </p>
            <Origin field={model.output} />
          </div>

          <PriceLine
            price={model.price}
            editing={priceEditing}
            onEdit={() => setPriceEditing(current => !current)}
          />
        </div>

        {priceEditing && (
          <PriceEditor
            providerId={model.providerId}
            modelId={model.upstreamModel}
            price={model.price}
            onClose={() => setPriceEditing(false)}
            onSaved={onSaved}
          />
        )}

        <div className="grid gap-3 lg:grid-cols-3">
          <CompressionLine plan={projection.compression} applied={model.compressionApplied} model={model} />
          <div className="cap-panel">
            <h4 className="cap-label">{t("newapi.capabilityMatch")}</h4>
            <p className="mt-1 font-mono text-[11px] text-foreground">
              {t("newapi.capabilityMatchedRef")}:{" "}
              {model.matched ? `${model.matched.providerId}/${model.matched.modelId}` : "—"}
            </p>
            {projection.contextIsFallback && (
              <p className="mt-0.5 text-[10px] text-muted-foreground/80">
                {t("newapi.capabilityFallbackNote")}
              </p>
            )}
          </div>
          <div className="cap-panel">
            <h4 className="cap-label">{t("newapi.capabilityConflicts")}</h4>
            {conflicts.length === 0 ? (
              <p className="mt-1 text-[11px] text-muted-foreground">
                {t("newapi.capabilityNoConflicts")}
              </p>
            ) : (
              <>
              <p className="mt-1 text-[10px] leading-relaxed text-muted-foreground">{t("newapi.capabilityConflictHint")}</p>
              <ul className="mt-1 space-y-1">
                {conflicts.map((conflict) => (
                  <li key={conflict.field} className="font-mono text-[10px] text-amber-600 dark:text-amber-400">
                    {t("newapi.capabilityField." + conflict.field, {
                      defaultValue: conflict.field,
                    })}
                    :{" "}
                    {conflict.candidates
                      .map((candidate) => `${candidate.value} (${candidate.modelRef.providerId}/${candidate.modelRef.modelId})`)
                      .join(" / ")}
                  </li>
                ))}
              </ul>
              </>
            )}
          </div>
        </div>

        <div className="cap-panel cap-panel-quiet">
          <h4 className="cap-label">{t("newapi.capabilityMatchSteps")}</h4>
          <ul className="mt-2 space-y-1">
            {model.steps.map((step) => (
              <li key={step.step + step.modelId} className="font-mono text-[10px] text-muted-foreground">
                <span className="text-foreground">{step.modelId}</span> · {t("newapi.capabilityMatchStep." + step.step, { defaultValue: step.step })} · {step.note}
              </li>
            ))}
          </ul>
        </div>

        {model.catalogHints.length > 0 && <div className="cap-panel cap-panel-quiet">
          <h4 className="cap-label">{t("newapi.capabilityCatalogIdentity")}</h4>
          <ul className="mt-2 space-y-2">
            {model.catalogHints.map(hint => <li key={`${hint.source.providerId}/${hint.source.modelId}`} className="font-mono text-[10px] text-muted-foreground">
              <span className="font-semibold text-foreground">{hint.source.providerId}/{hint.source.modelId}</span>
              {" → "}{hint.canonicalRaw ?? "—"}
              {" · "}{t("newapi.capabilityCanonicalStatus." + hint.canonicalStatus)}
              <span className="block">{t("newapi.capabilityReasoningOptions")}: {t("newapi.capabilityOptionsPresence." + hint.reasoningOptionsPresence)}
                {hint.reasoningOptions.length > 0 && ` · ${hint.reasoningOptions.map(option => option.kind === "effort"
                  ? `effort(${option.values.join("/")})` : option.kind === "budgetTokens"
                    ? `budget_tokens(${option.min ?? "?"}–${option.max ?? "?"})` : "toggle").join(", ")}`}
              </span>
              {hint.interleaved !== null && <span className="block">interleaved: {typeof hint.interleaved === "boolean"
                ? String(hint.interleaved) : `{ field: ${hint.interleaved.field} }`}</span>}
            </li>)}
          </ul>
        </div>}

        <CapabilityClearControls model={model} onSaved={onSaved} />
      </div>
    </details>
  );
}

function CatalogNotice({ catalog, children }: { catalog: ModelsDevCatalogStatus; children?: ReactNode }) {
  const { t } = useTranslation();
  const origin = t(`newapi.catalogOrigin${catalog.origin === "runtime" ? "Runtime" : "Embedded"}`);
  return (
    <div className="cap-catalog">
      <Database aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1">
        <span>
          {t("newapi.catalogInfo", {
            origin,
            providers: catalog.providers,
            models: catalog.models,
            revision: catalog.revision.slice(0, 8),
          })}
        </span>
        {" · "}
        <span>
          {catalog.fetchedAt
            ? t("newapi.catalogFetchedAt", {
                time: new Date(catalog.fetchedAt * 1000).toLocaleString(),
              })
            : t("newapi.catalogNever")}
        </span>
        {catalog.error && (
          <span className="block text-amber-600 dark:text-amber-400">
            {t("newapi.catalogError", { error: catalog.error })}
          </span>
        )}
      </span>
      {children}
    </div>
  );
}

export function ModelCapabilities() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<"all" | "unmatched" | "conflicts">("all");
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<string | null>(null);

  const query = useQuery({
    queryKey: ["model-capabilities"],
    queryFn: api.routing.capabilities,
    enabled: open,
    refetchInterval: open ? 30_000 : false,
  });

  const allModels = useMemo(() => query.data?.models ?? [], [query.data]);
  const models = useMemo(
    () =>
      allModels.filter((model) => {
        if (filter === "unmatched" && model.matchKind !== "unmatched") return false;
        if (filter === "conflicts" && model.conflicts.length === 0) return false;
        const haystack = [
          model.providerName,
          model.displayName,
          model.upstreamModel,
          model.matchedModel,
          ...(model.matched ? [`${model.matched.providerId}/${model.matched.modelId}`] : []),
        ];
        return haystack.some((value) => value.toLowerCase().includes(search.toLowerCase()));
      }),
    [allModels, filter, search],
  );

  const counts = {
    all: allModels.length,
    unmatched: allModels.filter((model) => model.matchKind === "unmatched").length,
    conflicts: allModels.filter((model) => model.conflicts.length > 0).length,
  };

  const refresh = async () => {
    setRefreshing(true);
    setRefreshError(null);
    try {
      await api.routing.refreshCatalog();
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["model-capabilities"] }),
        invalidateClaudeDesktop(queryClient),
      ]);
    } catch (error) {
      setRefreshError(formatInvokeError(error));
    } finally {
      setRefreshing(false);
    }
  };

  return (
    <section
      className="harbor-card"
      aria-label={t("newapi.modelCapabilities")}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="diag-toggle">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-background text-foreground ring-1 ring-border">
            <Cpu className="h-[18px] w-[18px]" />
          </span>
          <span className="font-display text-[17px] font-semibold tracking-tight">{t("newapi.modelCapabilities")}</span>
          <ChevronDown aria-hidden className={cn("h-4 w-4 text-muted-foreground transition-transform duration-200", open && "rotate-180")} />
        </button>

        {open && query.data && (
          <span className="rounded-full bg-muted/80 px-2.5 py-0.5 font-mono text-[11px] font-semibold text-muted-foreground">
            {t("newapi.capabilityCount", { shown: models.length, total: allModels.length })}
          </span>
        )}
      </div>

      {open && (
        <div className="mt-5 space-y-4 border-t border-border/60 pt-4">
          {query.data && (
            <CatalogNotice catalog={query.data.catalog}>
              <Button
                variant="outline"
                size="sm"
                disabled={refreshing || query.isFetching}
                onClick={() => void refresh()}
                className="h-8 gap-1.5 rounded-xl px-3 text-xs font-semibold"
              >
                <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin text-primary")} />
                <span>{t(refreshing ? "newapi.catalogRefreshing" : "newapi.catalogRefresh")}</span>
              </Button>
            </CatalogNotice>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <div className="relative min-w-[12rem] flex-1">
              <Search className="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                aria-label={t("newapi.capabilitySearch")}
                placeholder={t("newapi.capabilitySearch")}
                className="h-9 rounded-xl bg-card pl-8 pr-8 text-xs"
              />
              {search && (
                <button
                  type="button"
                  onClick={() => setSearch("")}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>

            <div className="agg-segmented !grid-cols-3 !p-1" role="group" aria-label={t("newapi.capabilityFilter")}>
              {(["all", "unmatched", "conflicts"] as const).map((value) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={filter === value}
                  onClick={() => setFilter(value)}
                  className="cap-filter"
                >
                  {t(
                    value === "all"
                      ? "newapi.capabilityFilterAll"
                      : value === "unmatched"
                        ? "newapi.capabilityFilterUnmatched"
                        : "newapi.capabilityFilterConflicts",
                  )}
                  <span aria-hidden className="cap-filter-count">{counts[value]}</span>
                </button>
              ))}
            </div>
          </div>

          {refreshError && (
            <div
              role="alert"
              className="rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs font-medium text-destructive"
            >
              {refreshError}
            </div>
          )}

          {query.error && (
            <div
              role="alert"
              className="rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs font-medium text-destructive"
            >
              {formatInvokeError(query.error)}
            </div>
          )}

          {query.data?.stale && (
            <div
              role="status"
              className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-xs font-medium text-amber-700 dark:text-amber-300"
            >
              <AlertTriangle className="h-4 w-4 shrink-0 text-amber-500 mt-0.5" />
              <span>{t("newapi.catalogStale")}</span>
            </div>
          )}

          <div className="cap-list">
            {models.map((model) => (
              <CapabilityCard
                key={model.providerId + "::" + model.upstreamModel}
                model={model}
                onSaved={() => {
                  void queryClient.invalidateQueries({ queryKey: ["model-capabilities"] });
                  void invalidateClaudeDesktop(queryClient);
                }}
              />
            ))}
          </div>

          {!query.isLoading && !models.length && (
            <div className="rounded-xl border border-dashed border-border/70 p-8 text-center text-muted-foreground">
              <p className="text-xs font-medium">
                {query.data?.catalog.models === 0
                  ? t("newapi.catalogEmpty")
                  : t("newapi.capabilityEmpty")}
              </p>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
