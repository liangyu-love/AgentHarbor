// Synthetic browser fixture: every application API used here is mocked.
import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@/components/ui/tooltip";
import { PageStage } from "@/components/layout/page-stage";
import { VirtualThreadList } from "@/components/threads/virtual-thread-list";
import { ThreadsPage } from "@/components/threads/threads-page";
import { ThreadRecoveryDialog } from "@/components/threads/thread-recovery-dialog";
import { ImageWorkbenchPage } from "@/components/image-workbench/image-workbench-page";
import { ProviderDialog } from "@/components/newapi/provider-dialog";
import { ProviderTestDialog } from "@/components/newapi/provider-test-dialog";
import { NewApiPage } from "@/components/newapi/newapi-page";
import { RelayModelsPage } from "@/components/newapi/relays-page";
import { ClaudeDesktopRouterExitDialog } from "@/components/newapi/claude-desktop-router-exit-dialog";
import { UpdateOverlay } from "@/components/update/update-overlay";
import { useUpdateCheck } from "@/hooks/use-update-check";
import { OverviewPage } from "@/components/overview/overview-page";
import { AccountsPage } from "@/components/accounts/accounts-page";
import { TokenUsagePanel } from "@/components/accounts/token-usage-panel";
import { McpPage } from "@/components/mcp/mcp-page";
import { SkillsPage } from "@/components/skills/skills-page";
import { ClientConfigs } from "@/components/newapi/client-configs";
import { TerminalProxySettings } from "@/components/settings/terminal-proxy-settings";
import type { CodexSubagentSettings, NewApiProvider, NewApiState } from "@/types/newapi";
import type { TokenUsagePayload } from "@/types";
import type { ClientConfigRequest, ClientStatus } from "@/types/clients";
import type { AggregateModel, AggregateView, AggregationState, ConnectionExportResult } from "@/types/aggregation";
import type { ThreadSummary } from "@/types/threads";
import { api } from "@/lib/api";
import { useToast } from "@/hooks/use-toast";
import i18n from "@/lib/i18n";
import { errorKey, errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { claudeOneMDefaults, clientDefaults } from "@/lib/client-config";
import { mergeModelCatalog } from "@/lib/provider-models";

const realBalanceQuery = api.providers.queryBalance;
const controls = window as any;
controls.__TAURI_INTERNALS__ = {};
const root = createRoot(document.getElementById("root")!);
function ThreadToastProbe() {
  const { toasts } = useToast();
  controls.review.lastToast = toasts[0]?.title;
  controls.review.lastToastDescription = toasts[0]?.description;
  return null;
}
function DesktopUpdaterFixture() {
  const update = useUpdateCheck();
  return <>
    <button onClick={() => void update.checkForUpdate()}>检查测试更新</button>
    <p data-updater-status>{update.status}</p>
    <ClaudeDesktopRouterExitDialog />
    {update.status !== "idle" && <UpdateOverlay status={update.status} currentVersion={update.updateInfo?.currentVersion ?? "1.0.0"}
      newVersion={update.updateInfo?.version} body={update.updateInfo?.body} progress={update.progress} error={update.error}
      onInstall={() => void update.installUpdate()} onRetry={() => void update.checkForUpdate()} onSkip={update.dismiss} />}
  </>;
}
const noop = () => {};
const model = (id: string) => ({ id, displayName: id, contextWindow: null, maxOutputTokens: null,
  endpointTypes: ["responses"], included: true, metadataSource: "upstream" as const,
  overrides: {}, catalogRef: null, priceRef: null, compression: { mode: "inherit" as const } });
const provider = (id: string) => ({ id, name: "Provider " + id, model: "model-" + id,
  models: [model("model-" + id)], hasApiKey: true, baseUrl: "https://" + id + ".invalid",
  enabled: true, protocol: "responses" as const, userId: null, hasAccessToken: false,
  contextWindow: null, autoCompactLimit: null, catalogProviders: [], priceCatalogProvider: null,
  compression: { mode: "inherit" as const }, networkMode: "direct" as const,
  modelsFetchedAt: null, balance: null, balanceSource: null, testStatus: "untested" as const,
  testAt: null, createdAt: 0, updatedAt: 0 });
const imageProvider = { ...provider("image"), model: "gpt-image-2", models: [model("gpt-image-2")] };
const groupedProviders: NewApiProvider[] = [
  { ...provider("group-a"), name: "Relay Alpha", baseUrl: "https://relay.invalid/v1", model: "alpha-model" },
  { ...provider("group-b"), name: "Relay Beta", baseUrl: "https://relay.invalid/v1/", model: "beta-model", balanceSharedFrom: "group-a" },
  { ...provider("group-c"), name: "Relay Gamma", baseUrl: "https://relay.invalid/v1", model: "gamma-model", balanceSharedFrom: "group-a" },
  { ...provider("solo"), name: "Solo provider", baseUrl: "https://solo.invalid/v1" },
];
const mixedProvider: NewApiProvider = { ...provider("mixed"), protocol: "chatCompletions", model: "responses-model", models: [
  { ...model("responses-model"), protocol: "responses" },
  { ...model("chat-model"), protocol: null, endpointTypes: ["chat_completions"] },
  { ...model("anthropic-model"), protocol: "anthropicMessages", endpointTypes: ["anthropic_messages"] },
  { ...model("inherited-model"), endpointTypes: [] },
] };
const versionedProvider: NewApiProvider = { ...provider("versions"), protocol: "anthropicMessages",
  model: "claude-opus-5-5", models: [
    ["claude-opus-5-5", "Opus 5.5"], ["claude-fable-5-1", "Fable 5.1"], ["claude-opus-4-8", "Opus 4.8"],
  ].map(([id, displayName]) => ({ ...model(id), displayName, protocol: "anthropicMessages", endpointTypes: ["anthropic_messages"] })) };
const galleryItems = Array.from({ length: 100 }, (_, i) => ({
  name: "gallery-" + i + ".png", path: "C:/synthetic/gallery-" + i + ".png", size: 100, modifiedAt: i,
}));
const pixel = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+nmQAAAABJRU5ErkJggg==";
const tokens = (totalTokens: number, inputTokens: number, outputTokens: number, cacheReadTokens = 0) => ({
  turnCount: 10, totalTokens, inputTokens, outputTokens, reasoningTokens: 0, cacheReadTokens, cacheWriteTokens: 0,
});
const relayTotals = (totalTokens: number, inputTokens: number, outputTokens: number, requestCount = 8) => ({
  totalTokens, inputTokens, outputTokens, reasoningTokens: 0, requestCount, successfulCount: requestCount, reportedCount: requestCount,
  cacheReadTokens: 0, cacheWriteTokens: 0, costMicros: Math.round(totalTokens * 0.00002 * 1_000_000), pricedCount: requestCount, unpricedCount: 0,
  fullyPricedCount: requestCount, partialCount: 0, coverageUnknownCount: 0,
});
const usageFixture: TokenUsagePayload = {
  generatedAt: Math.floor(Date.now() / 1000),
  local: {
    generatedAt: Math.floor(Date.now() / 1000), today: tokens(74_100_000, 2_030_000, 292_000, 71_778_000),
    week: tokens(510_000_000, 15_790_000, 2_476_000, 491_734_000),
    allTime: tokens(2_060_000_000, 170_000_000, 8_730_000, 1_881_270_000),
    bySource: [
      { sourceId: "codex", sourceName: "Codex", available: true, fileCount: 153, updatedFileCount: 4, totals: tokens(1_900_000_000, 170_000_000, 8_730_000, 1_721_270_000) },
      { sourceId: "claude", sourceName: "Claude Code", available: true, fileCount: 5, updatedFileCount: 1, totals: tokens(160_000_000, 2_940_000, 397_000, 156_663_000) },
    ],
    byModel: [
      { sourceId: "codex", sourceName: "Codex", model: "gpt-5.6-sol", totals: tokens(1_180_000_000, 80_000_000, 4_000_000, 1_096_000_000) },
      { sourceId: "codex", sourceName: "Codex", model: "gpt-6-sol", totals: tokens(260_000_000, 20_000_000, 1_500_000, 238_500_000) },
      { sourceId: "claude", sourceName: "Claude Code", model: "claude-opus-5.5", totals: tokens(160_000_000, 2_940_000, 397_000, 156_663_000) },
      { sourceId: "codex", sourceName: "Codex", model: "gpt-6-astra", totals: tokens(100_000_000, 7_000_000, 300_000, 92_700_000) },
    ],
    daily: Array.from({ length: 14 }, (_, index) => {
      const values = [26, 20, 2, 1, 1, 4, 35, 68, 4, 2, 3, 123, 78, 74];
      const total = values[index] * 1_000_000;
      return { date: `2026-09-${String(index + 14).padStart(2, "0")}`, totals: tokens(total, Math.round(total * .05), Math.round(total * .012), Math.round(total * .938)),
        byModel: [
          { sourceId: "codex", sourceName: "Codex", model: "gpt-5.6-sol", totals: tokens(Math.round(total * .62), 0, 0) },
          { sourceId: "codex", sourceName: "Codex", model: "gpt-6-sol", totals: tokens(Math.round(total * .24), 0, 0) },
          { sourceId: "claude", sourceName: "Claude Code", model: "claude-opus-5.5", totals: tokens(Math.round(total * .14), 0, 0) },
        ] };
    }),
    recent: [], warningCount: 0, warnings: [],
  },
  relay: {
    generatedAt: Math.floor(Date.now() / 1000), today: relayTotals(63_000, 62_988, 12, 1),
    week: relayTotals(120_000, 116_000, 4_000, 3), allTime: relayTotals(330_000, 310_000, 20_000, 5),
    lastSevenDays: relayTotals(220_000, 216_000, 4_000, 4), lastSevenDaysStart: "2026-09-21", lastSevenDaysEnd: "2026-09-27", statisticalTimezone: "local (+08:00)",
    byProvider: [{ providerId: "group-a", providerName: "Relay Alpha", totals: relayTotals(330_000, 310_000, 20_000, 5) }],
    byModel: [{ providerId: "group-a", providerName: "Relay Alpha", model: "gpt-6-luna", totals: relayTotals(330_000, 310_000, 20_000, 5) }],
    daily: Array.from({ length: 14 }, (_, index) => ({ date: `2026-09-${String(index + 14).padStart(2, "0")}`, totals: relayTotals(index === 13 ? 63_000 : index * 1_200, index === 13 ? 62_988 : index * 1_000, index === 13 ? 12 : index * 200, index === 13 ? 1 : 0) })),
    recent: [{ timestamp: Math.floor(Date.now() / 1000), date: "2026-09-27", providerId: "group-a", providerName: "Relay Alpha", model: "gpt-6-luna", protocol: "responses", statusCode: 200, durationMs: 1140, firstTokenMs: 320, success: true, usageReported: true, inputTokens: 62_988, outputTokens: 12, reasoningTokens: 0, totalTokens: 63_000, cacheReadTokens: 0, cacheWriteTokens: 0, costUsd: 0.00126, pricingStatus: "priced", pricingSource: "catalog", catalogRevision: "abcdef" }],
  },
};
let state: NewApiState = { providers: [imageProvider], router: { enabled: false, accessMode: "router", accountless: false, port: 0, configDrifted: false }, proxyRunning: false, activeProviderId: null };
api.providers.loadState = async () => state;
// Aggregation fixtures reuse exact saved provider/model identities. Availability
// changes with those same fixture providers, rather than inventing a fallback.
const aggregateProjection = (contextWindow = 1_000_000) => ({ contextWindow, contextSource: "catalog", contextIsFallback: false,
  budget: contextWindow, budgetSource: "catalog", inputLimit: null, outputLimit: 8192, outputSource: "catalog",
  reasoning: [], reasoningSource: "catalog", defaultReasoning: null,
  compression: { mode: "percent", percent: 90, tokens: null, source: "global", budget: contextWindow,
    budgetSource: "catalog", threshold: Math.floor(contextWindow * .9), state: "computed", note: null } });
const aggregationState = (): AggregationState => ({
  models: (controls.review.aggregateModels ?? []).map((entry: AggregateModel): AggregateView => {
    const memberStates = entry.members.map(member => {
      const p = state.providers.find(item => item.id === member.providerId);
      const m = p?.models.find(item => item.id === member.actualModelId && item.included);
      const available = member.enabled && !!p?.enabled && !!p?.hasApiKey && (!!m || p?.model === member.actualModelId);
      const context = m?.contextWindow ?? p?.contextWindow ?? 1_000_000;
      return { memberId: member.id, providerName: p?.name ?? member.providerId, protocol: m?.protocol ?? p?.protocol ?? null,
        available, reason: available ? null : !p ? "provider_missing" : !p.enabled ? "provider_disabled" : !m ? "model_missing" : "member_disabled",
        projection: available ? aggregateProjection(context) : null } as AggregateView["memberStates"][number];
    });
    const eligible = memberStates.filter(member => member.available);
    return { ...structuredClone(entry), available: entry.enabled && eligible.length > 0,
      reason: !entry.enabled ? "aggregate_disabled" : eligible.length ? null : "no_eligible_members",
      projection: eligible.length ? aggregateProjection(Math.min(...eligible.map(member => member.projection!.contextWindow))) as AggregateView["projection"] : null,
      routeRevision: "fixture-aggregate-v1", memberStates };
  }), externalEnabled: controls.review.aggregateExternalEnabled ?? false,
  listenerReady: !!controls.review.aggregateExternalEnabled || !!state.router.enabled,
});
api.aggregation.state = async () => aggregationState();
api.aggregation.save = async entry => {
  if (controls.review.holdAggregateSave) {
    await new Promise<void>(resolve => { controls.review.completeAggregateSave = resolve; });
  }
  const saved = { ...structuredClone(entry), id: entry.id || "aggregate-" + ((controls.review.aggregateSaves ?? []).length + 1) };
  controls.review.aggregateSaves = [...(controls.review.aggregateSaves ?? []), saved];
  controls.review.aggregateModels = [...(controls.review.aggregateModels ?? []).filter((model: AggregateModel) => model.id !== saved.id), saved];
  return aggregationState();
};
api.aggregation.delete = async id => {
  controls.review.aggregateDeletes = [...(controls.review.aggregateDeletes ?? []), id];
  controls.review.aggregateModels = (controls.review.aggregateModels ?? []).filter((entry: AggregateModel) => entry.id !== id);
  return aggregationState();
};
api.aggregation.setExternalEnabled = async enabled => { controls.review.aggregateExternalEnabled = enabled; return aggregationState(); };
api.aggregation.selectCodex = async aggregateId => {
  controls.review.aggregateCodexSelections = [...(controls.review.aggregateCodexSelections ?? []), aggregateId];
  state = { ...state, router: { ...state.router, aggregateModelId: aggregateId, accessMode: "router" } };
  return { state: structuredClone(state), applied: null };
};
api.aggregation.exportConnection = async request => {
  controls.review.aggregateExports = [...(controls.review.aggregateExports ?? []), structuredClone(request)];
  const models = aggregationState().models.filter(entry => request.aggregateIds.includes(entry.id) && entry.available);
  const selected = models.find(entry => entry.id === request.selectedAggregateId);
  if (request.selectedAggregateId && !selected) throw "aggregation.errors.unavailable";
  const baseUrl = "http://127.0.0.1:43127/aggregate" + (request.consumerProfile === "anthropicSdk" ? "" : "/v1");
  const configText = JSON.stringify({ baseUrl, model: selected?.publicModelId ?? null,
    apiKey: request.revealCredential ? "synthetic-local-aggregate-token" : "<redacted>",
    protocol: request.ingressProtocol }, null, 2);
  return { configText, connection: { schemaVersion: 1, consumerProfile: { id: request.consumerProfile,
    ingressProtocol: request.ingressProtocol, modelsFormat: request.ingressProtocol === "anthropicMessages" ? "anthropic" : "openAi" },
    baseUrl, requestPath: request.ingressProtocol === "chatCompletions" ? "/chat/completions" : request.ingressProtocol === "anthropicMessages" ? "/v1/messages" : "/responses", modelsPath: "/v1/models",
    selectedModelId: selected?.publicModelId ?? null, modelCatalog: models.map(entry => ({ targetRef: { kind: "aggregate", aggregateId: entry.id },
      publicModelId: entry.publicModelId, label: entry.displayName, capabilities: {}, limits: {} })), authScheme: "bearer", localCredentialRef: "fixture-local",
    listenerReady: aggregationState().listenerReady, runtimeRequirement: "keepApplicationRunning", configRevision: "fixture-v1",
    catalogRevision: "fixture-catalog-v1", tokenVersion: "fixture-token-v1" } } as ConnectionExportResult;
};
api.routing.diagnostics = async () => {
  if (controls.review.routeDiagnostics) return controls.review.routeDiagnostics;
  const base = { id: "synthetic-route", timestamp: 1_790_740_800, requestedModel: "relay::claude-opus-5-5-kiro",
  requested: { "reasoning.effort": "high" }, stage: "complete", done: true, status: 200, failureStage: null, error: null, durationMs: 840, firstTokenMs: 230,
  attempts: [{ providerId: "relay", providerName: "Synthetic channel", upstreamModel: "claude-opus-5-5-kiro", matchedModel: "claude-opus-5-5", protocol: "anthropicMessages",
    sent: { "thinking.type": "adaptive", "output_config.effort": "high", max_tokens: 8192 }, status: 200, failureStage: null, error: null, durationMs: 830,
    catalogRevision: "abcdef1234567890", matchKind: "normalized", pricingStatus: "priced", pricingSource: "catalog",
    attemptId: "synthetic-attempt", selectionReason: "active_provider", retryReason: null,
    credential: { kind: "providerApiKey", status: "recorded", reason: null, versionFingerprint: "fixture-version", accountId: null, accountStatus: "not_recorded", label: null },
    modelObservation: { servedModel: "claude-opus-5-5-kiro", modelMatchStatus: "same", modelMatchBasis: "raw_exact", unknownReason: null, conflictingModel: null } }] };
  return controls.review.desktopDiagnostics ? [base, { ...base, id: "desktop-route", clientSource: "claudeDesktop",
    requestedModel: "claude-sonnet-4-6", resolvedModel: "deepseek-fixture", routeRevision: 3,
    attempts: base.attempts.map(attempt => ({ ...attempt, selectionReason: "desktop_role_mapping" })) }] : [base];
};
const capabilityRef = { providerId: "anthropic", modelId: "claude-opus-5-5" };
api.routing.capabilities = async () => ({ stale: true, error: "catalog_unavailable_or_invalid", updatedAt: 1_790_740_800,
  catalog: { schemaVersion: 1, sourceUrl: "https://models.dev/api.json", fetchedAt: 1_790_740_000, lastCheckedAt: 1_790_740_800,
    stale: true, origin: "runtime", revision: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890", providers: 225, models: 8339, error: null },
  models: [{
    providerId: "relay", providerName: "Synthetic channel", displayName: "My custom alias",
    upstreamModel: "claude-opus-5-5-kiro", matchedModel: "anthropic/claude-opus-5-5",
    matchKind: "normalized", matched: capabilityRef,
    steps: [{ modelId: "claude-opus-5-5", step: "knownChannelSuffix", matched: [capabilityRef], note: "matched 1 scoped catalog entry" }],
    conflicts: [{ field: "outputLimit", candidates: [{ modelRef: capabilityRef, value: "128000" }, { modelRef: { providerId: "github-copilot", modelId: "claude-opus-5-5" }, value: "64000" }] }],
    catalogHints: [{ source: capabilityRef, canonicalRaw: "anthropic/claude-opus-5-5", canonicalTarget: capabilityRef,
      canonicalStatus: "self", reasoningOptionsPresence: "declared",
      reasoningOptions: [{ kind: "effort", values: ["low", "high"], min: null, max: null }],
      interleaved: { field: "reasoning_content" } }],
    reasoning: { value: [{ effort: "low", description: "Low" }, { effort: "high", description: "High" }], source: "catalog", sourceRef: capabilityRef, updatedAt: "2025-11-24T00:00:00Z" },
    reasoningSupports: true,
    reasoningOptions: [{ kind: "effort", values: ["low", "high"], min: null, max: null }],
    defaultReasoning: "low", defaultReasoningSource: "policy",
    context: { value: 1_000_000, source: "local", sourceRef: null, updatedAt: null },
    inputLimit: { value: null, source: "unknown", sourceRef: null, updatedAt: null },
    output: { value: 128_000, source: "catalog", sourceRef: capabilityRef, updatedAt: "2025-11-24T00:00:00Z" },
    price: { currency: "USD", unit: "perMillionTokens", input: 4, output: 20, cacheRead: 0.4, cacheWrite: 5, reasoning: null,
      inputAudio: null, outputAudio: null, contextOver200k: null, tiers: [],
      source: "catalog", sourceRef: capabilityRef, updatedAt: "2025-11-24T00:00:00Z", complete: true, note: null },
    projection: { contextWindow: 1_000_000, contextSource: "local", contextIsFallback: false, budget: 1_000_000, budgetSource: "local",
      inputLimit: null, outputLimit: 128_000, outputSource: "catalog",
      reasoning: [{ effort: "low", description: "Low" }, { effort: "high", description: "High" }], reasoningSource: "catalog", defaultReasoning: "low",
      compression: { mode: "percent", percent: 90, tokens: null, source: "global", budget: 1_000_000, budgetSource: "local", threshold: 900_000, state: "computed", note: null } },
    compressionApplied: true, compressionAppliedSource: "catalog",
    compressionAppliedThreshold: 900_000, compressionSharedThreshold: null, compressionLimitingModels: [],
    compressionScope: "routerModel", compressionSharedApplied: null,
    compressionSharedPlan: null, compressionConfigActive: true, compressionWrittenThreshold: 900_000,
  }].flatMap(model => controls.review.capabilityVariants?.map((patch: any) => ({ ...model, ...patch })) ?? [model]) });
api.routing.catalogStatus = async () => ({ schemaVersion: 1, sourceUrl: "https://models.dev/api.json", fetchedAt: 1_790_740_000,
  lastCheckedAt: 1_790_740_800, stale: true, origin: "runtime", revision: "abcdef", providers: 225, models: 8339, error: null }) as any;
api.routing.refreshCatalog = async () => ({ schemaVersion: 1, sourceUrl: "https://models.dev/api.json", fetchedAt: 1_790_740_800,
  lastCheckedAt: 1_790_740_800, stale: false, origin: "runtime", revision: "abcdef", providers: 225, models: 8339, error: null }) as any;
api.routing.setModelPrice = async (providerId, modelId, price) => {
  controls.review.priceSaves = [...(controls.review.priceSaves ?? []), { providerId, modelId, price }];
  return { state, applied: null } as any;
};
api.routing.setWebSearchProvider = async providerId => { state = { ...state, router: { ...state.router, webSearchProviderId: providerId } }; return { state, applied: null }; };
api.loadSnapshot = async () => ({ data: { status: {
  paths: { codexHome: "C:/synthetic/codex", authExists: true, registryExists: true, sessionsExists: true },
  api: { proxy: { mode: "manual", url: "http://127.0.0.1:7890" } },
  autoSwitch: { supported: true, enabled: false, threshold5hPercent: 15, thresholdWeeklyPercent: 10 },
} } }) as any;
const mcpServers = [
  { name: "filesystem", transport: "stdio", enabled: true, sourcePath: "C:/synthetic/.codex/config.toml", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "C:/work"], url: null, headers: {}, environment: { LOG_LEVEL: "info" } },
  { name: "docs", transport: "http", enabled: true, sourcePath: "C:/synthetic/.codex/config.toml", command: null, args: [], url: "https://docs.example.invalid/mcp", headers: { Authorization: "Bearer synthetic" }, environment: {} },
  { name: "legacy-sse", transport: "sse", enabled: false, sourcePath: "C:/synthetic/.codex/config.toml", command: null, args: [], url: "https://sse.example.invalid/events", headers: {}, environment: {} },
  ...Array.from({ length: 3 }, (_, i) => ({ name: "tool-" + i, transport: "stdio", enabled: i !== 1, sourcePath: "C:/synthetic/.codex/config.toml", command: "uvx", args: ["tool-" + i], url: null, headers: {}, environment: {} })),
];
api.loadMcpServers = async () => ({ schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
  data: { items: mcpServers, total: mcpServers.length, sourcePath: "C:/synthetic/.codex/config.toml", lastScanAt: 0 } }) as any;
const quotaWindow = (remainingPercent: number, windowMinutes: number) => ({ usedPercent: 100 - remainingPercent, remainingPercent, windowMinutes, resetsAt: 1_900_000_000 });
const accountList = { activeAccountKey: "acct-a", refreshedAt: 1_800_000_000, items: [
  { accountKey: "acct-a", email: "alpha@example.invalid", displayName: "Alpha", plan: "pro", authMode: "chatgpt", isActive: true, createdAt: 1_700_000_000, lastUsedAt: 1_800_000_000, lastUsageAt: 1_800_000_000, primaryWindow: quotaWindow(72, 300), secondaryWindow: quotaWindow(40, 10080), usageStatus: "healthy", usageError: null },
  { accountKey: "acct-b", email: "beta@example.invalid", displayName: "Beta", plan: "plus", authMode: "chatgpt", isActive: false, createdAt: 1_700_000_000, lastUsedAt: null, lastUsageAt: 1_800_000_000, primaryWindow: quotaWindow(8, 300), secondaryWindow: quotaWindow(90, 10080), usageStatus: "healthy", usageError: null },
  { accountKey: "acct-c", email: "gamma@example.invalid", displayName: "Gamma", plan: "team", authMode: "chatgpt", isActive: false, createdAt: 1_700_000_000, lastUsedAt: null, lastUsageAt: null, primaryWindow: null, secondaryWindow: null, usageStatus: "expired", usageError: "token expired" },
] };
api.accounts.refresh = async () => accountList as any;
api.accounts.list = async () => accountList as any;
api.usage.overview = async () => ({ ...usageFixture, relay: { ...usageFixture.relay, ...controls.review.relayPatch } });
api.loadInstalledSkills = async (source = "codex") => {
  (controls.review.skillsReadCalls ??= []).push(source);
  const items = source === "claudeCode" ? controls.review.claudeSkills : controls.review.skillItems;
  return { warnings: source === "codex" ? controls.review.skillWarnings ?? [] : [], data: { total: items?.length ?? (source === "codex" ? 36 : 0),
    items: items ?? [], rootPath: `C:/synthetic/.${source === "claudeCode" ? "claude" : "codex"}/skills` } } as any;
};
api.setSkillEnabled = async (id, enabled, source = "codex") => {
  controls.review.skillToggleCalls.push({ id, enabled, source });
  if (controls.review.skillToggleHold) await new Promise(resolve => { controls.review.skillToggleResolve = resolve; });
  if (controls.review.skillToggleFailure) throw new Error("Synthetic skill permission failure");
  const skill = (source === "claudeCode" ? controls.review.claudeSkills : controls.review.skillItems).find((item: any) => item.id === id);
  Object.assign(skill, { enabled, directoryPath: enabled ? `C:/synthetic/.${source === "claudeCode" ? "claude" : "codex"}/skills/sample` : `C:/synthetic/app/${source === "claudeCode" ? "skill-clients/claude-code/" : ""}disabled-skills/sample` });
  return { data: skill } as any;
};
api.resolveSkillConflict = async (id, keepEnabled, source = "codex") => {
  const skill = (source === "claudeCode" ? controls.review.claudeSkills : controls.review.skillItems).find((item: any) => item.id === id);
  Object.assign(skill, { enabled: keepEnabled, conflict: false });
  controls.review.skillConflictChoice = { id, keepEnabled, source };
  return { data: skill } as any;
};
api.loadSkillBackups = async (source = "codex") => {
  (controls.review.backupsReadCalls ??= []).push(source);
  const items = source === "claudeCode" ? controls.review.claudeSkillBackups ?? [] : [];
  return { data: { total: items.length, items, rootPath: source === "claudeCode" ? "C:/synthetic/app/skill-clients/claude-code/skill-backups" : "C:/synthetic/app/skill-backups" } } as any;
};
api.restoreSkillBackup = async (id, source = "codex") => { (controls.review.skillRestoreCalls ??= []).push({ id, source }); return { data: { restored: 1 } } as any; };
api.deleteSkillBackup = async (id, source = "codex") => { (controls.review.skillBackupDeleteCalls ??= []).push({ id, source }); return { data: { deleted: 1 } } as any; };
api.removeSkill = async (id, source = "codex") => { (controls.review.skillRemoveCalls ??= []).push({ id, source }); return { data: { removed: 1 } } as any; };
api.loadCustomInstructionState = async () => ({ data: { current: { managedBlockPresent: false, lastTemplateTitle: null } } }) as any;
api.getSystemInfo = async () => ({ os: "Windows", arch: "x64" }) as any;
api.providers.queryBalance = id => new Promise((resolve, reject) => {
  controls.review.balanceRejectors.push(reject);
  controls.review.balanceRequests.push(id);
  controls.review.balanceResolvers.push(() => resolve({ ...(state.providers.find(provider => provider.id === id) ?? mixedProvider), id, balance: { remaining: 9, used: null, unit: "USD", rawQuota: 9,
    quotaPerUnit: null, requestCount: null, checkedAt: Math.floor(Date.now() / 1000), conversionAvailable: true }, balanceSource: "newapi" }));
});
api.providers.saveBalanceCredentials = async (id, expectedBaseUrl, userId, accessToken) => {
  controls.review.balanceCredentialSaves.push({ id, expectedBaseUrl, userId, accessToken });
  const current = state.providers.find(provider => provider.id === id)!;
  const updated = { ...current, userId, hasAccessToken: current.hasAccessToken || !!accessToken, balance: null };
  state = { ...state, providers: state.providers.map(provider => provider.id === id ? updated : provider) };
  return updated;
};
api.providers.setBalanceSharing = async (id, _expectedBaseUrl, shared) => {
  const current = state.providers.find(provider => provider.id === id)!;
  const updated = { ...current, balanceSharedFrom: shared ? "group-a" : null };
  state = { ...state, providers: state.providers.map(provider => provider.id === id ? updated : provider) };
  return updated;
};
api.providers.setBalanceRefreshInterval = async (id, expectedBaseUrl, intervalMinutes) => {
  controls.review.balanceIntervalSaves = [...(controls.review.balanceIntervalSaves ?? []), { id, expectedBaseUrl, intervalMinutes }];
  if (controls.review.failBalanceIntervalSave) throw "newapi.errors.storage";
  const current = state.providers.find(provider => provider.id === id)!;
  const ownerId = current.balanceSharedFrom ?? id;
  state = { ...state, providers: state.providers.map(provider => provider.id === ownerId || provider.balanceSharedFrom === ownerId
    ? { ...provider, balanceRefreshIntervalMinutes: intervalMinutes } : provider) };
  return state.providers.find(provider => provider.id === id)!;
};
api.providers.onBalanceUpdated = async callback => {
  const listeners = controls.review.balanceUpdateListeners ??= new Set<() => void>();
  listeners.add(callback);
  return () => listeners.delete(callback);
};
api.routing.setAccessMode = async mode => {
  controls.review.accessCalls.push(mode);
  state = { ...state, router: { ...state.router, accessMode: mode } };
  return { state, applied: null };
};
api.routing.setEnabled = async enabled => {
  state = { ...state, router: { ...state.router, enabled } };
  return { state, applied: null };
};
api.routing.setAccountless = async accountless => {
  state = { ...state, router: { ...state.router, accountless } };
  return { state, applied: null };
};
let subagentSettings: CodexSubagentSettings | null = null;
api.routing.subagents = async () => {
  if (controls.review.failSubagentLoad) throw "newapi.errors.configRead";
  const efforts = ["low", "medium", "high", "xhigh"];
  const models = controls.review.subagentModels ?? (state.router.aggregateModelId
    ? [{ id: "aggregate-luna", label: "Luna Pool", reasoningEfforts: efforts }]
    : state.router.accessMode === "native"
      ? [{ id: "responses-model", label: "responses-model", reasoningEfforts: efforts }]
      : [{ id: "gpt-6-luna", label: "Official Luna", reasoningEfforts: efforts },
        { id: "mixed::responses-model", label: "Relay / responses-model", reasoningEfforts: efforts }]);
  return { settings: structuredClone(subagentSettings ?? controls.review.originalSubagents ?? { model: null, reasoningEffort: null }),
    managed: subagentSettings !== null, applied: state.router.enabled && subagentSettings !== null,
    models, inheritedReasoningEfforts: efforts };
};
api.routing.saveSubagents = async settings => {
  controls.review.subagentSaves = [...(controls.review.subagentSaves ?? []), structuredClone(settings)];
  if (controls.review.holdSubagentSave) await new Promise<void>(resolve => { controls.review.releaseSubagentSave = resolve; });
  if (controls.review.failSubagentSave) throw "newapi.errors.configWrite";
  subagentSettings = structuredClone(settings);
  return { state: structuredClone(state), applied: state.router.enabled
    ? { path: "C:/synthetic/config.toml", backupPath: null } : null };
};
api.targets.restart = async target => {
  controls.review.restartCalls.push(target);
};
api.targets.configs = async () => [];
api.targets.previewConfig = async request => {
  controls.review.previewed = request;
  return { path: "C:/synthetic/config.json", revision: "mock-revision", content: JSON.stringify({ model: request.model }) };
};
api.providers.test = async (id, selectedModel, onProgress) => {
  controls.review.tested = { id, model: selectedModel };
  onProgress?.({ stage: "request", elapsedMs: 0, status: null, text: "POST /v1/messages\nHost: synthetic.invalid\nAccept: text/event-stream" });
  onProgress?.({ stage: "headers", elapsedMs: 120, status: 200, text: "" });
  await new Promise<void>((resolve, reject) => {
    controls.review.finishProbe = () => {
      onProgress?.({ stage: "firstByte", elapsedMs: 320, status: null, text: "" });
      onProgress?.({ stage: "content", elapsedMs: 330, status: null, text: "Hello from the streaming model." });
      onProgress?.({ stage: "complete", elapsedMs: 380, status: null, text: "" });
      resolve();
    };
    controls.review.failProbe = () => { onProgress?.({ stage: "failed", elapsedMs: 900, status: null, text: "newapi.errors.timeout" }); reject("newapi.errors.timeout"); };
  });
  return { ...mixedProvider, testStatus: "ok" };
};
api.providers.saveBalanceScript = async (id, expectedBaseUrl, script) => {
  controls.review.savedBalanceScript = { id, expectedBaseUrl, script };
  const current = state.providers.find(item => item.id === id)!;
  const next = { ...current, balanceScript: script, balance: null, balanceSource: null };
  state = { ...state, providers: state.providers.map(item => item.id === id ? next : item) };
  return next;
};
api.image.keyStatus = async () => false;
api.image.keySites = async () => [];
api.image.generate = (request, jobId) => new Promise((resolve, reject) => {
  controls.review.imageRequests.push(request);
  controls.review.imageResolvers.push((images = [pixel]) => resolve({ images, model: request.model, revisedPrompts: [] }));
  if (jobId) controls.review.imageCancels[jobId] = () => reject("image.errors.cancelled");
});
api.image.cancelGeneration = async jobId => {
  controls.review.cancelledJobs.push(jobId);
  controls.review.imageCancels[jobId]?.();
  return true;
};
api.image.saveToGallery = async () => {
  controls.review.gallerySaves += 1;
  if (controls.review.failGallerySave) throw new Error("image.errors.save");
  return { directory: "C:/synthetic", files: ["C:/synthetic/generated.png"] };
};
api.image.listGallery = async () => ({ directory: "C:/synthetic", items: galleryItems });
api.openPath = async path => {
  controls.review.openedPath = path;
  if (controls.review.failOpenPath) throw new Error("synthetic open path failure");
};
api.image.loadGalleryThumbnail = path => {
  controls.review.thumbnailCalls.push(path);
  return controls.review.holdThumbnails
    ? new Promise(resolve => controls.review.thumbnailResolvers.push(() => resolve(pixel)))
    : Promise.resolve(pixel);
};
api.image.loadGalleryImage = path => {
  controls.review.originalCalls.push(path);
  return controls.review.holdOriginal
    ? new Promise(resolve => { controls.review.resolveOriginal = () => resolve(pixel); })
    : Promise.resolve(pixel);
};
api.providers.fetchModels = () => new Promise(resolve => {
  controls.review.resolveModels = (fetched = [model("model-A-returned")]) => resolve(fetched);
});
// Deliberately synthetic resolver output: this fixture verifies rendering and
// async isolation, while Rust tests verify model matching and precedence.
const modelResolutionFixture = (draft: any) => ({ models: draft.models.map((item: any) => {
  const known = ["model-A", "model-B", "newly-fetched-model"].includes(item.id);
  const ref = known ? { providerId: "fixture-official", modelId: item.id } : null;
  const field = (value: number | null) => ({ value, source: known ? "catalog" : "unknown", sourceRef: ref, updatedAt: null });
  const result = {
    modelId: item.id, matchKind: known ? "exact" : "unmatched", matched: ref,
    context: field(known ? 1_000_000 : null), inputLimit: field(known ? 800_000 : null), output: field(known ? 65_000 : null),
    projection: { contextWindow: known ? 1_000_000 : 200_000, contextSource: known ? "catalog" : "fallback", contextIsFallback: !known,
      budget: known ? 800_000 : 200_000, budgetSource: known ? "catalog" : "fallback", inputLimit: known ? 800_000 : null,
      outputLimit: known ? 65_000 : 8_192, outputSource: known ? "catalog" : "fallback", reasoning: [], reasoningSource: "unknown", defaultReasoning: null,
      compression: { mode: "percent", percent: 90, tokens: null, source: "global", budget: known ? 800_000 : 200_000,
        budgetSource: known ? "catalog" : "fallback", threshold: known ? 720_000 : 180_000, state: "computed", note: null } },
  };
  const patch = controls.review.modelResolutionPatches?.[item.id] ?? {};
  return { ...result, ...patch, projection: { ...result.projection, ...patch.projection,
    compression: { ...result.projection.compression, ...patch.projection?.compression } } };
}) });
api.providers.previewModelResolutions = draft => {
  const snapshot = JSON.parse(JSON.stringify(draft));
  controls.review.modelResolutionRequests.push(snapshot);
  const response = modelResolutionFixture(snapshot);
  return new Promise((resolve, reject) => {
    controls.review.modelResolutionResolvers.push((replacement = response) => resolve(replacement));
    controls.review.modelResolutionRejectors.push(reject);
    if (!controls.review.holdModelResolutions) resolve(response);
  });
};
api.providers.revealApiKey = id => {
  controls.review.revealedIds = [...(controls.review.revealedIds ?? []), id];
  return new Promise((resolve, reject) => {
    controls.review.resolveApiKey = () => resolve(`synthetic-key-${id}`);
    controls.review.rejectApiKey = () => reject('newapi.errors.apiKeyMissing');
  });
};
api.providers.save = async input => {
  controls.review.saved = input;
  return { state, applied: controls.review.savedApplied ?? null };
};

function ProviderFixture({ mixed = false }: { mixed?: boolean }) {
  const [dialog, setDialog] = useState({ open: true, provider: controls.review.dialogProviderFixture ?? (mixed ? mixedProvider : provider("A")) });
  return <><button id="open-b" onClick={() => setDialog({ open: true, provider: provider("B") })}>Open B</button>
    <button id="reopen-saved" onClick={() => setDialog(current => ({ open: true, provider: { ...current.provider, ...controls.review.saved } }))}>Reopen saved</button>
    <ProviderDialog open={dialog.open} provider={dialog.provider}
      onOpenChange={open => setDialog(current => ({ ...current, open }))} onSaved={noop} /></>;
}

type FixtureMode = "skills" | "desktop-updater" | "desktop-exit" | "terminal-proxy" | "thread-details" | "thread-details-missing" | "thread-details-unknown" | "thread-details-error" | "compact" | "grid" | "image" | "mcp" | "accounts" | "account-resets" | "account-resets-unavailable" | "thread-tree" | "groups" | "overview" | "usage" | "provider" | "warnings" | "thread-actions" | "thread-queue-recovery" | "mixed" | "access" | "incompatible" | "native-invalid" | "test" | "clients" | "clients-existing" | "clients-conflict" | "clients-opencode" | "clients-opencode-stale" | "clients-versions" | "clients-versions-existing" | "recovery-missing" | "recovery-journal" | "recovery-official" | "recovery-long" | "recovery-router";

function RecoveryFixture({ thread }: { thread: ThreadSummary }) {
  const [open, setOpen] = useState(true);
  return <ThreadRecoveryDialog thread={open ? thread : null} onOpenChange={setOpen}
    onRecovered={() => {
      controls.review.recoveredCalls += 1;
      if (controls.review.recoveryRefreshError) throw new Error("Synthetic refresh failed");
    }} />;
}

function recoveryPreview(mode: FixtureMode) {
  const base = {
    threadId: "t0", revision: "recovery-revision", configuredProvider: "agentharbor_native",
    targetProvider: "agentharbor_native", databaseProvider: "openai", rolloutProvider: "openai",
    rolloutPath: "C:/synthetic/rollout-t0.jsonl", catalogAnalysis: null, catalogFields: [],
    coordination: null, status: "providerMismatch", canApply: true,
    reason: "Synthetic recovery preview", changes: [{ target: "database", path: "C:/synthetic/state.sqlite",
      fromProvider: "openai", toProvider: "agentharbor_native" }],
  };
  if (mode === "recovery-router") return { ...base,
    configuredProvider: "agentharbor_relay", targetProvider: "agentharbor_relay",
    changes: base.changes.map(change => ({ ...change, toProvider: "agentharbor_relay" })) };
  if (mode === "recovery-long") return { ...base, changes: [base.changes[0], {
    target: "rollout", path: "C:/synthetic/sessions/2026/09/30/" + "long-workspace-name-".repeat(30) + ".jsonl",
    fromProvider: "openai", toProvider: "agentharbor_native",
  }] };
  if (mode === "recovery-missing") return { ...base, databaseProvider: null, status: "missingCatalog",
    catalogAnalysis: { schemaSignature: "fixture-schema", ready: true, missingFields: [], unsupportedColumns: [],
      relation: "topLevel", mappingErrors: [] },
    catalogFields: [{ name: "id", value: "t0" }, { name: "model_provider", value: "agentharbor_native" },
      { name: "rollout_path", value: "C:/synthetic/rollout-t0.jsonl" }] };
  if (mode === "recovery-journal") return { ...base, status: "journalBlocked", canApply: false, changes: [],
    coordination: { journalPath: "C:/synthetic/recovery/thread-recovery-journal.json",
      directoryPath: "C:/synthetic/recovery", backupPath: "C:/synthetic/recovery/thread.backup.json",
      detail: "Unsupported journal version" } };
  return { ...base, configuredProvider: "openai", targetProvider: "openai", databaseProvider: "openai",
    rolloutProvider: "openai", status: "officialAccountUnverified", canApply: false, changes: [],
    reason: "Local metadata cannot identify the ChatGPT account that owns the server thread" };
}

let mountSequence = 0;
function mount(mode: FixtureMode) {
  localStorage.removeItem("threads_source");
  if (mode === "overview") { controls.review.claudeSkills = [{ id: "one" }, { id: "two" }, { id: "three" }]; controls.review.claudeSkillBackups = [{ id: "claude-b1" }, { id: "claude-b2" }]; }
  controls.review.conversationCalls = [];
  controls.review.conversationListResolvers = [];
  if (mode === "skills") {
    controls.review.claudeSkills = [
      { id: "sample", name: "sample", title: "Claude Sample Skill", summary: "Claude user skill", relativePath: "sample",
        directoryPath: "C:/synthetic/.claude/skills/sample", skillFilePath: "C:/synthetic/.claude/skills/sample/SKILL.md", enabled: true },
      { id: "linked", name: "linked", title: "Linked Skill", summary: "cc-switch managed", relativePath: "linked",
        directoryPath: "C:/synthetic/.claude/skills/linked", skillFilePath: "C:/synthetic/.claude/skills/linked/SKILL.md", enabled: true, readOnly: true }
    ];
    controls.review.claudeSkillBackups = [{ id: "claude-backup", name: "sample", title: "Claude Backup", relativePath: "sample", directoryPath: "C:/synthetic/app/skill-clients/claude-code/skill-backups/claude-backup", reason: "removed", createdAt: 0 }];
    controls.review.skillToggleCalls = [];
    controls.review.skillConflictChoice = null;
    controls.review.skillWarnings = [{ code: "skills.warnings.invalidEncoding", message: "C:/synthetic/.codex/skills/bad-encoding" }];
    controls.review.skillItems = [
      { id: "sample", name: "sample", title: "Sample Skill", summary: "Synthetic skill", relativePath: "sample",
        directoryPath: "C:/synthetic/.codex/skills/sample", skillFilePath: "C:/synthetic/.codex/skills/sample/SKILL.md", updatedAt: 0, enabled: true },
      { id: "conflict", name: "conflict", title: "Conflict Skill", summary: null, relativePath: "conflict",
        directoryPath: "C:/synthetic/.codex/skills/conflict", conflictDirectoryPath: "C:/synthetic/app/disabled-skills/conflict", skillFilePath: "C:/synthetic/.codex/skills/conflict/SKILL.md", updatedAt: 0, enabled: true, conflict: true },
    ];
  }
  if (mode === "terminal-proxy") {
    controls.review.terminalSetCalls = [];
    controls.review.terminalTestCalls = [];
    controls.review.terminalTestResolvers = [];
    controls.review.terminalOpenCalls = 0;
    controls.review.terminalStatus = { supported: true, enabled: false, applied: false,
      url: null, noProxy: "localhost,127.0.0.1,::1", recoveryRequired: false, conflicts: [],
      preservedVariables: [], notificationSent: true, ...controls.review.terminalStatusPatch };
    api.terminalProxy.load = async () => {
      if (controls.review.terminalLoadFailure) throw "settings.terminalProxy.errors.environment";
      return controls.review.terminalStatus;
    };
    api.terminalProxy.set = async (enabled, url, noProxy) => {
      controls.review.terminalSetCalls.push({ enabled, url, noProxy });
      if (controls.review.terminalSaveHold) await new Promise(resolve => { controls.review.terminalSaveResolver = resolve; });
      if (controls.review.terminalSaveFailure) throw controls.review.terminalSaveFailure;
      const result = { ...controls.review.terminalStatus, enabled, applied: enabled, url: enabled ? url : controls.review.terminalStatus.url,
        noProxy, recoveryRequired: false, conflicts: [], ...controls.review.terminalSetResultPatch };
      controls.review.terminalStatus = result;
      return result;
    };
    api.terminalProxy.test = async url => {
      controls.review.terminalTestCalls.push(url);
      const probes = [{ service: "Anthropic", reachable: true, statusCode: 404 },
        { service: "OpenAI", reachable: true, statusCode: 401 }];
      if (controls.review.terminalTestHold) return new Promise(resolve => controls.review.terminalTestResolvers.push(resolve));
      if (controls.review.terminalTestFailure) throw controls.review.terminalTestFailure;
      return probes;
    };
    api.terminalProxy.openTerminal = async () => {
      controls.review.terminalOpenCalls++;
      if (controls.review.terminalOpenFailure) throw "settings.terminalProxy.errors.launch";
    };
  }
  controls.review.loginCalls = [];
  controls.review.cancelCalls = [];
  controls.review.cancelFailures = 0;
  controls.review.cancelResult = true;
  controls.review.cancelHold = false;
  api.accounts.login = attemptId => new Promise((resolve, reject) => {
    controls.review.loginCalls.push({ attemptId, resolve: () => resolve(accountList as any), reject });
  });
  api.accounts.cancelLogin = async attemptId => {
    controls.review.cancelCalls.push(attemptId);
    if (controls.review.cancelFailures > 0) {
      controls.review.cancelFailures--;
      throw "accounts.cancelFailed";
    }
    if (controls.review.cancelHold) await new Promise(resolve => { controls.review.releaseCancel = resolve; });
    return controls.review.cancelResult;
  };
  if (mode.startsWith("account-resets")) {
    const now = Math.floor(Date.now() / 1000);
    const credits = [
      { status: "available", grantedAt: now - 86400, expiresAt: now + 23 * 86400 },
      { status: "available", grantedAt: now, expiresAt: now + 30 * 86400 },
      { status: "redeemed", grantedAt: now - 7 * 86400, expiresAt: null },
      { status: "expired", grantedAt: now - 40 * 86400, expiresAt: now - 10 * 86400 },
    ];
    const payload = { ...accountList, items: accountList.items.map((account, index) => ({ ...account,
      lastUsageAt: now, resetCredits: index === 0 ? { availableCount: 2, credits: mode.endsWith("unavailable") ? null : credits }
        : index === 1 ? { availableCount: 0, credits: [credits[2]] } : null,
    })) };
    api.accounts.list = async () => payload as any;
    api.accounts.refresh = async () => { controls.review.accountRefreshCalls = (controls.review.accountRefreshCalls ?? 0) + 1; return payload as any; };
  } else {
    api.accounts.list = async () => accountList as any;
    api.accounts.refresh = async () => (controls.review.emptyAccounts ? { ...accountList, items: [] } : accountList) as any;
  }
  const selectedProvider = mode.startsWith("clients-versions") ? versionedProvider
    : mode === "incompatible" || mode === "native-invalid" ? { ...mixedProvider, model: "chat-model" } : mixedProvider;
  state = { providers: mode === "image" ? [imageProvider] : mode === "groups" ? groupedProviders : mode === "overview" ? [...groupedProviders, provider("extra-1"), provider("extra-2")] : mode === "usage" ? [{ ...groupedProviders[0], balance: { remaining: 12.5, used: 7.5, unit: "USD", rawQuota: 12.5, quotaPerUnit: null, requestCount: 5, checkedAt: Math.floor(Date.now() / 1000), conversionAvailable: true } }] : [selectedProvider], router: { enabled: false, accessMode: mode === "native-invalid" ? "native" : "router", accountless: false, port: 43127, configDrifted: false }, proxyRunning: false, activeProviderId: mode === "image" || mode === "groups" || mode === "overview" || mode === "usage" ? null : selectedProvider.id };
  if (controls.review.providerFixtures) state.providers = controls.review.providerFixtures;
  if (controls.review.routerPatch) state.router = { ...state.router, ...controls.review.routerPatch };
  if (mode === "incompatible") state.providers.push(provider("alternate"));
  const savedClient: ClientConfigRequest | null = mode === "clients-opencode-stale" ? {
    targetId: "opencode", providerId: "mixed", model: "responses-model", contextWindow: 128000,
    maxOutputTokens: 8192, accessMode: "router", providerIds: ["mixed"],
    providerModelIds: { mixed: ["responses-model", "removed-model"] },
  } : mode === "clients-versions-existing" ? {
    targetId: "claudeCode", providerId: "versions", model: "claude-opus-4-8", contextWindow: 200000,
    maxOutputTokens: 8192, accessMode: "native", providerIds: ["versions"], authField: "authToken",
    claudeModels: { sonnet: "", opus: "claude-opus-4-8", fable: "", haiku: "", subagent: "" },
    claudeModelIds: ["claude-opus-5-5", "claude-opus-4-8"],
  } : (mode === "clients-existing" || mode === "clients-conflict") ? {
    targetId: "claudeCode", providerId: "mixed", model: "responses-model", contextWindow: 128000,
    maxOutputTokens: 8192, accessMode: "router", providerIds: ["mixed"], authField: "apiKey",
    claudeModels: { sonnet: "chat-model", opus: "", fable: "", haiku: "", subagent: "" },
  } : null;
  const openCode = mode.startsWith("clients-opencode");
  const clientStatuses: ClientStatus[] = controls.review.clientStatusFixtures ?? (mode.startsWith("clients") ? [{
    targetId: openCode ? "opencode" : "claudeCode", name: openCode ? "OpenCode" : "Claude Code",
    path: "C:/synthetic/config.json", protocols: openCode ? ["responses", "chatCompletions", "anthropicMessages"] : ["anthropicMessages"],
    configured: !!savedClient, conflict: mode === "clients-conflict", conflictFields: mode === "clients-conflict" ? ["model"] : [], error: null, providerId: savedClient?.providerId ?? null,
    model: savedClient?.model ?? null, request: savedClient,
  }] : []);
  api.targets.configs = async () => structuredClone(clientStatuses);
  controls.review.releasedClientCount = 0;
  api.targets.previewConfig = async (request, replaceConflicts = false) => {
    if (clientStatuses[0]?.conflict && !replaceConflicts) throw "clients.errors.conflict";
    controls.review.previewed = request;
    controls.review.previewReplacesConflicts = replaceConflicts;
    return { path: "C:/synthetic/config.json", revision: "mock-revision", content: JSON.stringify({ model: request.model }) };
  };
  api.targets.applyConfig = async (request, revision, replaceConflicts = false) => {
    if (clientStatuses[0]?.conflict && !replaceConflicts) throw "clients.errors.conflict";
    controls.review.appliedClient = { request, revision, replaceConflicts };
    Object.assign(clientStatuses[0], { configured: true, conflict: false, conflictFields: [], providerId: request.providerId,
      model: request.model, request: JSON.parse(JSON.stringify(request)) });
    return { path: "C:/synthetic/config.json", backupPath: "C:/synthetic/config.backup" };
  };
  api.targets.restoreConfig = async (_targetId, replaceConflicts = false) => {
    if (clientStatuses[0].conflict && !replaceConflicts) throw "clients.errors.conflict";
    controls.review.restoredClient = { replaceConflicts };
    Object.assign(clientStatuses[0], { configured: false, conflict: false, conflictFields: [], providerId: null, model: null, request: null });
    return { path: "C:/synthetic/config.json", backupPath: "C:/synthetic/config.backup" };
  };
  api.targets.reapplyConfig = async (_targetId, rebaseline) => {
    if (clientStatuses[0].conflict && !rebaseline) throw "clients.errors.conflict";
    controls.review.reappliedClient = { rebaseline };
    Object.assign(clientStatuses[0], { conflict: false, conflictFields: [] });
    return { path: "C:/synthetic/config.json", backupPath: "C:/synthetic/config.backup" };
  };
  api.targets.releaseConfig = async () => { controls.review.releasedClientCount += 1; };
  const threads = Array.from({ length: 100 }, (_, i) => ({ key: "t" + i, id: "t" + i,
    cwd: "C:/synthetic", folderName: "synthetic", title: "Review thread " + i,
    createdAt: 0, updatedAt: 0, userMessages: 1, agentMessages: 1, fileSize: 100,
    archived: false, source: "", cliVersion: "", filePath: "C:/synthetic/t" + i,
    modelProvider: "agentharbor_native", model: "gpt-6-sol", tokensUsed: 10,
    inputTokens: 4, cachedTokens: 3, outputTokens: 6,
    threadSource: "sqlite", rolloutStatus: "available" }));
  if (mode === "recovery-long") threads[0].title = (
    "恢复已有会话：检查模型切换与长路径 C:/synthetic/" + "unbroken-title".repeat(30) + "。"
  ).repeat(3);
  if (mode === "thread-tree") {
    threads.forEach((thread, index) => {
      const project = index < 90 ? "C:/work/AiMaMi" : index < 95 ? "D:/work/AiMaMi" : index < 99 ? "C:/research" : "";
      Object.assign(thread, { cwd: project, folderName: project.split("/").at(-1) ?? "", updatedAt: 100 - index, archived: index % 2 === 0 });
    });
    controls.review.threadItems = threads;
  }
  // Router-adopted threads are the default; thread-actions has one direct thread.
  controls.review.batchCalls = [];
  controls.review.routedThreads = false;
  api.conversations.routeOverview = async () => ({
    routerActive: mode === "thread-actions" || controls.review.routedThreads,
    states: mode === "thread-actions" ? { t0: "direct", t1: "routed" }
      : controls.review.routedThreads ? { t0: "routed" } : {},
  });
  api.conversations.previewBatch = async (target, selection) => {
    controls.review.batchCalls.push({ operation: "preview", target, selection });
    return { targetProvider: "agentharbor_relay", providers: { openai: 1 }, skipped: [],
      ready: [{ threadId: "t0", revision: "batch-revision", fromProvider: "openai" }] };
  };
  api.conversations.applyBatch = async (target, candidates) => {
    controls.review.batchCalls.push({ operation: "apply", target, candidates });
    return { changed: candidates.length, unchanged: 0, failed: [], reopenError: null };
  };
  if (mode === "thread-actions" || mode === "thread-queue-recovery") {
    controls.review.recoveryApplyCalls = 0;
    controls.review.deleteCalls = [];
    controls.review.queueDeletes = false;
    controls.review.pendingDeletionKeys = [];
    controls.review.pendingDeletionRecoveryNeeded = mode === "thread-queue-recovery";
    api.conversations.previewRecovery = async threadId => ({ schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
      data: { ...recoveryPreview("recovery-missing"), threadId, status: threadId === "t0" ? "providerMismatch" : "compatible",
        canApply: threadId === "t0", catalogAnalysis: null, catalogFields: [] } as any });
    api.conversations.applyRecovery = async () => {
      controls.review.recoveryApplyCalls += 1;
      const preview = recoveryPreview("recovery-missing") as any;
      return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
        data: { changed: true, backupPath: "C:/synthetic/recovery/backup.json", preview } };
    };
    api.conversations.delete = async (_target, keys) => {
      controls.review.deleteCalls.push(keys);
      if (controls.review.queueDeletes) controls.review.pendingDeletionKeys.push(...keys);
      return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
        data: { deleted: controls.review.queueDeletes ? 0 : keys.length, queued: controls.review.queueDeletes ? keys.length : 0,
          catalogRowsDeleted: controls.review.queueDeletes ? 0 : keys.length, rolloutFilesDeleted: controls.review.queueDeletes ? 0 : keys.length,
          backupPath: "C:/synthetic/thread-delete-backups/fixture" } };
    };
    api.conversations.cancelPendingDeletions = async () => {
      const count = controls.review.pendingDeletionKeys.length;
      controls.review.pendingDeletionKeys = [];
      controls.review.pendingDeletionRecoveryNeeded = false;
      return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [], data: count };
    };
  } else if (mode.startsWith("recovery-")) {
    api.conversations.previewRecovery = async () => ({ schemaVersion: 1, success: true, code: "ok", message: "", warnings: [], data: recoveryPreview(mode) as any });
    api.conversations.applyRecovery = async (threadId, revision) => {
      controls.review.recoveryApplyCalls += 1;
      controls.review.recoveryApplyArguments = { threadId, revision };
      if (controls.review.holdRecoveryApply) {
        await new Promise<void>(resolve => { controls.review.finishRecoveryApply = resolve; });
      }
      if (controls.review.recoveryApplyError) throw new Error(controls.review.recoveryApplyError);
      const preview = recoveryPreview(mode) as any;
      return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
        data: { changed: true, backupPath: "C:/synthetic/recovery/backup.json", preview } };
    };
    api.conversations.adoptRouter = async (threadId, revision) => {
      controls.review.routerAdoptionCalls = (controls.review.routerAdoptionCalls ?? 0) + 1;
      const result = await api.conversations.applyRecovery(threadId, revision);
      return { ...result, warnings: controls.review.recoveryLaunchWarning
        ? [{ code: "CODEX_REOPEN_FAILED", message: "Synthetic launch failure" }] : [] };
    };
    api.openPath = async path => { controls.review.openedPath = path; };
  }
  controls.review.claudeThreads = [{ ...threads[0], key: "claude:project/11111111-1111-4111-8111-111111111111.jsonl",
    id: "11111111-1111-4111-8111-111111111111", title: "Claude review session", archived: false,
    source: "claudeCode", threadSource: "claudeJsonl", modelProvider: "", model: "claude-sonnet-5-5",
    filePath: "C:/synthetic/.claude/projects/project/11111111-1111-4111-8111-111111111111.jsonl" }];
  api.conversations.export = async (target, keys, targetDir, format) => {
    controls.review.conversationCalls.push({ operation: "export", target, keys, format });
    return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [], data: { exported: keys.length, directory: targetDir, files: [] } };
  };
  api.conversations.list = async target => {
    controls.review.conversationCalls.push({ operation: "list", target });
    const items = target === "claudeCode" ? controls.review.claudeThreads : null;
    if (controls.review.holdConversationLists?.includes(target)) await new Promise<void>(resolve => {
      controls.review.conversationListResolvers.push({ target, resolve });
    });
    controls.review.conversationCalls.push({ operation: "listResolved", target });
    if (items) return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
      data: { total: items.length, lastScanAt: 0, pendingDeletionKeys: [], items } };
    return ({ schemaVersion: 1, success: true, code: "ok", message: "",
    warnings: [{ code: "thread_scan_skipped", message: "Skipped synthetic-corrupt-session.jsonl: invalid UTF-8" }],
    data: { total: mode === "thread-actions" ? 2 : 1, lastScanAt: 0,
      pendingDeletionKeys: controls.review.pendingDeletionKeys ?? [],
      pendingDeletionRecoveryNeeded: controls.review.pendingDeletionRecoveryNeeded ?? false,
      items: mode === "thread-tree" ? controls.review.threadItems : mode === "thread-details-missing" ? [{ ...threads[0], rolloutStatus: "missing" as const }] : threads.slice(0, mode === "thread-actions" ? 2 : 1) } });
  };
  controls.review.detailCalls = 0;
  controls.review.messageOffsets = [];
  controls.review.messageRequests = [];
  const diagnostics = { invalidLines: 1, oversizedLines: 0, encryptedRecords: 2, compactions: 1, counterResets: 0, lastUsageOnly: 0, undatedUsage: 0, inheritedSessions: 0 };
  const usage = { input: 33100000, cached: 29400000, output: 201900, reasoning: 68600, total: 33301900 };
  api.conversations.details = async (target, key) => {
    controls.review.conversationCalls.push({ operation: "details", target, key });
    controls.review.detailCalls += 1;
    if (controls.review.detailError || mode === "thread-details-error") throw new Error("Synthetic rollout identity mismatch");
    return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [], data: {
      snapshotBytes: controls.review.snapshotBytes ?? 12345, snapshotId: controls.review.snapshotId ?? "fixture-1", usage: mode === "thread-details-unknown" ? null : usage,
      byModel: [{ label: target === "claudeCode" ? "claude-fixture-model" : "aimami_relay_2d66f8bc24::gpt-6-astra", usage: { ...usage, total: 27401900 } }, { label: "gpt-6.1-sol", usage: { ...usage, total: 5900000 } }],
      byDate: [{ label: "2026-09-29", usage: { ...usage, total: 14000000 } }, { label: "2026-09-30", usage: { ...usage, total: 19301900 } }],
      tools: [{ name: "exec_command", calls: 12 }, { name: "apply_patch", calls: 3 }], diagnostics,
    } };
  };
  api.conversations.messages = async (target, key, _id, snapshot, offset, snapshotId) => {
    controls.review.conversationCalls.push({ operation: "messages", target, key });
    controls.review.messageOffsets.push(offset);
    controls.review.messageRequests.push({ snapshot, offset, snapshotId });
    if (controls.review.messageError || snapshotId !== (controls.review.snapshotId ?? "fixture-1"))
      throw new Error("Synthetic stale snapshot; refresh");
    return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [], data: { diagnostics,
      nextOffset: offset === 0 ? 50 : null,
      messages: controls.review.messageFixtures?.[offset] ?? (offset === 0 ? [
        { role: "user", timestamp: "2026-09-30T08:00:00Z", text: target === "claudeCode" ? "Claude fixture message" : "合成的用户问题", truncated: false },
        { role: "assistant", timestamp: "2026-09-30T08:00:01Z", text: "<script>never execute this</script>\n" + "可读长消息。".repeat(500), truncated: true },
        { role: "toolCall", timestamp: "2026-09-30T08:00:02Z", text: "exec_command\npwd", truncated: false },
        { role: "toolResult", timestamp: "2026-09-30T08:00:03Z", text: "C:/synthetic/workspace", truncated: false },
      ] : [{ role: "assistant", timestamp: "", text: "第二页的合成消息", truncated: false }]),
    } };
  };
  const groups = Array.from({ length: 10 }, (_, i) => ({ key: "g" + i, cwd: "C:/synthetic/g" + i,
    folderName: "Workspace " + i, items: threads.slice(i * 10, i * 10 + 10), latestAt: 0,
    totalMessages: 20, totalSize: 1000, userMessages: 10, agentMessages: 10 }));
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  controls.review.frontendClient = queryClient;
  if (mode === "terminal-proxy") controls.review.terminalClient = queryClient;
  if (mode === "groups") controls.review.providersClient = queryClient;
  root.render(<QueryClientProvider key={++mountSequence} client={queryClient}><TooltipProvider>
    <div style={{ position: "relative", height: 600 }}><PageStage state="active">
      {(mode.startsWith("thread-details") || mode === "thread-tree" || mode === "thread-actions" || mode === "terminal-proxy" || mode === "skills" || mode === "groups" || mode === "access") && <ThreadToastProbe />}
      {mode === "desktop-updater" ? <DesktopUpdaterFixture /> : mode === "desktop-exit" ? <ClaudeDesktopRouterExitDialog /> : mode.startsWith("recovery-") ? <RecoveryFixture thread={threads[0] as ThreadSummary} /> :
        mode === "skills" ? <SkillsPage /> : mode === "terminal-proxy" ? <div className="mx-auto max-w-xl rounded-xl bg-card shadow-sm"><TerminalProxySettings currentProxy={{ mode: "manual", url: "http://127.0.0.1:7890" }} /></div> :
        mode === "image" ? <ImageWorkbenchPage /> : mode === "mcp" ? <McpPage /> : mode === "accounts" || mode.startsWith("account-resets") ? <AccountsPage refreshInterval="5m" onManageProviders={noop} /> : mode === "overview" ? <OverviewPage onNavigate={route => { controls.review.navigatedTo = route; }} onOpenProxySettings={() => { controls.review.proxySettingsOpened = true; }} /> : mode === "usage" ? <TokenUsagePanel data={{ ...usageFixture, relay: { ...usageFixture.relay, ...controls.review.relayPatch } }} loading={false} error={null} onRetry={noop} onManageProviders={noop} /> : mode === "provider" || mode === "mixed" ? <ProviderFixture mixed={mode === "mixed"} /> : mode.startsWith("thread-details") || mode === "thread-tree" || mode === "warnings" || mode === "thread-actions" || mode === "thread-queue-recovery" ? <ThreadsPage /> :
        mode === "access" || mode === "incompatible" || mode === "native-invalid" || mode === "groups" ? <><NewApiPage onOpenRelays={() => { controls.review.navigatedTo = "relays"; }} /><RelayModelsPage /></> :
        mode === "test" ? <ProviderTestDialog open provider={mixedProvider} onOpenChange={noop} onTested={noop} onFailed={noop} /> :
        mode.startsWith("clients") ? <ClientConfigs providers={controls.review.providerFixtures ? state.providers : openCode ? [mixedProvider, provider("alternate")] : [selectedProvider]} /> :
        <VirtualThreadList groups={groups} collapsedGroups={new Set()} selected={new Set()} viewMode={mode}
          onToggleSelect={noop} onToggleGroup={noop} onToggleCollapse={noop} onExport={noop}
          onOpenPath={noop} onRecover={noop} onDelete={noop} onCopyPath={noop} copiedGroupPath={null} t={key => key} />}
    </PageStage></div>
  </TooltipProvider></QueryClientProvider>);
}

controls.review = { mount, t: (key: string, options?: Record<string, unknown>) => i18n.t(key, options),
  setLanguage: (language: string) => i18n.changeLanguage(language),
  emitBalanceUpdate: (id: string, remaining: number) => {
    state = { ...state, providers: state.providers.map(provider => provider.id === id && provider.balance
      ? { ...provider, balance: { ...provider.balance, remaining } } : provider) };
    for (const callback of controls.review.balanceUpdateListeners ?? []) callback();
  },
  realBalanceQuery, clientDefaults, claudeOneMDefaults, mergeModelCatalog, errors: { errorKey, errorMessage, formatInvokeError }, getState: () => state, pixel, accessCalls: [], thumbnailCalls: [], originalCalls: [], thumbnailResolvers: [],
  modelResolutionFixture, modelResolutionRequests: [], modelResolutionResolvers: [], modelResolutionRejectors: [], holdModelResolutions: false, modelResolutionPatches: {},
  navigatedTo: null, proxySettingsOpened: false,
  imageRequests: [], imageResolvers: [], imageCancels: {}, cancelledJobs: [], gallerySaves: 0, failGallerySave: false,
  balanceRequests: [], balanceResolvers: [], balanceRejectors: [], balanceCredentialSaves: [],
  restartCalls: [], recoveryApplyCalls: 0, recoveredCalls: 0, deleteCalls: [], openedPath: null,
  holdThumbnails: false, holdOriginal: false, labels: {
    fetchModels: i18n.t("newapi.fetchModels"), cancel: i18n.t("common.cancel"), close: i18n.t("common.close"),
    clearModels: i18n.t("newapi.clearModels"),
    selectMatchingModels: i18n.t("newapi.selectMatchingModels"),
    apiKeyMissing: i18n.t("newapi.errors.apiKeyMissing"),
    clearPendingModels: i18n.t("newapi.clearPendingModels"),
    includeModelA: i18n.t("newapi.includeModelAria", { model: "model-A" }),
    includeFetchedModel: i18n.t("newapi.includeModelAria", { model: "newly-fetched-model" }),
    windowRemaining: i18n.t("newapi.balanceWindowRemaining"),
    weeklyWindow: i18n.t("newapi.balanceWindow.weekly"),
    addModelPlaceholder: i18n.t("newapi.addModelPlaceholder"),
    addModel: i18n.t("newapi.addModel"),
    defaultModelA: i18n.t("newapi.setDefaultModel", { model: "model-A" }),
    modelContext: i18n.t("newapi.contextFor", { model: "model-A" }),
    modelCatalog: i18n.t("newapi.catalogMapping.modelLevel"),
    modelCompression: i18n.t("newapi.compression.modelLevel"),
    providerCompression: i18n.t("newapi.compression.providerLevel"),
    modelResolutionRetry: i18n.t("newapi.modelResolution.retry"),
    modelInput: `${i18n.t("newapi.capabilityInput")} · model-A`,
    modelOutput: `${i18n.t("newapi.capabilityOutput")} · model-A`,
    modelReasoning: `${i18n.t("newapi.capabilityReasoning")} · model-A`,
    modelDefaultEffort: `${i18n.t("newapi.capabilityDefaultEffort")} · model-A`,
    modelFieldOverrides: i18n.t("newapi.modelFieldOverrides"),
    galleryTitle: i18n.t("image.galleryTitle"),
    chatProtocol: i18n.t("newapi.protocolFor", { model: "chat-model" }),
    inheritedProtocol: i18n.t("newapi.protocolFor", { model: "inherited-model" }),
    defaultChat: i18n.t("newapi.setDefaultModel", { model: "chat-model" }),
    defaultResponses: i18n.t("newapi.setDefaultModel", { model: "responses-model" }),
    responses: i18n.t("newapi.protocolResponses"), chat: i18n.t("newapi.protocolChatCompletions"), anthropic: i18n.t("newapi.protocolAnthropicMessages"),
    inheritChat: i18n.t("newapi.inheritProtocol", { protocol: i18n.t("newapi.protocolShortChat") }),
    inheritAnthropic: i18n.t("newapi.inheritProtocol", { protocol: i18n.t("newapi.protocolShortAnthropic") }),
    codexAccess: i18n.t("newapi.codexAccess"), accountless: i18n.t("newapi.accountless"),
    accessNative: i18n.t("newapi.accessNative"), accessRouter: i18n.t("newapi.accessRouter"), nativeNoRouter: i18n.t("newapi.nativeNoRouter"),
    restartCodex: i18n.t("newapi.restartCodex"), restartConfirmTitle: i18n.t("newapi.restartConfirmTitle"),
    nativeProtocolHint: i18n.t("newapi.nativeProtocolHint"), nativeAuthHint: i18n.t("newapi.nativeAuthHint"),
    modelCount: i18n.t("newapi.modelCount", { count: 4 }), refreshModels: i18n.t("newapi.refreshModels"), sendTest: i18n.t("newapi.sendTest"),
    configure: i18n.t("clients.configure"), changeClient: i18n.t("clients.change"), preview: i18n.t("clients.preview"), incompatibleClient: i18n.t("clients.errors.protocol"),
    clientNative: i18n.t("clients.accessNative"), clientRouter: i18n.t("clients.accessRouter"),
    clientAdvanced: i18n.t("clients.advanced"), clientApply: i18n.t("clients.apply"),
    clientApplyConflict: i18n.t("clients.applyConflict"), clientRestore: i18n.t("clients.restore"),
    clientReapply: i18n.t("clients.reapply"), clientRebaseline: i18n.t("clients.rebaseline"),
    clientSelectAllModels: i18n.t("clients.selectAllModels"),
    clientDeselectAllModels: i18n.t("clients.deselectAllModels"),
    oneMStartup: i18n.t("clients.oneMStartup"),
    oneMRoles: ["sonnet", "opus", "fable", "haiku", "subagent"].map(role => i18n.t("clients.oneMForRole", { role: i18n.t("clients." + role) })),
    openCodeModelsUnavailable: i18n.t("clients.errors.openCodeModelsUnavailable"),
    profileModelsUnavailable: i18n.t("clients.errors.profileModelsUnavailable"),
    recoveryTitle: i18n.t("threads.recoveryTitle"), recoveryCatalogConfirm: i18n.t("threads.recoveryCatalogConfirm"),
    recoveryConfirm: i18n.t("threads.recoveryConfirm"), recoveryApplying: i18n.t("threads.recoveryApplying"),
    routerAdoptionConfirm: i18n.t("threads.routerAdoptionConfirm"),
    routerAdoptionDescription: i18n.t("threads.routerAdoptionDescription"),
    recoveryMissingStatus: i18n.t("threads.recoveryStatus.missingCatalog"),
    recoveryJournalStatus: i18n.t("threads.recoveryStatus.journalBlocked"),
    recoveryOfficialStatus: i18n.t("threads.recoveryStatus.officialAccountUnverified"),
    recoveryCoordinationTitle: i18n.t("threads.recoveryCoordinationTitle"),
    recoveryOpenDirectory: i18n.t("threads.recoveryOpenDirectory"),
    disableRoutingRestore: i18n.t("newapi.disableRoutingRestore"),
    repairAll: i18n.t("threads.repairAll"), repairAllApply: i18n.t("threads.repairAllApply"),
    routeDirect: i18n.t("threads.routeDirect"), deleteThread: i18n.t("threads.deleteThread"),
    deleteSelected: i18n.t("threads.deleteSelected", { count: 1 }),
    clearDeletionQueue: i18n.t("threads.clearDeletionQueue"),
    delete: i18n.t("common.delete"),
    openGallery: i18n.t("image.openGallery"),
    openGalleryFailed: i18n.t("image.openGalleryFailed"),
    manageProfiles: i18n.t("clients.manageProfiles"),
    deleteProfile: i18n.t("clients.deleteProfile", { name: "Regression profile" }),
  } };
mount("compact");
