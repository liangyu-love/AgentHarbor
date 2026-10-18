import { Fragment, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronRight } from "lucide-react";

import { cacheHitRate, formatCacheRate, requestCacheUsage } from "@/lib/cache-usage";
import { formatCost } from "@/lib/format-cost";
import { cn } from "@/lib/utils";
import { useScrollPaging } from "@/hooks/use-scroll-paging";
import type { UsageChannel, UsageEntry } from "@/types";
import { channelLabel, clientLabel, formatTokens } from "./usage-format";

const PAGE = 50;

function entryKey(entry: UsageEntry, index: number) {
  return `${entry.timestamp}-${entry.client}-${entry.model}-${index}`;
}

/** Every call of the range in one table; the router's diagnostics fold into each row. */
export function UsageRequests({ entries }: { entries: UsageEntry[] }) {
  const { t } = useTranslation();
  const [client, setClient] = useState("all");
  const [channel, setChannel] = useState<"all" | UsageChannel>("all");
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const clients = useMemo(() => [...new Set(entries.map(entry => entry.client))]
    .sort((a, b) => clientLabel(a, t).localeCompare(clientLabel(b, t))), [entries, t]);
  const filtered = entries.filter(entry => (client === "all" || entry.client === client)
    && (channel === "all" || entry.channel === channel));
  const paging = useScrollPaging(filtered.length, PAGE);
  const toggle = (key: string) => setOpen(current => {
    const next = new Set(current);
    if (!next.delete(key)) next.add(key);
    return next;
  });
  const pricingTitle = (entry: UsageEntry) => {
    if (!entry.usageReported) return t("accounts.usageNotReported");
    const parts = [
      entry.pricingStatus === "partial" ? t("accounts.costPartial") : null,
      entry.pricingSource ? t(`newapi.capabilitySource.${entry.pricingSource}`, { defaultValue: entry.pricingSource })
        : t("accounts.costUnavailable"),
      entry.multiplier != null ? t("newapi.priceMultiplierApplied", { value: entry.multiplier }) : null,
      entry.estimated ? t("usage.kpi.costHint") : null,
    ];
    return parts.filter(Boolean).join(" · ");
  };

  return <section className="harbor-card route-rise min-w-0" aria-labelledby="usage-requests-title" data-usage-requests>
    <div className="usage-card-head">
      <div className="usage-card-title">
        <span className="poster-kicker text-[9.5px] text-primary">Requests</span>
        <h3 id="usage-requests-title" className="font-display">{t("usage.requests.title")}</h3>
        <p>{t("usage.requests.subtitle", { count: entries.length })}</p>
      </div>
      <div className="usage-filters">
        <label className="usage-filter">{t("newapi.routeSourceLabel")}
          <select aria-label={t("newapi.routeSourceLabel")} value={client} onChange={event => { setClient(event.target.value); paging.reset(); }}>
            <option value="all">{t("newapi.routeSourceAll")}</option>
            {clients.map(value => <option key={value} value={value}>{clientLabel(value, t)}</option>)}
          </select>
        </label>
        <label className="usage-filter">{t("usage.requests.channelLabel")}
          <select aria-label={t("usage.requests.channelLabel")} value={channel} onChange={event => { setChannel(event.target.value as "all" | UsageChannel); paging.reset(); }}>
            <option value="all">{t("usage.requests.all")}</option>
            {(["official", "relay", "aggregate"] as const).map(value => <option key={value} value={value}>{t(`usage.channel.${value}`)}</option>)}
          </select>
        </label>
      </div>
    </div>
    {filtered.length === 0 ? <p className="usage-rank-empty">{t("usage.requests.empty")}</p> : <div className="usage-table-wrap">
      <table className="usage-table">
        <thead><tr>
          <th>{t("usage.requests.time")}</th><th>{t("usage.requests.client")}</th><th>{t("usage.requests.channel")}</th>
          <th>{t("usage.requests.model")}</th><th>{t("usage.requests.tokens")}</th><th>{t("usage.requests.cost")}</th>
          <th>{t("usage.requests.latency")}</th><th>{t("usage.requests.status")}</th>
        </tr></thead>
        <tbody>{filtered.slice(0, paging.limit).map((entry, index) => {
          const key = entryKey(entry, index);
          const expanded = open.has(key);
          const cache = requestCacheUsage(entry);
          const newTokens = entry.inputTokens + entry.cacheWriteTokens + entry.outputTokens;
          const mapped = entry.requestedModel && entry.requestedModel !== entry.model;
          const facts = entry.request;
          return <Fragment key={key}>
            <tr data-open={expanded || undefined} data-usage-entry={entry.channel}>
              <td data-num>{new Date(entry.timestamp * 1000).toLocaleString()}</td>
              <td><span data-usage-client={entry.client}>{clientLabel(entry.client, t)}</span></td>
              <td>{channelLabel(entry.channel, entry.channel === "aggregate" ? entry.requestedModel ?? entry.model : entry.providerName, t)}</td>
              <td className="max-w-[16rem] font-mono">
                <button type="button" className="usage-row-toggle" aria-expanded={expanded}
                  aria-label={`${t("usage.requests.details")} ${entry.model}`} onClick={() => toggle(key)}>
                  <ChevronRight aria-hidden /><span className="truncate">{entry.model || "—"}</span>
                </button>
                {mapped && <small>{entry.requestedModel} → {entry.model}</small>}
              </td>
              <td data-num>
                {entry.usageReported ? t("usage.requests.newTokens", { value: formatTokens(newTokens) }) : "—"}
                <small title={t("accounts.cacheRateFormula")}>
                  {t("accounts.cacheHitRate")} {formatCacheRate(cacheHitRate(cache))}
                  {(entry.cacheReported || entry.cacheReadTokens + entry.cacheWriteTokens > 0)
                    && ` · ${t("accounts.cacheReadWrite", { read: formatTokens(entry.cacheReadTokens), write: formatTokens(entry.cacheWriteTokens) })}`}
                </small>
              </td>
              <td data-num title={pricingTitle(entry)}>
                {!entry.usageReported ? t("accounts.usageNotReported")
                  : entry.costUsd != null && entry.pricingStatus !== "unpriced" ? formatCost(entry.costUsd * 1_000_000) : t("accounts.costUnpriced")}
                {entry.pricingStatus === "partial" && <span className="ml-1 text-amber-600">*</span>}
                {entry.estimated && <small>{t("usage.requests.estimated")}</small>}
              </td>
              <td data-num>{facts ? <>{facts.durationMs} ms<small>{t("accounts.firstTokenShort")} {facts.firstTokenMs == null ? "—" : `${facts.firstTokenMs} ms`}</small></> : "—"}</td>
              <td>{facts
                ? <span className={cn("usage-tag", facts.success ? "is-primary" : "is-error")}>{facts.statusCode}</span>
                : <span className="usage-tag">{t("usage.requests.local")}</span>}</td>
            </tr>
            {expanded && <tr data-open><td colSpan={8}>
              <dl className="usage-detail">
                {entry.sessionId && <div><dt>{t("usage.requests.session")}</dt><dd className="font-mono">{entry.sessionId}</dd></div>}
                {entry.project && <div><dt>{t("usage.requests.project")}</dt><dd>{entry.project}</dd></div>}
                {mapped && <div><dt>{t("usage.requests.requested")}</dt><dd className="font-mono">{entry.requestedModel}</dd></div>}
                {facts && <div><dt>{t("usage.requests.served")}</dt><dd>
                  {facts.servedModel ?? "—"} · {t(`accounts.relayModelStatus.${facts.modelMatchStatus || "unknown"}`, { defaultValue: facts.modelMatchStatus })}
                </dd></div>}
                {facts && <div><dt>{t("usage.requests.credential")}</dt><dd>
                  {t(`accounts.relayCredentialStatus.${facts.credentialStatus || "not_recorded"}`, { defaultValue: facts.credentialStatus })}
                  {facts.credentialFingerprint && ` · ${facts.credentialFingerprint.slice(0, 12)}`}
                </dd></div>}
                <div><dt>{t("usage.requests.cache")}</dt><dd>
                  {t("accounts.cacheReadWrite", { read: formatTokens(entry.cacheReadTokens), write: formatTokens(entry.cacheWriteTokens) })}
                </dd></div>
                {entry.reasoningTokens > 0 && <div><dt>{t("usage.requests.reasoning")}</dt><dd>{formatTokens(entry.reasoningTokens)}</dd></div>}
                <div><dt>{t("usage.requests.pricing")}</dt><dd>{pricingTitle(entry)}</dd></div>
                {facts?.matched && <div><dt>{t("usage.requests.session")}</dt><dd>{t("usage.requests.matched")}</dd></div>}
              </dl>
            </td></tr>}
          </Fragment>;
        })}</tbody>
      </table>
      {paging.more && <div ref={paging.endRef} aria-hidden data-usage-requests-end className="h-px" />}
    </div>}
  </section>;
}
