import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowDownRight, ArrowUpRight, Minus } from "lucide-react";

import { CountUp } from "@/components/ui/count-up";
import { cacheHitRate, formatCacheRate } from "@/lib/cache-usage";
import { formatCost } from "@/lib/format-cost";
import type { UsagePayload } from "@/types";
import { change, coverageNotes, formatTokens } from "./usage-format";

/** How a figure moved against the same span before it; direction only, no verdict. */
function Delta({ value, unit, label }: { value: number | null; unit: "percent" | "points"; label: string }) {
  const { t } = useTranslation();
  if (value == null) return <span className="usage-delta" title={label}>{t("usage.deltaNone")}</span>;
  const rounded = Math.abs(value) >= 10 ? Math.round(value) : Math.round(value * 10) / 10;
  const Icon = rounded > 0 ? ArrowUpRight : rounded < 0 ? ArrowDownRight : Minus;
  const text = unit === "points"
    ? t("usage.points", { value: `${rounded > 0 ? "+" : ""}${rounded}` })
    : `${rounded > 0 ? "+" : ""}${rounded}%`;
  return <span className="usage-delta" title={label} data-delta={rounded > 0 ? "up" : rounded < 0 ? "down" : "flat"}>
    <Icon aria-hidden />{text}<span className="sr-only"> {label}</span>
  </span>;
}

function Kpi({ kicker, label, value, delta, note, title }: {
  kicker: string; label: string; value: ReactNode; delta: ReactNode; note?: ReactNode; title?: string;
}) {
  return <div className="usage-kpi" title={title} data-usage-kpi={kicker.toLowerCase()}>
    <span className="usage-kpi-label"><span className="poster-kicker text-[9.5px] text-primary">{kicker}</span>{label}</span>
    <strong className="usage-kpi-value poster-number">{value}</strong>
    {delta}
    {note && <span className="usage-kpi-note">{note}</span>}
  </div>;
}

export function UsageKpis({ data }: { data: UsagePayload }) {
  const { t } = useTranslation();
  const { current, previous } = data;
  const compare = t(`usage.compare.${data.range}`);
  const rate = cacheHitRate(current.cacheUsage);
  const previousRate = cacheHitRate(previous.cacheUsage);
  const priced = current.pricedCount > 0;
  const costNotes = [
    current.estimatedMicros > 0 ? t("usage.kpi.estimated", { value: formatCost(current.estimatedMicros) }) : null,
    ...coverageNotes(current, t),
  ].filter(Boolean).join(" · ");
  return <section className="harbor-card route-rise" aria-label={t("usage.title")}>
    <div className="usage-kpis">
      <Kpi kicker="Cost" label={t("usage.kpi.cost")} title={t("usage.kpi.costHint")}
        value={priced ? <CountUp value={current.costMicros} format={formatCost} /> : "—"}
        delta={<Delta value={priced ? change(current.costMicros, previous.costMicros) : null} unit="percent" label={compare} />}
        note={priced ? costNotes || undefined : t("usage.kpi.noCost")} />
      <Kpi kicker="Tokens" label={t("usage.kpi.tokens")} title={t("usage.kpi.tokensHint")}
        value={<CountUp value={current.newTokens} format={formatTokens} />}
        delta={<Delta value={change(current.newTokens, previous.newTokens)} unit="percent" label={compare} />}
        note={current.cacheReadTokens > 0 ? t("usage.kpi.cacheRead", { value: formatTokens(current.cacheReadTokens) }) : undefined} />
      <Kpi kicker="Cache" label={t("usage.kpi.cache")} title={t("accounts.cacheRateFormula")}
        value={formatCacheRate(rate)}
        delta={<Delta value={rate != null && previousRate != null ? rate - previousRate : null} unit="points" label={compare} />}
        note={current.cacheSavingsMicros > 0 ? t("usage.kpi.savings", { value: formatCost(current.cacheSavingsMicros) }) : undefined} />
      <Kpi kicker="Requests" label={t("usage.kpi.requests")}
        value={<CountUp value={current.requestCount} format={value => new Intl.NumberFormat().format(Math.round(value))} />}
        delta={<Delta value={change(current.requestCount, previous.requestCount)} unit="percent" label={compare} />}
        note={t("usage.kpi.requestsTotal", { value: new Intl.NumberFormat().format(data.allTime.requestCount) })} />
    </div>
  </section>;
}
