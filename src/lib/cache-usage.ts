import type { CacheUsageTotals, UsageEntry } from "@/types";

/** Aggregate token ratio, never the arithmetic mean of per-request percentages. */
export function cacheHitRate(usage: CacheUsageTotals | undefined): number | null {
  if (!usage || usage.reportedCount === 0 || !Number.isFinite(usage.promptTokens) || usage.promptTokens <= 0
    || !Number.isFinite(usage.readTokens) || usage.readTokens < 0 || usage.readTokens > usage.promptTokens) return null;
  return usage.readTokens / usage.promptTokens * 100;
}

export function requestCacheUsage(record: UsageEntry): CacheUsageTotals | undefined {
  if (!record.usageReported || !record.cacheReported) return undefined;
  return { reportedCount: 1, promptTokens: record.inputTokens + record.cacheReadTokens + record.cacheWriteTokens,
    readTokens: record.cacheReadTokens, writeTokens: record.cacheWriteTokens };
}

export function formatCacheRate(rate: number | null): string {
  return rate == null ? "—" : `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(rate)}%`;
}
