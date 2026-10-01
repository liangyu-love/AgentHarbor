import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  Activity,
  AlertCircle,
  ChevronDown,
  Clock,
  Gauge,
  Layers,
  RefreshCw,
  Route,
  Zap,
} from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { CompressionPolicy } from "@/types/newapi";

function Parameters({
  values,
  empty,
}: {
  values: Record<string, string | number>;
  empty: string;
}) {
  const entries = Object.entries(values);
  if (!entries.length) {
    return <span className="text-xs text-muted-foreground italic">{empty}</span>;
  }
  return (
    <dl className="grid gap-1.5 font-mono text-xs">
      {entries.map(([key, value]) => (
        <div
          key={key}
          className="flex flex-wrap items-center justify-between gap-x-3 rounded-md bg-muted/40 px-2.5 py-1"
        >
          <dt className="text-muted-foreground">{key}</dt>
          <dd className="font-semibold text-foreground">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function RoutingDiagnostics({
  failover,
  disabled,
  onFailover,
  compression,
  onCompression,
}: {
  failover: boolean;
  disabled: boolean;
  onFailover: (enabled: boolean) => void;
  compression: CompressionPolicy;
  onCompression: (policy: CompressionPolicy) => void;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [compressionValue, setCompressionValue] = useState("");
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

  return (
    <section
      className="rounded-2xl border border-border/70 bg-card p-4.5 backdrop-blur-md shadow-xs transition-all"
      aria-label={t("newapi.routeDiagnostics")}
    >
      {/* Header bar */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Button
          variant="ghost"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex items-center gap-2 px-2 text-sm font-bold tracking-tight text-foreground hover:bg-muted/80 rounded-xl"
        >
          <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Route className="h-4 w-4" />
          </div>
          <span>{t("newapi.routeDiagnostics")}</span>
          <ChevronDown
            className={cn(
              "h-4 w-4 text-muted-foreground transition-transform duration-200",
              open && "rotate-180 text-foreground",
            )}
          />
        </Button>

        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-2 rounded-xl border border-border/60 bg-muted/30 px-3 py-1.5 text-xs font-medium">
            <Gauge className="h-3.5 w-3.5 text-primary" aria-hidden />
            <span className="text-foreground">{t("newapi.compression.globalDefault")}</span>
            <Select
              value={mode}
              disabled={disabled}
              onValueChange={value => commitCompression(value, compressionValue || currentValue)}
            >
              <SelectTrigger className="h-7 w-32 text-xs" aria-label={t("newapi.compression.title")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="percent">{t("newapi.compression.percent")}</SelectItem>
                <SelectItem value="tokens">{t("newapi.compression.tokens")}</SelectItem>
                <SelectItem value="clientDefault">{t("newapi.compression.clientDefault")}</SelectItem>
              </SelectContent>
            </Select>
            {mode !== "clientDefault" && (
              <Input
                type="number"
                min={mode === "percent" ? 1 : 1}
                max={mode === "percent" ? 99 : 10_000_000}
                inputMode="numeric"
                disabled={disabled}
                className="h-7 w-24 text-xs tabular-nums"
                aria-label={t("newapi.compression.title")}
                value={compressionValue || currentValue}
                onChange={event => setCompressionValue(event.target.value)}
                onBlur={event => {
                  commitCompression(mode, event.target.value);
                  setCompressionValue("");
                }}
              />
            )}
          </label>

          <label className="flex items-center gap-2.5 rounded-xl border border-border/60 bg-muted/30 px-3 py-1.5 text-xs font-medium cursor-pointer hover:bg-muted/50 transition-colors">
            <Switch
              checked={failover}
              disabled={disabled}
              onCheckedChange={onFailover}
              aria-label={t("newapi.sameModelFailover")}
            />
            <span className="text-foreground">{t("newapi.sameModelFailover")}</span>
          </label>
        </div>
      </div>

      <p className="mt-2 text-xs text-muted-foreground/80 leading-relaxed px-2">
        {t("newapi.failoverRules")}
      </p>

      <p className="mt-1 text-xs text-muted-foreground/80 leading-relaxed px-2">
        {t("newapi.compression.description")}
      </p>

      {/* Expanded Content */}
      {open && (
        <div className="mt-4 space-y-3.5 border-t border-border/50 pt-3">
          {/* Top Sub-Bar */}
          <div className="flex flex-wrap items-center justify-between gap-3 px-1 text-xs">
            <p className="text-muted-foreground/90 leading-relaxed max-w-xl">
              {t("newapi.tracePrivacy")}
            </p>
            <Button
              size="sm"
              variant="outline"
              disabled={query.isFetching}
              onClick={() => void query.refetch()}
              className="h-7.5 gap-1.5 rounded-xl px-2.5 text-xs font-semibold shadow-xs"
            >
              <RefreshCw
                className={cn(
                  "h-3.5 w-3.5",
                  query.isFetching && "animate-spin text-primary",
                )}
              />
              <span>{t("common.refresh")}</span>
            </Button>
          </div>

          {query.error && (
            <div
              role="alert"
              className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs font-medium text-destructive"
            >
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{formatInvokeError(query.error)}</span>
            </div>
          )}

          {!query.isLoading && query.data?.length === 0 && (
            <div className="rounded-xl border border-dashed border-border/70 p-8 text-center text-muted-foreground">
              <Activity className="mx-auto h-8 w-8 opacity-40 mb-2" />
              <p className="text-xs font-medium">{t("newapi.traceEmpty")}</p>
            </div>
          )}

          {/* Trace Items List */}
          <div className="max-h-[32rem] space-y-2.5 overflow-y-auto pr-0.5">
            {query.data?.map((route) => {
              const state =
                route.error || (route.status != null && route.status >= 400)
                  ? "failed"
                  : route.done
                    ? "complete"
                    : "running";

              return (
                <details
                  key={route.id}
                  className="group rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-primary/40"
                >
                  <summary className="flex cursor-pointer flex-wrap items-center justify-between gap-2.5 text-xs font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-lg py-0.5">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge
                        variant="outline"
                        className={cn(
                          "px-2 py-0 text-[10px] font-semibold",
                          state === "complete"
                            ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                            : state === "failed"
                              ? "border-rose-500/30 bg-rose-500/10 text-rose-600 dark:text-rose-400"
                              : "border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400",
                        )}
                      >
                        {state === "running" ? (
                          <span className="flex items-center gap-1">
                            <span className="h-1.5 w-1.5 rounded-full bg-amber-500 animate-pulse" />
                            {t("newapi.traceRunning")}
                          </span>
                        ) : state === "failed" ? (
                          t("newapi.traceFailed")
                        ) : (
                          t("newapi.traceComplete")
                        )}
                      </Badge>

                      <span className="font-mono font-bold text-foreground">
                        {route.requestedModel || "—"}
                      </span>

                      <span className="rounded bg-muted px-1.5 py-0 font-mono text-[10px] text-muted-foreground">
                        HTTP {route.status ?? "—"}
                      </span>

                      <span className="flex items-center gap-1 font-mono text-muted-foreground">
                        <Clock className="h-3 w-3 opacity-70" />
                        {route.durationMs} ms
                      </span>
                    </div>

                    <div className="flex items-center gap-2 text-muted-foreground">
                      <span className="font-mono text-[10px]">
                        {new Date(route.timestamp * 1000).toLocaleTimeString()}
                      </span>
                      <ChevronDown className="h-3.5 w-3.5 transition-transform duration-200 group-open:rotate-180" />
                    </div>
                  </summary>

                  {/* Expanded Detail Panel */}
                  <div className="mt-3.5 space-y-3.5 border-t border-border/40 pt-3 text-xs">
                    {/* Timestamp & TTFT metrics */}
                    <div className="flex flex-wrap items-center justify-between gap-2 text-muted-foreground font-mono text-[11px] rounded-lg bg-muted/20 px-2.5 py-1.5">
                      <span>{new Date(route.timestamp * 1000).toLocaleString()}</span>
                      <span className="font-semibold text-foreground">
                        {t("accounts.firstTokenShort")}{" "}
                        {route.firstTokenMs == null ? "—" : `${route.firstTokenMs} ms`}
                      </span>
                    </div>

                    {/* Client Parameters */}
                    <div className="rounded-xl border border-border/50 bg-muted/10 p-3">
                      <h4 className="mb-2 flex items-center gap-1.5 font-bold text-foreground">
                        <Zap className="h-3.5 w-3.5 text-primary" />
                        <span>{t("newapi.traceRequested")}</span>
                      </h4>
                      <Parameters
                        values={route.requested}
                        empty={t("newapi.traceNoParameters")}
                      />
                    </div>

                    {/* Attempts Flow Chain */}
                    <div className="space-y-2">
                      <h4 className="font-bold text-foreground flex items-center gap-1.5 px-0.5">
                        <Layers className="h-3.5 w-3.5 text-primary" />
                        <span>{t("newapi.traceAttempts", { count: route.attempts.length })}</span>
                      </h4>
                      {route.attempts.map((attempt, index) => (
                        <div
                          key={index}
                          className="space-y-2 rounded-xl border border-border/60 bg-muted/30 p-3 shadow-2xs"
                        >
                          <div className="flex flex-wrap items-center justify-between gap-2 font-mono text-xs">
                            <span className="font-semibold text-foreground">
                              #{index + 1} · {route.requestedModel} → {attempt.matchedModel} →{" "}
                              {attempt.providerName} · {attempt.protocol}
                            </span>
                            <span className="rounded bg-muted px-1.5 py-0 text-[10px]">
                              HTTP {attempt.status ?? "—"}
                            </span>
                          </div>

                          <p className="break-words font-mono text-[11px] text-muted-foreground">
                            {t("newapi.upstreamModel")}:{" "}
                            <span className="font-semibold text-foreground">
                              {attempt.upstreamModel}
                            </span>
                          </p>

                          <div className="pt-1">
                            <h4 className="mb-1.5 font-semibold text-foreground text-[11px]">
                              {t("newapi.traceSent")}
                            </h4>
                            <Parameters
                              values={attempt.sent}
                              empty={t("newapi.traceNoParameters")}
                            />
                          </div>

                          {attempt.error && (
                            <p className="rounded-lg bg-destructive/10 p-2 text-xs font-medium text-destructive">
                              {attempt.failureStage}: {attempt.error}
                            </p>
                          )}
                        </div>
                      ))}
                    </div>

                    {route.error && (
                      <div className="rounded-xl border border-destructive/40 bg-destructive/10 p-3 text-xs font-semibold text-destructive">
                        {route.failureStage}: {route.error}
                      </div>
                    )}
                  </div>
                </details>
              );
            })}
          </div>
        </div>
      )}
    </section>
  );
}
