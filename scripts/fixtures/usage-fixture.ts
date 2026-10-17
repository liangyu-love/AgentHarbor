// Synthetic usage for the frontend regressions: a list of calls aggregated the
// way the backend ledger does, so patched entries stay consistent everywhere.
import type { UsageBucket, UsageEntry, UsageGroup, UsagePart, UsagePayload, UsageRange, UsageTotals } from "@/types";

const HOUR = 3600;
const DAY = 24 * HOUR;

export function emptyTotals(): UsageTotals {
  return {
    requestCount: 0, inputTokens: 0, outputTokens: 0, reasoningTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0,
    totalTokens: 0, newTokens: 0, costMicros: 0, pricedCount: 0, unpricedCount: 0, partialCount: 0,
    coverageUnknownCount: 0, estimatedMicros: 0, estimatedCount: 0, cacheSavingsMicros: 0,
    cacheUsage: { reportedCount: 0, promptTokens: 0, readTokens: 0, writeTokens: 0 },
  };
}

const fresh = (entry: UsageEntry) => entry.inputTokens + entry.cacheWriteTokens + entry.outputTokens;
const micros = (usd: number) => Math.round(usd * 1_000_000);

function add(totals: UsageTotals, entry: UsageEntry) {
  totals.requestCount += 1;
  totals.inputTokens += entry.inputTokens;
  totals.outputTokens += entry.outputTokens;
  totals.reasoningTokens += entry.reasoningTokens;
  totals.cacheReadTokens += entry.cacheReadTokens;
  totals.cacheWriteTokens += entry.cacheWriteTokens;
  totals.totalTokens += entry.totalTokens;
  totals.newTokens += fresh(entry);
  if (entry.usageReported && entry.cacheReported) {
    totals.cacheUsage.reportedCount += 1;
    totals.cacheUsage.promptTokens += entry.inputTokens + entry.cacheReadTokens + entry.cacheWriteTokens;
    totals.cacheUsage.readTokens += entry.cacheReadTokens;
    totals.cacheUsage.writeTokens += entry.cacheWriteTokens;
  }
  if (entry.costUsd != null && entry.pricingStatus !== "unpriced") {
    totals.costMicros += micros(entry.costUsd);
    totals.pricedCount += 1;
    if (entry.pricingStatus === "partial") totals.partialCount += 1;
    else if (entry.pricingStatus !== "priced") totals.coverageUnknownCount += 1;
    if (entry.estimated) {
      totals.estimatedMicros += micros(entry.costUsd);
      totals.estimatedCount += 1;
    }
  } else {
    totals.unpricedCount += 1;
  }
  totals.cacheSavingsMicros += micros(entry.cacheSavingsUsd ?? 0);
}

function grouped(entries: UsageEntry[], key: (entry: UsageEntry) => string | null | undefined,
  make: (entry: UsageEntry, key: string) => Omit<UsageGroup, "totals">) {
  const groups = new Map<string, UsageGroup>();
  for (const entry of entries) {
    const id = key(entry);
    if (!id) continue;
    if (!groups.has(id)) groups.set(id, { ...make(entry, id), totals: emptyTotals() });
    add(groups.get(id)!.totals, entry);
  }
  return [...groups.values()].sort((a, b) => b.totals.newTokens - a.totals.newTokens || b.totals.costMicros - a.totals.costMicros);
}

function channelKey(entry: UsageEntry) {
  return entry.channel === "official" ? "official"
    : entry.channel === "relay" ? `relay:${entry.providerId ?? ""}` : `aggregate:${entry.requestedModel ?? entry.model}`;
}

function addPart(parts: UsagePart[], key: string, entry: UsageEntry) {
  let part = parts.find(item => item.key === key);
  if (!part) parts.push(part = { key, newTokens: 0, costMicros: 0 });
  part.newTokens += fresh(entry);
  if (entry.pricingStatus !== "unpriced") part.costMicros += micros(entry.costUsd ?? 0);
}

const midnight = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime() / 1000;
const isoDate = (seconds: number) => {
  const date = new Date(seconds * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
};

export function buildUsage(range: UsageRange, entries: UsageEntry[], now = new Date()): UsagePayload {
  const days = range === "today" ? 1 : range === "week" ? 7 : 30;
  const nowSeconds = Math.floor(now.getTime() / 1000);
  const today = midnight(now);
  const start = today - (days - 1) * DAY;
  const previousStart = start - days * DAY;
  const span = nowSeconds - start;
  const inRange = entries.filter(entry => entry.timestamp >= start);
  const current = emptyTotals();
  const previous = emptyTotals();
  const todayTotals = emptyTotals();
  const allTime = emptyTotals();
  for (const entry of entries) {
    add(allTime, entry);
    if (entry.timestamp >= today) add(todayTotals, entry);
    if (entry.timestamp >= previousStart && entry.timestamp < previousStart + span) add(previous, entry);
    if (entry.timestamp >= start) add(current, entry);
  }
  const byClient = grouped(inRange, entry => entry.client, (entry, key) => ({ key, label: key, client: key }));
  const byChannel = grouped(inRange, channelKey, (entry, key) => ({
    key, channel: entry.channel, providerId: entry.channel === "relay" ? entry.providerId : null,
    label: entry.channel === "official" ? "" : entry.channel === "relay" ? entry.providerName ?? "" : entry.requestedModel ?? entry.model,
  }));
  const byModel = grouped(inRange, entry => entry.model.toLowerCase(), (entry, key) => ({ key, label: entry.model }))
    .map(group => ({ ...group, detail: [...new Set(inRange.filter(entry => entry.model.toLowerCase() === group.key)
      .map(entry => entry.channel === "official" ? "official" : entry.providerName ?? channelKey(entry)))].join(" · ") }));
  const projectName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;
  const byProject = grouped(inRange, entry => entry.project, (entry, key) => ({ key, label: projectName(key), detail: key }));
  const bySession = grouped(inRange, entry => entry.sessionId ? `${entry.client}:${entry.sessionId}` : null, entry => ({
    key: `${entry.client}:${entry.sessionId}`, label: entry.sessionId!, client: entry.client,
    detail: entry.project ? projectName(entry.project) : null,
  }));
  const top = (groups: UsageGroup[]) => new Set(groups.slice(0, 6).map(group => group.key));
  const clientKeys = top(byClient);
  const modelKeys = top(byModel);
  const buckets: UsageBucket[] = Array.from({ length: range === "today" ? 24 : days }, (_, index) => {
    const bucketStart = range === "today" ? start + index * HOUR : start + index * DAY;
    return { key: range === "today" ? String(index).padStart(2, "0") : isoDate(bucketStart), start: bucketStart,
      totals: emptyTotals(), byClient: [], byChannel: [], byModel: [] };
  });
  for (const entry of inRange) {
    const index = range === "today" ? new Date(entry.timestamp * 1000).getHours() : Math.floor((entry.timestamp - start) / DAY);
    const bucket = buckets[index];
    if (!bucket) continue;
    add(bucket.totals, entry);
    addPart(bucket.byClient, clientKeys.has(entry.client) ? entry.client : "other", entry);
    addPart(bucket.byChannel, entry.channel, entry);
    addPart(bucket.byModel, modelKeys.has(entry.model.toLowerCase()) ? entry.model.toLowerCase() : "other", entry);
  }
  const relay = entries.filter(entry => entry.request);
  return {
    generatedAt: nowSeconds, range, start, previousStart, today: todayTotals, current, previous, allTime, buckets,
    byClient, byChannel, byModel: byModel.slice(0, 12), byProject: byProject.slice(0, 12), bySession: bySession.slice(0, 12),
    recent: [...inRange].sort((a, b) => b.timestamp - a.timestamp).slice(0, 200),
    sources: {
      transcripts: [
        { sourceId: "codex", available: true, fileCount: 153, updatedFileCount: 4 },
        { sourceId: "claude", available: true, fileCount: 5, updatedFileCount: 1 },
      ],
      relaySince: relay.length ? Math.min(...relay.map(entry => entry.timestamp)) : null,
      relayCount: relay.length,
      matchedCount: relay.filter(entry => entry.request?.matched).length,
    },
    warningCount: 0, warnings: [],
  };
}

export function usageEntry(overrides: Partial<UsageEntry> & Pick<UsageEntry, "timestamp">): UsageEntry {
  return {
    client: "codex", channel: "official", model: "gpt-6-sol", inputTokens: 1_000, outputTokens: 200, reasoningTokens: 0,
    cacheReadTokens: 0, cacheWriteTokens: 0, totalTokens: 1_200, usageReported: true, cacheReported: true,
    costUsd: 0.01, pricingStatus: "priced", estimated: true, pricingSource: "catalogReference", ...overrides,
  };
}

const relayRequest = (durationMs: number, firstTokenMs: number | null, statusCode = 200, matched = false) => ({
  statusCode, success: statusCode < 400, durationMs, firstTokenMs, servedModel: null, modelMatchStatus: "unknown",
  credentialStatus: "recorded", credentialFingerprint: "fingerprint-0123456789", matched,
});

/** Two weeks of mixed traffic: official accounts, relays, an aggregate and other clients. */
export function defaultUsageEntries(now = new Date()): UsageEntry[] {
  const at = (daysAgo: number, hour: number) => {
    const date = new Date(now.getFullYear(), now.getMonth(), now.getDate() - daysAgo, hour, 15);
    return Math.min(Math.floor(date.getTime() / 1000), Math.floor(now.getTime() / 1000) - 60);
  };
  const entries: UsageEntry[] = [];
  const shape = [26, 20, 2, 1, 1, 4, 35, 68, 4, 2, 3, 123, 78, 74];
  shape.forEach((size, offset) => {
    const daysAgo = shape.length - 1 - offset;
    entries.push(usageEntry({ timestamp: at(daysAgo, 9), model: "gpt-6-sol", project: "D:\\work\\AgentHarbor", sessionId: `codex-session-${daysAgo % 4}`,
      inputTokens: size * 9_000, cacheReadTokens: size * 120_000, outputTokens: size * 1_400, reasoningTokens: size * 400,
      totalTokens: size * 130_400, costUsd: size * 0.042, cacheSavingsUsd: size * 0.11 }));
    entries.push(usageEntry({ timestamp: at(daysAgo, 14), client: "claudeCode", model: "claude-opus-5-5", project: "/home/me/aimami",
      sessionId: `claude-session-${daysAgo % 3}`, inputTokens: size * 1_500, cacheReadTokens: size * 60_000, cacheWriteTokens: size * 3_000,
      outputTokens: size * 900, totalTokens: size * 65_400, costUsd: size * 0.093, cacheSavingsUsd: size * 0.27 }));
  });
  entries.push(usageEntry({ timestamp: at(0, 0), channel: "relay", providerId: "group-a", providerName: "Relay Alpha",
    model: "gpt-6-luna", project: "D:\\work\\AgentHarbor", sessionId: "codex-session-0", inputTokens: 62_988, outputTokens: 12,
    totalTokens: 63_000, costUsd: 0.00126, estimated: false, pricingSource: "catalogReference", request: relayRequest(1_140, 320, 200, true) }));
  entries.push(usageEntry({ timestamp: at(1, 11), client: "opencode", channel: "relay", providerId: "group-a", providerName: "Relay Alpha",
    model: "glm-5.3", inputTokens: 40_000, outputTokens: 2_000, totalTokens: 42_000, costUsd: 0.03, estimated: false,
    pricingSource: "manual", request: relayRequest(2_400, 610) }));
  entries.push(usageEntry({ timestamp: at(2, 16), client: "cursor", channel: "relay", providerId: "group-b", providerName: "Relay Beta",
    model: "deepseek-v4-pro", inputTokens: 20_000, outputTokens: 1_000, totalTokens: 21_000, costUsd: null, pricingStatus: "unpriced",
    estimated: false, pricingSource: null, request: relayRequest(900, null, 502) }));
  entries.push(usageEntry({ timestamp: at(3, 10), client: "claudeDesktop", channel: "aggregate", providerId: "group-a", providerName: "Relay Alpha",
    requestedModel: "claude-sonnet-4-6", model: "deepseek-fixture", inputTokens: 5_000, outputTokens: 800, totalTokens: 5_800,
    costUsd: 0.002, estimated: false, pricingSource: "catalogReference", request: relayRequest(1_800, 450) }));
  return entries;
}
