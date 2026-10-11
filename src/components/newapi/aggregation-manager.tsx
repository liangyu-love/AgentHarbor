import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { GitBranch, Network, Plus, RefreshCw, X } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { useFold } from "@/hooks/use-fold";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { aggregationLiveQueryKey, aggregationQueryKey, type AggregateModel } from "@/types/aggregation";
import type { NewApiProvider } from "@/types/newapi";
import { AggregateEditor } from "./aggregate-editor";
import { AggregateRouteRow } from "./aggregate-route-row";
import { AggregateSuggestions } from "./aggregate-suggestions";
import { aggregateFromSuggestion, initialAggregate, mergeSuggestions, normalizeTiers } from "./aggregation-model";

// How often the routing board asks which member each aggregate sticks to.
const LIVE_INTERVAL = 4000;

export function AggregationManager({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const cache = useQueryClient();
  const query = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });
  const models = query.data?.models ?? [];
  const live = useQuery({
    queryKey: aggregationLiveQueryKey,
    queryFn: api.aggregation.live,
    enabled: models.length > 0,
    refetchInterval: LIVE_INTERVAL,
  });
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
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const fold = useFold("aggregation");

  useEffect(() => {
    if (copiedId === null) return;
    const timer = window.setTimeout(() => setCopiedId(null), 2000);
    return () => window.clearTimeout(timer);
  }, [copiedId]);

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
  const memberTargets = new Set((draft?.members ?? []).map((member) => JSON.stringify([member.providerId, member.actualModelId])));
  const memberReferences = referenceSources.filter((source) => memberTargets.has(JSON.stringify([source.providerId, source.modelId])));
  // Matches for the draft's own members come first; every card's stay offered.
  const suggestions = [...new Set([
    ...memberReferences.map((source) => source.reference.modelId),
    ...references.map((reference) => reference.modelId),
  ])];
  const merges = useMemo(() => mergeSuggestions(providers, models), [providers, models]);
  const liveById = new Map((live.data ?? []).map((entry) => [entry.aggregateId, entry]));

  const changed = async () => {
    await Promise.all([
      cache.invalidateQueries({ queryKey: aggregationQueryKey }),
      cache.invalidateQueries({ queryKey: aggregationLiveQueryKey }),
      cache.invalidateQueries({ queryKey: queryKeys.providers }),
      cache.invalidateQueries({ queryKey: queryKeys.clientConfigs }),
      cache.invalidateQueries({ queryKey: queryKeys.claudeDesktopCandidates }),
      cache.invalidateQueries({ queryKey: queryKeys.claudeDesktopStatus }),
    ]);
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

  const update = (patch: Partial<AggregateModel>) => setDraft((current) => (current ? { ...current, ...patch } : current));

  const open = (model: AggregateModel) => {
    if (!fold.open) fold.toggle();
    setDraft(model);
    setError(null);
  };

  const save = () =>
    void run(async () => {
      if (!draft) return;
      if (
        !draft.publicModelId.trim() ||
        draft.members.length === 0 ||
        draft.members.some((member) =>
          !member.providerId || !member.actualModelId || !Number.isInteger(member.weight) || member.weight < 1 || member.weight > 1000)
      ) {
        setError(t("aggregation.formInvalid"));
        return;
      }
      await api.aggregation.save({ ...draft, members: normalizeTiers(draft.members) });
      setDraft(null);
      await changed();
      toast({ title: t("aggregation.saved"), variant: "success" });
    });

  const copyPublicId = (publicId: string) =>
    void run(async () => {
      setCopiedId(null);
      await navigator.clipboard.writeText(publicId);
      setCopiedId(publicId);
      toast({ title: t("aggregation.publicIdCopied"), description: publicId, variant: "success" });
    });

  const editor = draft && (
    <AggregateEditor
      draft={draft}
      update={update}
      providers={providers}
      busy={busy}
      suggestions={suggestions}
      references={references}
      onSave={save}
      onCancel={() => setDraft(null)}
    />
  );
  const liveCount = (live.data ?? []).filter((entry) => entry.preferredMemberId).length;

  return (
    <section className="harbor-card space-y-5" aria-labelledby="aggregation-title">
      <FoldHeader
        open={fold.open}
        onToggle={fold.toggle}
        titleId="aggregation-title"
        contentId="aggregation-body"
        title={t("aggregation.title")}
        titleClassName="font-display text-[17px] tracking-tight"
        leading={
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-background text-foreground ring-1 ring-border">
            <Network className="h-[18px] w-[18px]" />
          </div>
        }
        meta={
          <>
            <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">{models.length}</span>
            {liveCount > 0 && (
              <span className="hidden items-center gap-1.5 text-[11px] font-medium text-primary sm:inline-flex" data-aggregation-live>
                <span aria-hidden className="agg-dot-live h-1.5 w-1.5 rounded-full bg-primary" />
                {t("aggregation.live.routing", { count: liveCount })}
              </span>
            )}
          </>
        }
        actions={
          <Button
            disabled={busy || draft !== null || models.length >= 128}
            className="gap-1.5 shadow-sm transition-all motion-safe:active:scale-95"
            onClick={() => open(initialAggregate())}
          >
            <Plus className="h-4 w-4" />
            {t("aggregation.add")}
          </Button>
        }
      />

      <FoldBody open={fold.open} id="aggregation-body" className="space-y-5">
        {(error || query.error) && (
          <div role="alert" className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3.5 text-sm text-destructive">
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

        {!draft && (
          <AggregateSuggestions
            suggestions={merges}
            disabled={busy || models.length >= 128}
            onCreate={(suggestion) => open(aggregateFromSuggestion(suggestion))}
          />
        )}

        {draft && !draft.id && editor}

        {!query.isPending && models.length === 0 && !draft && (
          <div className="flex flex-col items-center gap-3 rounded-2xl border border-dashed border-border/80 p-8 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-muted text-muted-foreground">
              <GitBranch className="h-6 w-6" />
            </div>
            <p role="status" className="text-sm text-muted-foreground">{t("aggregation.empty")}</p>
          </div>
        )}

        {models.length > 0 && (
          <ul className="agg-board">
            {models.map((model) =>
              draft?.id === model.id ? (
                <li key={model.id} className="agg-board-editing">{editor}</li>
              ) : (
                <AggregateRouteRow
                  key={model.id}
                  model={model}
                  providers={providers}
                  live={liveById.get(model.id)}
                  busy={busy || draft !== null}
                  copied={copiedId === model.publicModelId}
                  onCopy={() => copyPublicId(model.publicModelId)}
                  onEdit={() => open({
                    ...model,
                    retryPolicy: { ...model.retryPolicy, failureThreshold: model.retryPolicy.failureThreshold ?? 3 },
                    firstTokenTimeoutSecs: model.firstTokenTimeoutSecs ?? 0,
                    members: model.members.map((member) => ({ ...member })),
                  })}
                  onDelete={() =>
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
                />
              ),
            )}
          </ul>
        )}

      </FoldBody>

      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </section>
  );
}
