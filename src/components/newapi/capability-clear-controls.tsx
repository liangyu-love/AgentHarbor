import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Loader2, RotateCcw } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { aggregationQueryKey } from "@/types/aggregation";
import type { ClearCapabilityField, ClearCapabilityPreview, CostRates, ModelCapability, ResolvedField } from "@/types/newapi";

const modelFields: ClearCapabilityField[] = [
  "contextWindow", "inputLimit", "outputLimit", "reasoning", "defaultEffort",
  "price", "catalogRef", "priceRef", "compression",
];
const providerFields: ClearCapabilityField[] = [
  "providerContextWindow", "providerCatalogProviders", "providerPriceCatalogProvider", "providerCompression",
];
type Cell = { lines: string[]; source?: string; reference?: string };
type Row = { id: string; label: string; before: Cell; after: Cell; changed: boolean };
type Selection = { field: ClearCapabilityField; modelSignature: string; result: ClearCapabilityPreview };
type Notice = { tone: "success" | "warning" | "error"; text: string };

function ComparisonCell({ cell }: { cell: Cell }) {
  return <div className="min-w-0 space-y-1">
    {cell.lines.map((line, index) => <div key={index} className="break-words font-mono tabular-nums [overflow-wrap:anywhere]">{line}</div>)}
    {cell.source && <div className="text-muted-foreground">{cell.source}</div>}
    {cell.reference && <div className="break-all font-mono text-[11px] text-muted-foreground">{cell.reference}</div>}
  </div>;
}

export function CapabilityClearControls({ model, onSaved }: { model: ModelCapability; onSaved: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const fieldId = useId();
  const [field, setField] = useState<ClearCapabilityField>("contextWindow");
  const [selection, setSelection] = useState<Selection | null>(null);
  const [busy, setBusy] = useState<"preview" | "apply" | null>(null);
  const [notice, setNotice] = useState<Notice | null>(null);
  const [showAll, setShowAll] = useState(false);
  const operation = useRef(0);
  const inFlight = useRef(false);
  const pending = useRef<"preview" | "apply" | null>(null);
  const modelSignature = useMemo(() => JSON.stringify(model), [model]);
  const currentSignature = useRef(modelSignature);
  currentSignature.current = modelSignature;
  const active = selection?.field === field && selection.modelSignature === modelSignature ? selection.result : null;
  const providerScope = field.startsWith("provider");

  useEffect(() => {
    if (pending.current !== "apply") {
      operation.current += 1;
      inFlight.current = false;
      pending.current = null;
      setBusy(null);
    }
    setSelection(null);
    setShowAll(false);
  }, [modelSignature]);
  useEffect(() => () => { operation.current += 1; }, []);

  const changeField = (next: ClearCapabilityField) => {
    operation.current += 1;
    inFlight.current = false;
    pending.current = null;
    setField(next);
    setSelection(null);
    setNotice(null);
    setBusy(null);
    setShowAll(false);
  };

  const preview = async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    pending.current = "preview";
    const request = ++operation.current;
    const signature = modelSignature;
    setBusy("preview");
    setSelection(null);
    setNotice(null);
    setShowAll(false);
    try {
      const result = await api.routing.previewCapabilityClear(model.providerId, model.upstreamModel, field);
      if (request === operation.current && signature === currentSignature.current) {
        setSelection({ field, modelSignature: signature, result });
      }
    } catch (error) {
      if (request === operation.current && signature === currentSignature.current) setNotice({ tone: "error", text: formatInvokeError(error) });
    } finally {
      if (request === operation.current) { inFlight.current = false; pending.current = null; setBusy(null); }
    }
  };

  const apply = async () => {
    if (!active?.hasOverride || inFlight.current) return;
    inFlight.current = true;
    pending.current = "apply";
    const request = ++operation.current;
    setBusy("apply");
    setNotice(null);
    try {
      // Consume the operation and model identity bound to the reviewed preview.
      const result = await api.routing.applyCapabilityClear(active);
      if (result.refreshedPreview) {
        if (request === operation.current && modelSignature === currentSignature.current) {
          setSelection({ field: result.refreshedPreview.field, modelSignature, result: result.refreshedPreview });
          setNotice({ tone: "warning", text: t("newapi.capabilityClearStale") });
        }
      } else if (result.applied) {
        if (request === operation.current) {
          setSelection(null);
          setNotice({ tone: "success", text: t("newapi.capabilityClearApplied") });
        }
        queryClient.setQueryData(queryKeys.providers, result.applied.state);
        if (result.applied.applied) queryClient.setQueryData(queryKeys.providerApplied, result.applied.applied);
        void queryClient.invalidateQueries({ queryKey: aggregationQueryKey });
        void queryClient.invalidateQueries({ queryKey: queryKeys.clientConfigs });
        void queryClient.invalidateQueries({ queryKey: queryKeys.codexSubagents });
        onSaved();
      }
    } catch (error) {
      if (request === operation.current) setNotice({ tone: "error", text: formatInvokeError(error) });
    } finally {
      if (request === operation.current) { inFlight.current = false; pending.current = null; setBusy(null); }
    }
  };

  const source = (value: string) => t("newapi.capabilitySource." + value, { defaultValue: value === "none" ? "—" : value });
  const number = (value: number | null | undefined) => value == null ? "—" : value.toLocaleString();
  const reference = (value: { providerId: string; modelId: string } | null) => value ? `${value.providerId}/${value.modelId}` : undefined;
  const numeric = (value: ResolvedField<number>): Cell => ({ lines: [number(value.value)], source: source(value.source), reference: reference(value.sourceRef) });
  const rows: Row[] = [];
  if (active) {
    const add = (id: string, label: string, select: (value: ModelCapability) => unknown, render: (value: ModelCapability) => Cell) => {
      rows.push({ id, label, before: render(active.before), after: render(active.after),
        changed: JSON.stringify(select(active.before)) !== JSON.stringify(select(active.after)) });
    };
    for (const [id, select] of [
      ["contextWindow", (value: ModelCapability) => value.context],
      ["inputLimit", (value: ModelCapability) => value.inputLimit],
      ["outputLimit", (value: ModelCapability) => value.output],
    ] as const) add(id, t("newapi.capabilityField." + id), select, value => numeric(select(value)));
    for (const [id, key, sourceKey] of [
      ["clientContext", "contextWindow", "contextSource"],
      ["clientBudget", "budget", "budgetSource"],
      ["clientOutput", "outputLimit", "outputSource"],
    ] as const) {
      add(id, t("newapi.capabilityClearClientValue." + key), value => [value.projection[key], value.projection[sourceKey]],
        value => ({ lines: [number(value.projection[key])], source: source(value.projection[sourceKey]) }));
    }
    add("reasoning", t("newapi.capabilityField.reasoning"), value => value.reasoning, value => ({
      lines: [value.reasoning.value?.map(level => level.effort).join(" / ") || "—"],
      source: source(value.reasoning.source), reference: reference(value.reasoning.sourceRef),
    }));
    add("defaultEffort", t("newapi.capabilityDefaultEffort"), value => [value.defaultReasoning, value.defaultReasoningSource], value => ({
      lines: [value.defaultReasoning ?? "—"], source: source(value.defaultReasoningSource),
    }));
    const compression = (value: ModelCapability): Cell => {
      const plan = value.projection.compression;
      const mode = plan.mode === "percent" ? `${plan.percent}%` : plan.mode === "tokens" ? number(plan.tokens) : t("newapi.compression.clientDefault");
      return { lines: [number(plan.threshold)], source: `${mode} · ${t("newapi.capabilityClearCompressionSource." + plan.source, { defaultValue: source(plan.source) })}` };
    };
    add("compression", t("newapi.capabilityClearPlannedCompression"), value => value.projection.compression, compression);
    if (active.before.compressionScope === "nativeShared" || active.after.compressionScope === "nativeShared") {
      add("sharedCompression", t("newapi.capabilityClearSharedCompression"), value => [value.compressionSharedThreshold, value.compressionSharedPlan], value => ({
        lines: [number(value.compressionSharedThreshold)],
        source: t("newapi.capabilityClearSharedState." + (value.compressionSharedPlan?.state ?? "unknown")),
        reference: value.compressionLimitingModels.length ? t("newapi.capabilityClearLimitedBy", { models: value.compressionLimitingModels.join(", ") }) : undefined,
      }));
    }
    add("appliedCompression", t("newapi.capabilityClearAppliedCompression"), value => value.compressionAppliedThreshold,
      value => ({ lines: [number(value.compressionAppliedThreshold)] }));
    add("match", t("newapi.capabilityMatch"), value => [value.matched, value.matchKind, value.matchedModel], value => ({
      lines: [reference(value.matched) ?? "—"], source: t("newapi.matchKind." + value.matchKind, { defaultValue: value.matchKind }),
    }));
    add("price", t("newapi.capabilityPrice"), value => value.price, value => {
      const rateLines = (rates: CostRates) => (["input", "output", "cacheRead", "cacheWrite", "reasoning", "inputAudio", "outputAudio"] as const)
        .filter(key => rates[key] != null).map(key => `${t("newapi.priceRates." + key)} $${rates[key]}`);
      const lines = rateLines(value.price);
      if (value.price.contextOver200k) lines.push(t("newapi.capabilityPriceOver200k"), ...rateLines(value.price.contextOver200k));
      for (const tier of value.price.tiers) lines.push(t("newapi.capabilityPriceTier", { value: number(tier.tier.size) }), ...rateLines(tier));
      return { lines: lines.length ? lines : ["—"], source: `${source(value.price.source)} · ${t("newapi.capabilityPricePerMillion")}`, reference: reference(value.price.sourceRef) };
    });
    add("conflicts", t("newapi.capabilityConflicts"), value => value.conflicts, value => ({
      lines: value.conflicts.length ? value.conflicts.map(conflict => `${t("newapi.capabilityField." + conflict.field, { defaultValue: conflict.field })}: ${conflict.candidates.map(candidate => candidate.value).join(" / ")}`) : ["—"],
    }));
  }
  const shownRows = showAll ? rows : rows.filter(row => row.changed);

  return <section data-capability-clear className="space-y-3 border-t border-border/60 pt-3" aria-label={t("newapi.capabilityClearTitle")} aria-busy={busy !== null}>
    <div className="flex flex-wrap items-center justify-between gap-2">
      <h4 className="flex items-center gap-2 text-sm font-semibold"><RotateCcw aria-hidden className="h-4 w-4 text-muted-foreground" />{t("newapi.capabilityClearTitle")}</h4>
      <Badge variant="outline" className="text-xs font-normal">{t(providerScope ? "newapi.capabilityClearProviderGroup" : "newapi.capabilityClearModelGroup")}</Badge>
    </div>
    <div className="flex flex-wrap items-end gap-2">
      <div className="min-w-0 flex-1 space-y-1.5">
        <label htmlFor={fieldId} className="text-xs text-muted-foreground">{t("newapi.capabilityClearFieldLabel")}</label>
        <select id={fieldId} value={field} disabled={busy === "apply"} onChange={event => changeField(event.target.value as ClearCapabilityField)}
          className="h-9 w-full rounded-md border border-input bg-background px-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50">
          <optgroup label={t("newapi.capabilityClearModelGroup")}>{modelFields.map(value => <option key={value} value={value}>{t("newapi.capabilityClearField." + value)}</option>)}</optgroup>
          <optgroup label={t("newapi.capabilityClearProviderGroup")}>{providerFields.map(value => <option key={value} value={value}>{t("newapi.capabilityClearField." + value)}</option>)}</optgroup>
        </select>
      </div>
      <Button size="sm" variant="outline" disabled={busy !== null} onClick={() => void preview()} className="h-9 gap-1.5">
        {busy === "preview" && <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />}
        {t(busy === "preview" ? "newapi.capabilityClearPreviewing" : "newapi.capabilityClearPreview")}
      </Button>
    </div>
    {notice && <p role={notice.tone === "error" ? "alert" : "status"} className={cn("text-sm break-words",
      notice.tone === "error" ? "text-destructive" : notice.tone === "warning" ? "text-amber-700 dark:text-amber-300" : "text-foreground")}>
      {notice.text}
    </p>}
    {active && <div data-clear-result className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
        <p className="min-w-0 break-all font-medium">{providerScope ? model.providerName : model.upstreamModel}</p>
        <span className="text-muted-foreground">{t("newapi.capabilityClearAffected", { changed: active.affectedModels.length, total: active.scopeCount })}</span>
      </div>
      {!active.hasOverride && <p role="status" className="text-sm text-muted-foreground">{t("newapi.capabilityClearNoOverride")}</p>}
      {active.hasOverride && shownRows.length === 0 && <p className="text-sm text-muted-foreground">{t("newapi.capabilityClearUnchanged")}</p>}
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 break-words text-xs text-muted-foreground">{t("newapi.capabilityClearCurrentModel", { model: model.displayName || model.upstreamModel })}</span>
        <Button size="sm" variant="ghost" aria-pressed={showAll} onClick={() => setShowAll(!showAll)} className="h-8 shrink-0 px-2 text-xs">
          {t(showAll ? "newapi.capabilityClearChangedOnly" : "newapi.capabilityClearShowAll")}
        </Button>
      </div>
      {shownRows.length > 0 && <div className="text-xs">
        <div className="hidden grid-cols-[minmax(6rem,0.7fr)_minmax(0,1fr)_minmax(0,1fr)] gap-3 pb-2 text-muted-foreground sm:grid">
          <span>{t("newapi.capabilityClearValue")}</span><span>{t("newapi.capabilityClearBefore")}</span><span>{t("newapi.capabilityClearAfter")}</span>
        </div>
        {shownRows.map(row => <div key={row.id} data-clear-row={row.id} className="grid grid-cols-2 gap-3 border-t border-border/50 py-3 sm:grid-cols-[minmax(6rem,0.7fr)_minmax(0,1fr)_minmax(0,1fr)]">
          <div className="col-span-2 font-medium sm:col-span-1">{row.label}</div>
          <div className="min-w-0"><span className="mb-1 block text-muted-foreground sm:hidden">{t("newapi.capabilityClearBefore")}</span><ComparisonCell cell={row.before} /></div>
          <div className={cn("min-w-0", row.changed && "text-primary")}><span className="mb-1 block text-muted-foreground sm:hidden">{t("newapi.capabilityClearAfter")}</span><ComparisonCell cell={row.after} /></div>
        </div>)}
      </div>}
      {(providerScope || active.affectedModels.length > 1) && active.affectedModels.length > 0 && <details className="text-xs">
        <summary className="cursor-pointer rounded py-1 font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{t("newapi.capabilityClearAffectedModels", { count: active.affectedModels.length })}</summary>
        <ul className="mt-1 max-h-32 space-y-2 overflow-y-auto py-1 pr-1">{active.affectedModels.map(affected => <li key={affected.modelId}>
          <span className="block break-words">{affected.displayName || affected.modelId}</span>
          {affected.displayName && affected.displayName !== affected.modelId && <span className="block break-all font-mono text-muted-foreground">{affected.modelId}</span>}
        </li>)}</ul>
      </details>}
      <div data-clear-actions className="sticky bottom-0 z-10 flex flex-wrap items-center justify-between gap-2 border-t border-border/60 bg-card py-3">
        <span className="text-[11px] text-muted-foreground">{t("newapi.capabilityClearRevision", { revision: active.catalogRevision.slice(0, 8) })}</span>
        <div className="flex flex-wrap items-center gap-2">
          <Button size="sm" variant="ghost" disabled={busy !== null} onClick={() => { setSelection(null); setNotice(null); }}>
            {t("newapi.capabilityClearDismiss")}
          </Button>
          <Button size="sm" disabled={busy !== null || !active.hasOverride} onClick={() => void apply()} className="gap-1.5">
            {busy === "apply" && <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />}
            {t(busy === "apply" ? "newapi.capabilityClearApplying" : "newapi.capabilityClearApply")}
          </Button>
        </div>
      </div>
    </div>}
  </section>;
}
