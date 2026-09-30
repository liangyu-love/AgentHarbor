import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { CapabilityField } from "@/types/newapi";

function Origin({ field }: { field: CapabilityField<unknown> }) {
  const { t } = useTranslation();
  return <div className="mt-1 text-xs text-muted-foreground">{t("newapi.capabilitySource." + field.source)} · {field.updatedAt ? new Date(field.updatedAt * 1000).toLocaleString() : t("newapi.capabilityDateUnknown")}</div>;
}
export function ModelCapabilities() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const query = useQuery({ queryKey: ["model-capabilities"], queryFn: api.routing.capabilities, enabled: open, refetchInterval: open ? 30_000 : false });
  const models = query.data?.models.filter(m => [m.providerName, m.displayName, m.upstreamModel, m.matchedModel].some(s => s.toLowerCase().includes(search.toLowerCase()))) ?? [];
  return <section className="rounded-xl border bg-card p-4" aria-label={t("newapi.modelCapabilities")}>
    <Button variant="ghost" aria-expanded={open} onClick={() => setOpen(!open)}>{t("newapi.modelCapabilities")}</Button>
    {open && <div className="mt-3 space-y-3">
      <p className="text-xs text-muted-foreground">{t("newapi.capabilityRules")}</p>
      <div className="flex gap-2"><Input value={search} onChange={e => setSearch(e.target.value)} aria-label={t("newapi.capabilitySearch")} placeholder={t("newapi.capabilitySearch")} /><Button variant="outline" disabled={query.isFetching} onClick={() => void query.refetch()}>{t("common.refresh")}</Button></div>
      {query.error && <p role="alert" className="text-sm text-destructive">{formatInvokeError(query.error)}</p>}
      {query.data?.stale && <p role="status" className="text-sm text-amber-600 dark:text-amber-400">{t("newapi.catalogStale")}</p>}
      <div className="max-h-[30rem] space-y-2 overflow-y-auto">{models.map(m => <details key={m.providerId + "::" + m.upstreamModel} className="rounded-lg border p-3 text-sm">
        <summary className="cursor-pointer break-words">{m.providerName} / {m.displayName} → <span className="font-mono">{m.matchedModel}</span></summary>
        <p className="my-3 break-words font-mono text-xs">{t("newapi.upstreamModel")}: {m.upstreamModel}</p>
        <div className="grid gap-4 sm:grid-cols-3">
          <div><h4 className="font-medium">{t("newapi.capabilityReasoning")}</h4><p>{m.reasoning.value.map(v => v.effort).join(" / ") || "—"}</p><Origin field={m.reasoning} /></div>
          <div><h4 className="font-medium">{t("newapi.capabilityContext")}</h4><p>{m.context.value?.toLocaleString() ?? "—"}</p><Origin field={m.context} /></div>
          <div><h4 className="font-medium">{t("newapi.capabilityOutput")}</h4><p>{m.output.value?.toLocaleString() ?? "—"}</p><Origin field={m.output} /></div>
        </div>
      </details>)}</div>
      {!query.isLoading && !models.length && <p className="text-sm text-muted-foreground">{t("newapi.capabilityEmpty")}</p>}
    </div>}
  </section>;
}
