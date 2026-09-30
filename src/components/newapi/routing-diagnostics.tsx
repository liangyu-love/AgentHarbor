import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";

function Parameters({ values, empty }: { values: Record<string, string | number>; empty: string }) {
  const entries = Object.entries(values);
  return entries.length ? <dl className="grid gap-1 font-mono text-xs">{entries.map(([key, value]) =>
    <div key={key} className="flex flex-wrap gap-x-3"><dt className="text-muted-foreground">{key}</dt><dd>{value}</dd></div>)}</dl>
    : <span className="text-xs text-muted-foreground">{empty}</span>;
}

export function RoutingDiagnostics({ failover, disabled, onFailover }: { failover: boolean; disabled: boolean; onFailover: (enabled: boolean) => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const query = useQuery({ queryKey: ["route-diagnostics"], queryFn: api.routing.diagnostics, enabled: open, refetchInterval: open ? 2000 : false });
  return <section className="rounded-xl border bg-card p-4" aria-label={t("newapi.routeDiagnostics")}>
    <div className="flex flex-wrap items-center justify-between gap-3">
      <Button variant="ghost" aria-expanded={open} onClick={() => setOpen(!open)}>{t("newapi.routeDiagnostics")}</Button>
      <label className="flex items-center gap-2 text-sm"><Switch checked={failover} disabled={disabled} onCheckedChange={onFailover} aria-label={t("newapi.sameModelFailover")} />{t("newapi.sameModelFailover")}</label>
    </div>
    <p className="mt-2 text-xs text-muted-foreground">{t("newapi.failoverRules")}</p>
    {open && <div className="mt-4 space-y-3">
      <div className="flex items-center justify-between gap-3"><p className="text-xs text-muted-foreground">{t("newapi.tracePrivacy")}</p><Button size="sm" variant="outline" disabled={query.isFetching} onClick={() => void query.refetch()}>{t("common.refresh")}</Button></div>
      {query.error && <p role="alert" className="text-sm text-destructive">{formatInvokeError(query.error)}</p>}
      {!query.isLoading && query.data?.length === 0 && <p className="text-sm text-muted-foreground">{t("newapi.traceEmpty")}</p>}
      <div className="max-h-[32rem] space-y-2 overflow-y-auto">{query.data?.map(route => <details key={route.id} className="rounded-lg border p-3">
        <summary className="cursor-pointer break-words text-sm"><span className="font-mono">{route.requestedModel || "—"}</span> · {route.status ?? "—"} · {route.durationMs} ms · {route.done ? (route.error ? t("newapi.traceFailed") : t("newapi.traceComplete")) : t("newapi.traceRunning")}</summary>
        <div className="mt-3 space-y-3 text-sm">
          <p className="text-xs text-muted-foreground">{new Date(route.timestamp * 1000).toLocaleString()} · {t("accounts.firstTokenShort")} {route.firstTokenMs == null ? "—" : route.firstTokenMs + " ms"}</p>
          <div><h4 className="mb-1 font-medium">{t("newapi.traceRequested")}</h4><Parameters values={route.requested} empty={t("newapi.traceNoParameters")} /></div>
          {route.attempts.map((attempt, index) => <div key={index} className="space-y-2 rounded-md bg-muted/50 p-3">
            <p className="break-words font-mono text-xs">#{index + 1} · {route.requestedModel} → {attempt.matchedModel} → {attempt.providerName} · {attempt.protocol}</p>
            <p className="break-words text-xs">{t("newapi.upstreamModel")}: {attempt.upstreamModel} · HTTP {attempt.status ?? "—"}</p>
            <h4 className="font-medium">{t("newapi.traceSent")}</h4><Parameters values={attempt.sent} empty={t("newapi.traceNoParameters")} />
            {attempt.error && <p className="text-xs text-destructive">{attempt.failureStage}: {attempt.error}</p>}
          </div>)}
          {route.error && <p className="text-destructive">{route.failureStage}: {route.error}</p>}
        </div>
      </details>)}</div>
    </div>}
  </section>;
}
