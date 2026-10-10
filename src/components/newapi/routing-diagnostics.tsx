import { useState, type CSSProperties, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Activity, AlertCircle, ChevronDown, Gauge, Globe, RefreshCw, Route, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { toast } from "@/hooks/use-toast";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { CLIENT_SOURCES, clientSourceLabelKey, matchesClientSource, type ClientSourceFilter } from "@/lib/client-source";
import type { CompressionPolicy, NewApiProvider, RouteAttempt, RouteDiagnostic } from "@/types/newapi";

type RouteState = "complete" | "failed" | "running";

const routeState = (route: RouteDiagnostic): RouteState =>
  route.error || (route.status != null && route.status >= 400) ? "failed" : route.done ? "complete" : "running";

const statusTone = (status: number | null) =>
  status == null ? "diag-chip" : status >= 400 ? "diag-chip diag-chip-bad" : "diag-chip diag-chip-good";

// Summary figures read better in seconds once they pass one.
const duration = (ms: number | null) => ms == null ? "—" : ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${ms} ms`;

const median = (values: number[]) => {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : Math.round((sorted[middle - 1] + sorted[middle]) / 2);
};

// Request parameters as key/value chips; the keys are what the client sent.
function Parameters({ values, empty }: { values: Record<string, string | number>; empty: string }) {
  const entries = Object.entries(values);
  if (!entries.length) return <span className="text-xs italic text-muted-foreground">{empty}</span>;
  return (
    <dl className="flex flex-wrap gap-1.5 font-mono text-[11px]">
      {entries.map(([key, value]) => (
        <div key={key} className="diag-param">
          <dt className="text-muted-foreground">{key}</dt>
          <dd className="font-semibold text-foreground">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

// One "label: value" line; the label carries its colon so it reads as one phrase.
function Fact({ label, children, mono }: { label: string; children: ReactNode; mono?: boolean }) {
  return (
    <p className="min-w-0 break-words">
      <span className="text-muted-foreground">{label}: </span>
      <span className={cn("text-foreground", mono && "font-mono")}>{children}</span>
    </p>
  );
}

function Attempt({ route, attempt, index }: { route: RouteDiagnostic; attempt: RouteAttempt; index: number }) {
  const { t } = useTranslation();
  const failed = !!attempt.error || (attempt.status != null && attempt.status >= 400);
  const credential = attempt.credential;
  return (
    <li className="diag-attempt" data-attempt-state={failed ? "failed" : "ok"}>
      <div className="flex flex-wrap items-center gap-2">
        <span className="diag-attempt-index">#{index + 1}</span>
        <span className="min-w-0 break-all font-mono text-[12px] font-semibold text-foreground">
          {route.requestedModel} → {attempt.matchedModel} → {attempt.providerName}
        </span>
        <span className="diag-chip">{attempt.protocol}</span>
        <span className={statusTone(attempt.status)}>HTTP {attempt.status ?? "—"}</span>
        <span className="font-mono text-[11px] text-muted-foreground">{attempt.durationMs} ms</span>
      </div>

      <div className="diag-facts">
        <Fact label={t("newapi.upstreamModel")} mono>{attempt.upstreamModel}</Fact>
        <Fact label={t("newapi.traceServedModel")} mono>
          {attempt.modelObservation?.servedModel ?? "—"}
          {" · "}{t("newapi.traceModelStatus." + (attempt.modelObservation?.modelMatchStatus ?? "unknown"))}
        </Fact>
        <p className="min-w-0">{t("newapi.traceSelectionReason." + attempt.selectionReason, { defaultValue: attempt.selectionReason })}</p>
        {attempt.retryReason && (
          <p className="min-w-0 text-amber-700 dark:text-amber-300">
            {t("newapi.traceRetryReason." + attempt.retryReason, { defaultValue: attempt.retryReason })}
          </p>
        )}
        {attempt.modelObservation?.unknownReason && (
          <p className="min-w-0">{t("newapi.traceUnknownReason." + attempt.modelObservation.unknownReason,
            { defaultValue: attempt.modelObservation.unknownReason })}</p>
        )}
        {attempt.modelObservation?.conflictingModel && (
          <Fact label={t("newapi.traceConflictingModel")} mono>{attempt.modelObservation.conflictingModel}</Fact>
        )}
        {attempt.aggregateId && (
          <p className="min-w-0 break-all font-mono">
            {t("newapi.traceAggregate")}: {attempt.aggregateId.slice(0, 8)}
            {attempt.memberId && ` · ${t("newapi.traceMember")}: ${attempt.memberId.slice(0, 8)}`}
          </p>
        )}
        {attempt.configuredProtocol && (
          <p className="min-w-0">
            {t("newapi.traceProtocolPlan")}: {attempt.configuredProtocol} → {attempt.effectiveProtocol ?? attempt.protocol}
            {attempt.compatibilityPlan && ` · ${t("newapi.traceCompatibility." + attempt.compatibilityPlan, { defaultValue: attempt.compatibilityPlan })}`}
          </p>
        )}
        <Fact label={t("newapi.traceAttemptId")} mono>{attempt.attemptId?.slice(0, 8) ?? "—"}</Fact>
        <p className="min-w-0">
          {t("newapi.traceCredential")}: {t("newapi.traceCredentialStatus." + (credential?.status ?? "not_recorded"))}
          {credential?.versionFingerprint && ` · ${t("newapi.traceCredentialVersion")} ${credential.versionFingerprint.slice(0, 12)}`}
          {credential?.accountId && ` · ${t("newapi.traceAccount")} ${credential.accountId.slice(0, 12)}`}
          {credential?.kind === "codexLocal" && !credential.accountId && ` · ${t("newapi.traceAccountNotRecorded")}`}
          {credential?.reason && ` · ${t("newapi.traceCredentialReason." + credential.reason, { defaultValue: credential.reason })}`}
        </p>
      </div>

      {/* The catalog revision, match kind and pricing this attempt actually
          used; later refreshes never rewrite them. */}
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="diag-chip">{t("newapi.matchKind." + attempt.matchKind, { defaultValue: attempt.matchKind })}</span>
        <span className="diag-chip font-mono">
          {t("newapi.traceCatalogRevision", { revision: (attempt.catalogRevision ?? "").slice(0, 8) || "—" })}
        </span>
        <span className={cn("diag-chip", attempt.pricingStatus === "priced" ? "diag-chip-good" : "diag-chip-warn")}>
          {attempt.pricingStatus === "priced"
            ? t("newapi.tracePriced")
            : attempt.pricingStatus === "partial" ? t("newapi.costPartial") : t("newapi.traceUnpriced")}
          {attempt.pricingSource ? ` · ${t("newapi.capabilitySource." + attempt.pricingSource, { defaultValue: attempt.pricingSource })}` : ""}
        </span>
      </div>

      <div className="space-y-1.5">
        <h5 className="text-[11px] font-semibold text-muted-foreground">{t("newapi.traceSent")}</h5>
        <Parameters values={attempt.sent} empty={t("newapi.traceNoParameters")} />
      </div>

      {attempt.error && (
        <p className="rounded-lg bg-destructive/10 px-2.5 py-1.5 text-xs font-medium text-destructive">
          {attempt.failureStage}: {attempt.error}
        </p>
      )}
    </li>
  );
}

export function RoutingDiagnostics({
  disabled,
  compression,
  onCompression,
  webSearchProviderId,
  searchProviders,
  searchDisabled,
  onWebSearchProvider,
}: {
  disabled: boolean;
  compression: CompressionPolicy;
  onCompression: (policy: CompressionPolicy) => void;
  webSearchProviderId: string | null;
  searchProviders: NewApiProvider[];
  searchDisabled: boolean;
  onWebSearchProvider: (providerId: string | null) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [source, setSource] = useState<ClientSourceFilter>("all");
  const [failedOnly, setFailedOnly] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [clearError, setClearError] = useState<string | null>(null);
  const [compressionValue, setCompressionValue] = useState("");
  const selectedSearchUnavailable = webSearchProviderId !== null &&
    !searchProviders.some(provider => provider.id === webSearchProviderId);
  const query = useQuery({
    queryKey: ["route-diagnostics"],
    queryFn: api.routing.diagnostics,
    enabled: open,
    refetchInterval: open ? 2000 : false,
  });
  const mode =
    compression.mode === "tokens" || compression.mode === "clientDefault"
      ? compression.mode
      : "percent";
  const routes = query.data ?? [];
  const visibleRoutes = routes.filter(route =>
    matchesClientSource(route.clientSource, source) && (!failedOnly || routeState(route) === "failed"));
  const finished = visibleRoutes.filter(route => route.done);
  const failures = visibleRoutes.filter(route => routeState(route) === "failed").length;
  const averageMs = finished.length ? Math.round(finished.reduce((sum, route) => sum + route.durationMs, 0) / finished.length) : null;
  const firstToken = median(visibleRoutes.flatMap(route => route.firstTokenMs == null ? [] : [route.firstTokenMs]));
  const longest = Math.max(1, ...visibleRoutes.map(route => route.durationMs));
  const currentValue =
    compression.mode === "tokens"
      ? String(compression.tokens)
      : compression.mode === "percent"
        ? String(compression.percent)
        : "";
  const commitCompression = (next: string, raw: string) => {
    if (next === "clientDefault") {
      onCompression({ mode: "clientDefault" });
      return;
    }
    const value = Number(raw || (next === "percent" ? "90" : currentValue));
    if (!Number.isSafeInteger(value) || value <= 0) return;
    if (next === "percent") {
      if (value < 1 || value > 99) return;
      onCompression({ mode: "percent", percent: value });
    } else {
      onCompression({ mode: "tokens", tokens: value });
    }
  };

  const clear = async () => {
    setClearing(true);
    setClearError(null);
    try {
      const count = await api.routing.clearDiagnostics();
      await query.refetch();
      toast({ title: t("newapi.traceCleared", { count }), variant: "success" });
    } catch (cause) {
      setClearError(formatInvokeError(cause));
    } finally {
      setClearing(false);
    }
  };

  const stats = [
    { key: "requests", label: t("newapi.traceStatRequests"), value: String(visibleRoutes.length) },
    { key: "failed", label: t("newapi.traceStatFailed"), value: String(failures), bad: failures > 0 },
    { key: "average", label: t("newapi.traceStatAverage"), value: duration(averageMs) },
    { key: "firstToken", label: t("newapi.traceStatFirstToken"), value: duration(firstToken) },
  ];

  return (
    <section className="harbor-card" aria-label={t("newapi.routeDiagnostics")} data-route-diagnostics>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
            className="diag-toggle"
          >
            <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-background text-foreground ring-1 ring-border">
              <Route className="h-[18px] w-[18px]" />
            </span>
            <span className="font-display text-[17px] font-semibold tracking-tight">{t("newapi.routeDiagnostics")}</span>
            <ChevronDown aria-hidden className={cn("h-4 w-4 text-muted-foreground transition-transform duration-200", open && "rotate-180")} />
          </button>
          {open && query.isSuccess && (
            <span className="hidden items-center gap-1.5 text-[11px] font-medium text-primary sm:inline-flex" data-diagnostics-live>
              <span aria-hidden className="agg-dot-live h-1.5 w-1.5 rounded-full bg-primary" />
              {t("newapi.traceLive", { count: routes.length })}
            </span>
          )}
        </div>

        {/* Routing-wide settings that apply to every request shown below. */}
        <div className="flex flex-wrap items-center gap-2">
          {(searchProviders.length > 1 || selectedSearchUnavailable) && (
            <div className="diag-setting">
              <Globe className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
              <span id="web-search-provider-label">{t("newapi.webSearchService")}</span>
              <Select value={webSearchProviderId ?? "__automatic"} disabled={searchDisabled}
                onValueChange={value => onWebSearchProvider(value === "__automatic" ? null : value)}>
                <SelectTrigger className="h-7 w-36 bg-card text-xs" aria-labelledby="web-search-provider-label">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="__automatic">{t("newapi.webSearchAutomatic")}</SelectItem>
                  {selectedSearchUnavailable && webSearchProviderId && (
                    <SelectItem value={webSearchProviderId} disabled>{t("newapi.webSearchUnavailable")}</SelectItem>
                  )}
                  {searchProviders.map(provider => <SelectItem key={provider.id} value={provider.id}>{provider.name}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
          )}
          <label className="diag-setting">
            <Gauge className="h-3.5 w-3.5 text-muted-foreground" aria-hidden />
            <span>{t("newapi.compression.globalDefault")}</span>
            <Select value={mode} disabled={disabled} onValueChange={value => commitCompression(value, compressionValue || currentValue)}>
              <SelectTrigger className="h-7 w-32 bg-card text-xs" aria-label={t("newapi.compression.title")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="percent">{t("newapi.compression.percent")}</SelectItem>
                <SelectItem value="tokens">{t("newapi.compression.tokens")}</SelectItem>
                <SelectItem value="clientDefault">{t("newapi.compression.clientDefault")}</SelectItem>
              </SelectContent>
            </Select>
            {mode !== "clientDefault" && (
              <>
                <Input
                  type="number"
                  min={1}
                  max={mode === "percent" ? 99 : 10_000_000}
                  inputMode="numeric"
                  disabled={disabled}
                  className="h-7 w-20 bg-card text-xs tabular-nums"
                  aria-label={mode === "percent" ? t("newapi.compression.percent") : t("newapi.compression.tokens")}
                  value={compressionValue || currentValue}
                  onChange={event => setCompressionValue(event.target.value)}
                  onBlur={event => {
                    commitCompression(mode, event.target.value);
                    setCompressionValue("");
                  }}
                />
                <span className="text-[10px] text-muted-foreground">{t(mode === "percent" ? "newapi.percentUnit" : "newapi.tokenUnit")}</span>
              </>
            )}
          </label>
        </div>
      </div>

      {open && (
        <div className="mt-5 space-y-4 border-t border-border/60 pt-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <dl className="diag-stats">
              {stats.map(stat => (
                <div key={stat.key} className="diag-stat" data-stat={stat.key}>
                  <dt>{stat.label}</dt>
                  <dd className={cn(stat.bad && "text-destructive")}>{stat.value}</dd>
                </div>
              ))}
            </dl>
            <div className="flex flex-wrap items-center gap-2">
              <Select value={source} onValueChange={value => setSource(value as ClientSourceFilter)}>
                <SelectTrigger className="h-8 w-36 text-xs" aria-label={t("newapi.routeSourceLabel")}><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="all">{t("newapi.routeSourceAll")}</SelectItem>
                  {CLIENT_SOURCES.map(option => <SelectItem key={option.value} value={option.value}>{t(option.labelKey)}</SelectItem>)}
                </SelectContent>
              </Select>
              <button type="button" aria-pressed={failedOnly} onClick={() => setFailedOnly(!failedOnly)} className="diag-filter">
                <AlertCircle className="h-3.5 w-3.5" aria-hidden />
                {t("newapi.traceFailedOnly")}
              </button>
              <button type="button" className="diag-filter" disabled={query.isFetching} onClick={() => void query.refetch()}>
                <RefreshCw className={cn("h-3.5 w-3.5", query.isFetching && "animate-spin text-primary")} aria-hidden />
                {t("common.refresh")}
              </button>
              <button type="button" className="diag-filter hover:!text-destructive" data-trace-clear
                disabled={clearing || !routes.some(route => route.done)} onClick={() => void clear()}>
                <Trash2 className="h-3.5 w-3.5" aria-hidden />
                {t("newapi.traceClear")}
              </button>
            </div>
          </div>
          <p className="text-[11.5px] leading-relaxed text-muted-foreground">{t("newapi.tracePrivacy")} {t("newapi.traceRetention")}</p>

          {(query.error || clearError) && (
            <div role="alert" className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs font-medium text-destructive">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{clearError ?? formatInvokeError(query.error)}</span>
            </div>
          )}

          {!query.isLoading && visibleRoutes.length === 0 && (
            <div className="flex flex-col items-center gap-2 rounded-2xl border border-dashed border-border/70 p-8 text-center text-muted-foreground">
              <Activity className="h-7 w-7 opacity-40" />
              <p className="text-xs font-medium">{t(failedOnly && routes.length ? "newapi.traceNoFailures" : "newapi.traceEmpty")}</p>
            </div>
          )}

          {visibleRoutes.length > 0 && (
            <div className="diag-list">
              {visibleRoutes.map(route => {
                const state = routeState(route);
                return (
                  <details key={route.id} className="diag-row group bg-card" data-route-state={state}>
                    <summary className="diag-summary">
                      <span className="diag-state" data-state={state}>
                        <span aria-hidden className="diag-state-dot" />
                        {t(state === "running" ? "newapi.traceRunning" : state === "failed" ? "newapi.traceFailed" : "newapi.traceComplete")}
                      </span>
                      <span className="min-w-0 flex-1 basis-48 truncate font-mono text-[12.5px] font-semibold text-foreground">
                        {route.requestedModel || "—"}
                        {route.resolvedModel && route.resolvedModel !== route.requestedModel && ` → ${route.resolvedModel}`}
                      </span>
                      <span className="diag-chip">{t(clientSourceLabelKey(route.clientSource))}</span>
                      {route.attempts.length > 1 && (
                        <span className="diag-chip diag-chip-warn">{t("newapi.traceAttemptCount", { count: route.attempts.length })}</span>
                      )}
                      <span className={statusTone(route.status)}>HTTP {route.status ?? "—"}</span>
                      <span className="diag-duration" title={`${route.durationMs} ms`}>
                        <span className="diag-duration-track">
                          <span style={{ "--share": `${Math.max(4, (route.durationMs / longest) * 100)}%` } as CSSProperties} />
                        </span>
                        {route.durationMs} ms
                      </span>
                      <span className="font-mono text-[10.5px] text-muted-foreground">{new Date(route.timestamp * 1000).toLocaleTimeString()}</span>
                      <ChevronDown aria-hidden className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200 group-open:rotate-180" />
                    </summary>

                    <div className="diag-body">
                      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 font-mono text-[11px] text-muted-foreground">
                        <span>{new Date(route.timestamp * 1000).toLocaleString()}</span>
                        <span>
                          {t("accounts.firstTokenShort")}{" "}
                          <span className="font-semibold text-foreground">{route.firstTokenMs == null ? "—" : `${route.firstTokenMs} ms`}</span>
                        </span>
                      </div>

                      <div className="space-y-1.5">
                        <h4 className="text-[11px] font-semibold text-muted-foreground">{t("newapi.traceRequested")}</h4>
                        <Parameters values={route.requested} empty={t("newapi.traceNoParameters")} />
                      </div>

                      <div className="space-y-2">
                        <h4 className="text-[11px] font-semibold text-muted-foreground">
                          {t("newapi.traceAttempts", { count: route.attempts.length })}
                        </h4>
                        <ol className="diag-timeline">
                          {route.attempts.map((attempt, index) => (
                            <Attempt key={attempt.attemptId ?? index} route={route} attempt={attempt} index={index} />
                          ))}
                        </ol>
                      </div>

                      {route.error && (
                        <p className="rounded-xl border border-destructive/40 bg-destructive/10 p-3 text-xs font-semibold text-destructive">
                          {route.failureStage}: {route.error}
                        </p>
                      )}
                    </div>
                  </details>
                );
              })}
            </div>
          )}
        </div>
      )}
    </section>
  );
}
