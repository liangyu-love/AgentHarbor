import type { TFunction } from "i18next";

import { clientSourceLabelKey } from "@/lib/client-source";
import { costLabel, formatCost } from "@/lib/format-cost";
import type { UsageChannel, UsageGroup, UsageTotals } from "@/types";

/** Pages read usage by what it cost or by how many new tokens it took. */
export type UsageMetric = "cost" | "tokens";

export function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

export function metricValue(totals: Pick<UsageTotals, "costMicros" | "newTokens">, metric: UsageMetric) {
  return metric === "cost" ? totals.costMicros : totals.newTokens;
}

export function formatMetric(value: number, metric: UsageMetric) {
  return metric === "cost" ? formatCost(value) : formatTokens(value);
}

/** The recorded cost together with its pricing coverage. */
export function totalsCost(totals: UsageTotals, t: TFunction, unknown = t("accounts.costUnavailable")) {
  return costLabel(totals.costMicros, totals.pricedCount, totals.unpricedCount,
    count => t("accounts.unpricedRequests", { count }), unknown, {
      partialCount: totals.partialCount,
      coverageUnknownCount: totals.coverageUnknownCount,
      partialLabel: count => t("accounts.partialRequests", { count }),
      coverageUnknownLabel: count => t("accounts.coverageUnknownRequests", { count }),
    });
}

/** What the cost leaves out or only partly covers; empty when every call was priced. */
export function coverageNotes(totals: UsageTotals, t: TFunction) {
  return [
    totals.unpricedCount ? t("accounts.unpricedRequests", { count: totals.unpricedCount }) : null,
    totals.partialCount ? t("accounts.partialRequests", { count: totals.partialCount }) : null,
    totals.coverageUnknownCount ? t("accounts.coverageUnknownRequests", { count: totals.coverageUnknownCount }) : null,
  ].filter((note): note is string => !!note);
}

/** Percent change against the previous period, or `null` without one. */
export function change(current: number, previous: number): number | null {
  if (!Number.isFinite(previous) || previous <= 0) return null;
  return (current - previous) / previous * 100;
}

export function clientLabel(client: string, t: TFunction) {
  if (client.startsWith("gateway:")) return t("usage.gatewayClient", { name: client.slice("gateway:".length) });
  return t(clientSourceLabelKey(client));
}

export function channelLabel(channel: UsageChannel, name: string | null | undefined, t: TFunction) {
  if (channel === "official") return t("usage.channel.official");
  if (channel === "aggregate") return name ? t("usage.aggregateNamed", { name }) : t("usage.channel.aggregate");
  return name || t("usage.channel.relay");
}

export function groupChannelLabel(group: UsageGroup, t: TFunction) {
  return channelLabel(group.channel ?? "relay", group.label, t);
}

/** A round axis top and step: 1, 2 or 5 times a power of ten. */
export function niceScale(max: number, ticks = 4) {
  if (!Number.isFinite(max) || max <= 0) return { top: ticks, step: 1 };
  const rough = max / ticks;
  const power = 10 ** Math.floor(Math.log10(rough));
  const step = [1, 2, 5, 10].map(factor => factor * power).find(candidate => candidate >= rough) ?? 10 * power;
  return { top: Math.ceil(max / step) * step, step };
}

/** Series fill: the accent in steps for the leaders, a neutral for the rest. */
/** The usage page's eight categorical slots, in their validated order. */
export const SERIES_SLOTS = 8;

export function seriesColor(index: number, key: string) {
  if (key === "other") return "hsl(var(--muted-foreground) / 0.45)";
  return `var(--series-${Math.min(index, SERIES_SLOTS - 1) + 1})`;
}
