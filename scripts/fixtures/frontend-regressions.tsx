import { CustomInstructionsPage } from "@/components/custom-instructions/custom-instructions-page";
// Synthetic browser fixture: every application API used here is mocked.
import React, { useCallback, useState } from "react";
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
import { createPortal } from "react-dom";
import { AppSidebar, SIDEBAR_COLLAPSED_WIDTH_PX, SIDEBAR_EXPANDED_WIDTH_PX } from "@/components/layout/sidebar";
import { AppAvatarProvider } from "@/hooks/use-app-avatar";
import { SidebarInset, SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar";
import type { Route } from "@/types/navigation";
import { ClaudeDesktopRouterExitDialog } from "@/components/newapi/claude-desktop-router-exit-dialog";
import { UpdateOverlay } from "@/components/update/update-overlay";
import { useUpdateCheck } from "@/hooks/use-update-check";
import { OverviewPage } from "@/components/overview/overview-page";
import { AccountsPage } from "@/components/accounts/accounts-page";
import { UsagePage } from "@/components/usage/usage-page";
import type { AccountSource, AccountSourceKind, AccountSourceListPayload, AccountSourceMethod } from "@/types/account-sources";
import type { GatewayModel, GatewayState } from "@/types/gateway";
import type { CursorStatus } from "@/types/cursor";
import { McpPage } from "@/components/mcp/mcp-page";
import { CommandPalette } from "@/components/command-palette/command-palette";
import { SkillsPage } from "@/components/skills/skills-page";
import { ClientConfigs } from "@/components/newapi/client-configs";
import { TerminalProxySettings } from "@/components/settings/terminal-proxy-settings";
import { AutostartSetting } from "@/components/settings/autostart-setting";
import { MemoryOptimizer } from "@/components/maintenance/memory-optimizer";
import { MaintenancePage } from "@/components/maintenance/maintenance-page";
import { SettingsPage } from "@/components/settings/settings-page";
import { CustomBackgroundProvider } from "@/hooks/use-custom-background";
import type { CodexSubagentSettings, NewApiProvider, NewApiState } from "@/types/newapi";
import { buildUsage, defaultUsageEntries, usageEntry } from "./scripts/fixtures/usage-fixture";
import type { ClientConfigRequest, ClientStatus } from "@/types/clients";
import type { AggregateModel, AggregateView, AggregationState, ConnectionExportResult } from "@/types/aggregation";
import type { ThreadSummary } from "@/types/threads";
import { api } from "@/lib/api";
import { useToast } from "@/hooks/use-toast";
import i18n from "@/lib/i18n";
import { errorKey, errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { claudeOneMDefaults, clientDefaults } from "@/lib/client-config";
import { mergeModelCatalog } from "@/lib/provider-models";
import { shrinkReference } from "@/components/image-workbench/prompt-assist";
import { darkScrimFloor } from "@/lib/wallpaper-contrast";
import { installFileDropGuard } from "@/lib/file-drop-guard";

const realBalanceQuery = api.providers.queryBalance;
const controls = window as any;
controls.__TAURI_INTERNALS__ = {};
installFileDropGuard();
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
function McpClientFixture() {
  const [open, setOpen] = useState(false);
  return <><McpPage /><button onClick={() => setOpen(true)}>测试命令面板</button>
    <CommandPalette open={open} onOpenChange={setOpen} onNavigate={route => { controls.review.navigatedTo = route; }} onThemeToggle={noop} isDarkMode={false} /></>;
}
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
// Two image models and a chat model, so the model list must show exactly two.
const studioImageProvider = { ...provider("studio"), model: "gpt-image-2.5-sunburst",
  models: [model("gpt-image-2.5-sunburst"), model("gpt-image-2.5-flare"), model("studio-chat")] };
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
// Like the backend, modifiedAt is in milliseconds; item i is i hours old.
// The two newest carry the record saved beside them; older ones predate records.
const galleryMetas = [
  { prompt: "窗边侧光人像，胶片质感", revisedPrompt: "A portrait lit by soft window light, film grain", model: "gpt-image-2", size: "1024x1536", quality: "high" },
  { prompt: "雨夜霓虹街头的橘猫", revisedPrompt: null, model: "gpt-image-2.5-flare", size: "1536x1024", quality: "max" },
];
// Portrait, landscape and unknown sizes, so the masonry and album meet every shape.
const gallerySizes = [[1024, 1536], [1536, 1024], [1024, 1024], [null, null]];
const galleryItems = Array.from({ length: 100 }, (_, i) => ({
  name: "gallery-" + i + ".png", path: "C:/synthetic/gallery-" + i + ".png", size: 100, modifiedAt: Date.now() - i * 3_600_000,
  meta: galleryMetas[i] ?? null, width: gallerySizes[i % 4][0], height: gallerySizes[i % 4][1],
}));
// A gradient at the item's own proportions stands in for its thumbnail.
const galleryArt = (path: string) => {
  const i = Number(path.match(/gallery-(\d+)/)?.[1] ?? 0);
  const [width, height] = (gallerySizes[i % 4][0] ? gallerySizes[i % 4] : [800, 800]) as number[];
  const hue = (i * 47) % 360;
  return "data:image/svg+xml;utf8," + encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width / 4}" height="${height / 4}"><defs><linearGradient id="g" x2="1" y2="1">`
    + `<stop offset="0" stop-color="hsl(${hue} 70% 62%)"/><stop offset="1" stop-color="hsl(${(hue + 60) % 360} 60% 32%)"/></linearGradient></defs>`
    + `<rect width="100%" height="100%" fill="url(#g)"/><circle cx="50%" cy="42%" r="18%" fill="hsl(${(hue + 180) % 360} 70% 80% / 0.8)"/>`
    + `<text x="50%" y="84%" font-family="sans-serif" font-size="${width / 24}" fill="#fff" text-anchor="middle">${i}</text></svg>`);
};
// A full-size original large enough to zoom into and pan around.
const largeOriginal = "data:image/svg+xml;utf8," + encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="2400" height="1600"><rect width="2400" height="1600" fill="#c8844a"/>'
  + '<circle cx="1200" cy="800" r="420" fill="#24466e"/></svg>');
const pixel = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+nmQAAAABJRU5ErkJggg==";
let state: NewApiState = { providers: [imageProvider], router: { enabled: false, accessMode: "router", accountless: false, port: 0, configDrifted: false }, proxyRunning: false, activeProviderId: null };
api.providers.loadState = async () => state;
api.providers.portOwner = async () => controls.review.portOwner ?? null;
api.providers.endPortOwner = async pid => {
  controls.review.endedPortOwner = pid;
  controls.review.portOwner = null;
  state = { ...state, proxyRunning: true };
};
api.providers.setPort = async port => {
  (controls.review.routerPorts ??= []).push(port);
  controls.review.portOwner = null;
  state = { ...state, router: { ...state.router, port }, proxyRunning: true };
};
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
      routeRevision: "fixture-aggregate-v1", memberStates, gatewayId: `${entry.gatewayPrefix ?? "pool"}/${entry.publicModelId}` };
  }), externalEnabled: controls.review.aggregateExternalEnabled ?? false,
  listenerReady: !!controls.review.aggregateExternalEnabled || !!state.router.enabled,
});
api.aggregation.state = async () => aggregationState();
api.aggregation.live = async () => structuredClone(controls.review.aggregateLive ?? []);
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
api.aggregation.selectCodex = async aggregateId => {
  controls.review.aggregateCodexSelections = [...(controls.review.aggregateCodexSelections ?? []), aggregateId];
  // A default aggregate is offered in the menu it starts Codex on.
  const codexSources = state.router.codexSources && aggregateId ? { ...state.router.codexSources, aggregate: true } : state.router.codexSources;
  state = { ...state, router: { ...state.router, aggregateModelId: aggregateId, codexSources, accessMode: "router" } };
  return { state: structuredClone(state), applied: null };
};
api.aggregation.setCodexSources = async sources => {
  controls.review.codexSourceCalls = [...(controls.review.codexSourceCalls ?? []), sources];
  // Turning aggregates off drops the default aggregate with them.
  state = { ...state, router: { ...state.router, codexSources: sources,
    aggregateModelId: sources.aggregate ? state.router.aggregateModelId : null } };
  return { state: structuredClone(state), applied: null };
};
// Gateway: named keys, models by short name, connection export.
type FixtureKey = { id: string; name: string; secret: string };
const gatewayModelsFixture = (): GatewayModel[] => [
  { id: "relay-a/fast-model", kind: "relay", sourceName: "Relay A", sourceId: "relay-a", modelId: "fast-model", label: "Fast", protocol: "responses", contextWindow: 200_000 },
  { id: "relay-a/vendor/deep-model", kind: "relay", sourceName: "Relay A", sourceId: "relay-a", modelId: "vendor/deep-model", label: "Deep", protocol: "chatCompletions", contextWindow: null },
  { id: "workbuddy/glm-5.3", kind: "account", sourceName: "WorkBuddy", sourceId: "account-workbuddy", modelId: "glm-5.3", label: "GLM-5.3", protocol: "chatCompletions", contextWindow: 1_000_000 },
  { id: `${controls.review.gatewayPrefix ?? "pool"}/harbor-pool`, kind: "aggregate", sourceName: "Harbor Pool", sourceId: "pool", modelId: "harbor-pool", label: "Harbor Pool", protocol: "responses", contextWindow: 128_000 },
];
const gatewayState = (): GatewayState => {
  const all = controls.review.gatewayModels ?? gatewayModelsFixture();
  const sources = controls.review.gatewaySources ?? { relay: true, account: true, aggregate: true };
  const count = (kind: GatewayModel["kind"]) => all.filter((model: GatewayModel) => model.kind === kind).length;
  return {
    port: 43127, running: !!controls.review.gatewayKeepRunning, keepRunning: !!controls.review.gatewayKeepRunning,
    keys: (controls.review.gatewayKeys as FixtureKey[]).map(key => ({ id: key.id, name: key.name, masked: `sk-ah-…${key.secret.slice(-4)}`, createdAt: 1 })),
    models: all.filter((model: GatewayModel) => sources[model.kind]),
    aggregatePrefix: controls.review.gatewayPrefix ?? "pool",
    sources, kindCounts: { relay: count("relay"), account: count("account"), aggregate: count("aggregate") },
  };
};
const installGateway = () => {
  controls.review.gatewayKeys = structuredClone(controls.review.gatewayKeysFixture ?? []);
  controls.review.gatewayKeepRunning = false;
  controls.review.gatewayCalls = [];
  delete controls.review.gatewayPrefix;
  delete controls.review.gatewaySources;
  const record = (...call: unknown[]) => controls.review.gatewayCalls.push(call);
  const keyOf = (id: string) => {
    const key = (controls.review.gatewayKeys as FixtureKey[]).find(entry => entry.id === id);
    if (!key) throw "gateway.errors.keyNotFound";
    return key;
  };
  api.gateway.state = async () => gatewayState();
  api.gateway.createKey = async name => {
    record("create", name);
    if ((controls.review.gatewayKeys as FixtureKey[]).some(key => key.name === name)) throw "gateway.errors.keyNameTaken";
    controls.review.gatewayKeys.push({ id: `key-${controls.review.gatewayKeys.length + 1}`, name, secret: `sk-ah-fixture-${name.toLowerCase()}-0001` });
    return gatewayState();
  };
  api.gateway.renameKey = async (id, name) => { record("rename", id, name); keyOf(id).name = name; return gatewayState(); };
  api.gateway.regenerateKey = async id => { record("regenerate", id); keyOf(id).secret = keyOf(id).secret.replace(/\d{4}$/, "0002"); return gatewayState(); };
  api.gateway.deleteKey = async id => {
    record("delete", id);
    controls.review.gatewayKeys = (controls.review.gatewayKeys as FixtureKey[]).filter(key => key.id !== id);
    return gatewayState();
  };
  api.gateway.revealKey = async id => { record("reveal", id); return keyOf(id).secret; };
  api.gateway.setKeepRunning = async enabled => { record("keepRunning", enabled); controls.review.gatewayKeepRunning = enabled; return gatewayState(); };
  api.gateway.setAggregatePrefix = async prefix => {
    record("prefix", prefix);
    if (prefix === "relay-a") throw "gateway.errors.prefixTaken";
    controls.review.gatewayPrefix = prefix;
    return gatewayState();
  };
  api.gateway.setSources = async sources => {
    record("sources", structuredClone(sources));
    controls.review.gatewaySources = sources;
    return gatewayState();
  };
  api.gateway.exportConnection = async request => {
    record("export", structuredClone(request));
    const key = keyOf(request.keyId);
    const models = gatewayState().models;
    const selected = request.selectedModelId ? models.find(model => model.id === request.selectedModelId) : models[0];
    if (!selected) throw "gateway.errors.unavailable";
    const baseUrl = "http://127.0.0.1:43127/gateway" + (request.consumerProfile === "anthropicSdk" ? "" : "/v1");
    const configText = JSON.stringify({ baseUrl, model: selected.id, apiKey: request.revealCredential ? key.secret : "<gateway key>",
      protocol: request.ingressProtocol, template: request.consumerProfile }, null, 2);
    return { configText, connection: {} } as unknown as ConnectionExportResult;
  };
};
api.routing.diagnostics = async () => {
  if (controls.review.routeDiagnostics) return controls.review.routeDiagnostics;
  const base = { id: "synthetic-route", timestamp: 1_790_740_800, requestedModel: "relay::claude-opus-5-5-kiro",
  requested: { "reasoning.effort": "high" }, stage: "complete", done: true, status: 200, failureStage: null, error: null, durationMs: 840, firstTokenMs: 230,
  attempts: [{ providerId: "relay", providerName: "Synthetic channel", upstreamModel: "claude-opus-5-5-kiro", matchedModel: "claude-opus-5-5", protocol: "anthropicMessages",
    sent: { "thinking.type": "adaptive", "output_config.effort": "high", max_tokens: 8192 }, status: 200, failureStage: null, error: null, durationMs: 830,
    catalogRevision: "abcdef1234567890", matchKind: "normalized", pricingStatus: "priced", pricingSource: "catalogReference",
    attemptId: "synthetic-attempt", selectionReason: "active_provider", retryReason: null, searchCompatibility: "optional_search_omitted",
    credential: { kind: "providerApiKey", status: "recorded", reason: null, versionFingerprint: "fixture-version", accountId: null, accountStatus: "not_recorded", label: null },
    modelObservation: { servedModel: "claude-opus-5-5-kiro", modelMatchStatus: "same", modelMatchBasis: "raw_exact", unknownReason: null, conflictingModel: null } }] };
  return controls.review.desktopDiagnostics ? [base, { ...base, id: "desktop-route", clientSource: "claudeDesktop",
    requestedModel: "claude-sonnet-4-6", resolvedModel: "deepseek-fixture", routeRevision: 3,
    attempts: base.attempts.map(attempt => ({ ...attempt, selectionReason: "desktop_role_mapping" })) }] : [base];
};
// Clearing forgets finished routes and keeps running ones, as route_trace::clear does.
api.routing.clearDiagnostics = async () => {
  const current = await api.routing.diagnostics();
  controls.review.routeDiagnostics = current.filter(route => !route.done);
  controls.review.diagnosticsClears = (controls.review.diagnosticsClears ?? 0) + 1;
  return current.length - controls.review.routeDiagnostics.length;
};
const capabilityRef = { providerId: "anthropic", modelId: "claude-opus-5-5" };
api.routing.capabilities = async () => ({ stale: true, error: "catalog_unavailable_or_invalid", updatedAt: 1_790_740_800,
  catalog: { schemaVersion: 1, sourceUrl: "https://models.dev/api.json", fetchedAt: 1_790_740_000, lastCheckedAt: 1_790_740_800,
    stale: true, origin: "runtime", revision: "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890", providers: 225, models: 8339, error: null },
  models: [{
    providerId: "relay", providerName: "Synthetic channel", displayName: "My custom alias",
    manualOverrides: (["contextWindow", "price", "compression", "providerContextWindow", "providerCompression"] as const)
      .filter(field => !(controls.review.clearedFields ?? []).includes(field)),
    codexTotalCompactThreshold: 900_000,
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
      source: "catalogReference", sourceRef: capabilityRef, updatedAt: "2025-11-24T00:00:00Z", complete: true, note: null },
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
api.routing.previewCapabilityClear = async (providerId, modelId, field) => {
  (controls.review.clearPreviewCalls ??= []).push({ providerId, modelId, field });
  const base = (await api.routing.capabilities()).models[0];
  const before = structuredClone(base);
  const after = structuredClone(base);
  if (field === "contextWindow" || field === "providerContextWindow") {
    before.context = { ...before.context, value: 300000, source: "manual" };
    before.output = { ...before.output, value: 200000 };
    before.projection = { ...before.projection, contextWindow: 300000, budget: 300000, outputLimit: 200000 };
    after.context = { ...after.context, value: 100000, source: "catalog" };
    after.output = { ...after.output, value: 200000 };
    after.projection = { ...after.projection, contextWindow: 100000, budget: 100000, outputLimit: 100000,
      compression: { ...after.projection.compression, threshold: 90000 } };
    before.projection.compression.threshold = 270000;
    before.codexTotalCompactThreshold = 270000;
    after.codexTotalCompactThreshold = 90000;
  } else if (field === "price") {
    before.price.cacheRead = 0.4;
    after.price.cacheRead = 0.2;
  } else if (field === "providerCompression") {
    before.compressionScope = after.compressionScope = "nativeShared";
    before.codexTotalCompactThreshold = 90000;
    after.codexTotalCompactThreshold = 80000;
    before.compressionSharedThreshold = 90000;
    after.compressionSharedThreshold = 80000;
    before.compressionSharedPlan = { threshold: 90000, limitingModels: ["small"], blockingModels: [], state: "computed" };
    after.compressionSharedPlan = { threshold: 80000, limitingModels: ["small"], blockingModels: [], state: "computed" };
    after.compressionLimitingModels = ["small"];
  }
  const hasOverride = !controls.review.clearNoOverride;
  after.manualOverrides = after.manualOverrides.filter(value => value !== field);
  const result = {
    field, hasOverride, configRevision: "config-clear-1", catalogRevision: "catalog-clear-1", officialRevision: "official-clear-1",
    before, after: hasOverride ? after : before, scopeCount: field.startsWith("provider") || controls.review.clearSharedEffect ? 3 : 1,
    affectedModels: hasOverride ? (field.startsWith("provider") || controls.review.clearSharedEffect ? [
      { modelId, displayName: before.displayName },
      { modelId: "small", displayName: before.displayName },
      { modelId: "model-with-a-very-long-name-to-verify-wrapping-and-identity", displayName: "Long model" },
    ] : [{ modelId, displayName: before.displayName }]) : [],
  };
  if (controls.review.clearPreviewHold) await new Promise<void>(resolve => { (controls.review.clearPreviewResolvers ??= []).push(resolve); });
  if (controls.review.clearPreviewFailure) throw "newapi.errors.storage";
  return result;
};
api.routing.applyCapabilityClear = async preview => {
  (controls.review.clearApplyCalls ??= []).push(structuredClone(preview));
  if (controls.review.clearApplyHold) await new Promise<void>(resolve => { controls.review.clearApplyResolve = resolve; });
  if (controls.review.clearApplyFailure) throw "newapi.errors.storage";
  if (controls.review.clearApplyStale) {
    controls.review.clearApplyStale = false;
    return { applied: null, refreshedPreview: { ...preview, configRevision: "config-clear-2" } };
  }
  (controls.review.clearedFields ??= []).push(preview.field);
  return { applied: { state, applied: null }, refreshedPreview: null };
};
api.routing.setWebSearchProvider = async (providerId, modelId = null) => { state = { ...state, router: { ...state.router, webSearchProviderId: providerId, webSearchModelId: modelId } }; return { state, applied: null }; };
api.loadSnapshot = async () => ({ data: { status: {
  paths: { codexHome: "C:/synthetic/codex", authExists: true, registryExists: true, sessionsExists: true },
  api: { proxy: { mode: "manual", url: "http://127.0.0.1:7890" } },
  autoSwitch: { supported: true, enabled: false, threshold5hPercent: 15, thresholdWeeklyPercent: 10 },
} } }) as any;
// Like core/mcp/library.rs: one list, a switch per agent; unsupported and missing agents refuse.
const MCP_APPS = ["codex", "claudeCode", "openCode", "pi", "cursor", "harness", "claudeDesktop"];
const mcpInstalled = new Set(["codex", "claudeCode", "openCode", "cursor"]);
const mcpUnsupported = (app: string, name: string, spec: any) =>
  spec.transport === "sse" && ["codex", "pi", "harness"].includes(app) ? "mcp.unsupported.sse"
    : spec.transport !== "stdio" && app === "claudeDesktop" ? "mcp.unsupported.remote"
      : app === "harness" && !/^[A-Za-z0-9_-]{1,32}$/.test(name) ? "mcp.unsupported.harnessName" : null;
const mcpSeed = () => [
  { name: "filesystem", spec: { transport: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "C:/work"], env: { LOG_LEVEL: "info" } }, note: "", apps: ["codex", "claudeCode"] },
  { name: "docs", spec: { transport: "http", url: "https://docs.example.invalid/mcp", headers: { Authorization: "Bearer synthetic" } }, note: "文档检索", apps: ["codex"] },
  { name: "legacy-sse", spec: { transport: "sse", url: "https://sse.example.invalid/events" }, note: "", apps: ["claudeCode"] },
  ...Array.from({ length: 3 }, (_, i) => ({ name: "tool-" + i, spec: { transport: "stdio", command: "uvx", args: ["tool-" + i] }, note: "", apps: i === 1 ? [] : ["codex"] })),
];
const mcpView = (failures: any[] = [], added: string[] = []) => ({
  servers: [...controls.review.mcpLibrary].sort((a: any, b: any) => a.name.localeCompare(b.name)).map((server: any) => ({
    name: server.name, spec: structuredClone(server.spec), note: server.note,
    cells: MCP_APPS.map(app => {
      const reason = mcpUnsupported(app, server.name, server.spec);
      const on = server.apps.includes(app);
      return { app, on, state: !mcpInstalled.has(app) ? "notInstalled" : reason ? "unsupported" : on ? "synced" : "off", ...(reason ? { reason } : {}) };
    }),
  })),
  apps: MCP_APPS.map(app => ({ app, path: `C:/synthetic/${app}/mcp.json`, installed: mcpInstalled.has(app), unmanaged: app === "cursor" ? ["cursor-only"] : [] })),
  failures, added,
});
/** Turn a server on or off for an agent, as library.rs `switch` does. */
const mcpSwitch = (server: any, app: string, on: boolean) => {
  if (on && !mcpInstalled.has(app)) return [{ name: server.name, app, error: "mcp.errors.notInstalled" }];
  const reason = on && mcpUnsupported(app, server.name, server.spec);
  if (reason) return [{ name: server.name, app, error: reason }];
  server.apps = on ? [...new Set([...server.apps, app])] : server.apps.filter((item: string) => item !== app);
  return [];
};
const mcpChange = async (operation: string, detail: object, change: () => any[]) => {
  controls.review.mcpCalls.push({ operation, ...structuredClone(detail) });
  if (controls.review.mcpHold) await new Promise(resolve => { controls.review.mcpResolve = resolve; });
  return mcpView(change()) as any;
};
api.loadMcpLibrary = async () => { controls.review.mcpCalls.push({ operation: "load" }); return mcpView() as any; };
api.importMcpServers = async () => {
  const fresh = !controls.review.mcpLibrary.some((server: any) => server.name === "cursor-only");
  const view = await mcpChange("import", {}, () => {
    if (fresh) controls.review.mcpLibrary.push({ name: "cursor-only", spec: { transport: "stdio", command: "node", args: ["cursor.js"] }, note: "", apps: ["cursor"] });
    return [];
  });
  return { ...view, added: fresh ? ["cursor-only"] : [] };
};
api.saveMcpServer = async (original, server) => {
  if (!original && controls.review.mcpLibrary.some((item: any) => item.name === server.name)) throw "mcp.errors.duplicate";
  return mcpChange("save", { original, server }, () => {
    let entry = controls.review.mcpLibrary.find((item: any) => item.name === server.name);
    if (!entry) controls.review.mcpLibrary.push(entry = { name: server.name, spec: server.spec, note: server.note, apps: [] });
    entry.spec = server.spec;
    entry.note = server.note;
    return MCP_APPS.flatMap(app => server.apps.includes(app) || entry.apps.includes(app) ? mcpSwitch(entry, app, server.apps.includes(app)) : []);
  });
};
api.setMcpServers = async (names, app, on) => mcpChange("set", { names, app, on }, () => controls.review.mcpLibrary
  .filter((server: any) => names.includes(server.name))
  .flatMap((server: any) => app ? mcpSwitch(server, app, on)
    : MCP_APPS.filter(item => on ? mcpInstalled.has(item) && !mcpUnsupported(item, server.name, server.spec) : server.apps.includes(item))
      .flatMap(item => mcpSwitch(server, item, on))));
api.removeMcpServer = async name => mcpChange("remove", { name }, () => {
  controls.review.mcpLibrary = controls.review.mcpLibrary.filter((server: any) => server.name !== name);
  return [];
});
api.resyncMcpServers = async () => mcpChange("resync", {}, () => []);
api.parseMcpPaste = async text => {
  controls.review.mcpCalls.push({ operation: "paste", text });
  const trimmed = text.trim();
  if (trimmed.startsWith("{")) {
    const servers = JSON.parse(trimmed).mcpServers ?? {};
    return Object.entries(servers).map(([name, spec]: [string, any]) => ({ name, spec: spec.url
      ? { transport: spec.type === "sse" ? "sse" : "http", url: spec.url, headers: spec.headers ?? {} }
      : { transport: "stdio", command: spec.command, args: spec.args ?? [], env: spec.env ?? {} } })) as any;
  }
  const words = trimmed.split(/\s+/);
  if (!words[0]) throw "mcp.errors.paste";
  return [{ name: "", spec: { transport: "stdio", command: words[0], args: words.slice(1) } }] as any;
};
const quotaWindow = (remainingPercent: number, windowMinutes: number) => ({ usedPercent: 100 - remainingPercent, remainingPercent, windowMinutes, resetsAt: 1_900_000_000 });
const accountList = { activeAccountKey: "acct-a", refreshedAt: 1_800_000_000, items: [
  { accountKey: "acct-a", email: "alpha@example.invalid", displayName: "Alpha", plan: "pro", authMode: "chatgpt", isActive: true, createdAt: 1_700_000_000, lastUsedAt: 1_800_000_000, lastUsageAt: 1_800_000_000, primaryWindow: quotaWindow(72, 300), secondaryWindow: quotaWindow(40, 10080), usageStatus: "healthy", usageError: null },
  { accountKey: "acct-b", email: "beta@example.invalid", displayName: "Beta", plan: "plus", authMode: "chatgpt", isActive: false, createdAt: 1_700_000_000, lastUsedAt: null, lastUsageAt: 1_800_000_000, primaryWindow: quotaWindow(8, 300), secondaryWindow: quotaWindow(90, 10080), usageStatus: "healthy", usageError: null },
  { accountKey: "acct-c", email: "gamma@example.invalid", displayName: "Gamma", plan: "team", authMode: "chatgpt", isActive: false, createdAt: 1_700_000_000, lastUsedAt: null, lastUsageAt: null, primaryWindow: null, secondaryWindow: null, usageStatus: "expired", usageError: "token expired" },
] };
api.accounts.refresh = async () => accountList as any;
api.accounts.list = async () => accountList as any;
// Tests replace `usageEntries` to aggregate their own calls, or patch the result.
api.usage.overview = async range => ({
  ...buildUsage(range, controls.review.usageEntries ?? defaultUsageEntries()), ...controls.review.usagePatch,
});
// Account sources: one WorkBuddy account with credits and today's check-in,
// a Gemini CLI account still without its Google Cloud project, the rest
// empty; Command Code signs in three ways, one of them a pasted key.
const beijingToday = () => new Date(Date.now() + 8 * 3600_000).toISOString().slice(0, 10);
const ACCOUNT_METHOD = [{ id: "account", input: null }] as AccountSourceMethod[];
const plainSource = (kind: AccountSourceKind, name: string, extra: Partial<AccountSource> = {}): AccountSource => ({
  kind, name, providerId: `account-${kind}`, protocol: "chatCompletions", exposed: false, models: [], accounts: [], usageUrl: null,
  hasQuota: true, fetchesModels: true, methods: ACCOUNT_METHOD, checksIn: false, autoCheckin: false, ...extra,
});
const accountSourcesFixture = (): AccountSourceListPayload => ({ today: beijingToday(), sources: [
  plainSource("chatgpt-api", "ChatGPT", { protocol: "responses", usageUrl: "https://chatgpt.com/settings/usage", hasQuota: false }),
  plainSource("workbuddy", "WorkBuddy", { exposed: true, checksIn: true, autoCheckin: true,
    models: [{ id: "glm-5.3", name: "GLM-5.3", contextWindow: 1_000_000, reasoning: ["low", "high"] }, { id: "kimi-k3-1", name: "Kimi-K3", contextWindow: 1_000_000, reasoning: [] }],
    accounts: [
      { id: "wb-1", name: "Harbor", plan: "Pro", active: true, lapsed: null, quota: { used: 750, limit: 1000, unit: "credits", paid: true, checkedAt: 1, error: null }, addedAt: 1,
        checkin: { day: beijingToday(), at: 1, outcome: "claimed", credit: 5, streak: 3, message: null, validUntil: null, offline: false, tries: 0 }, checkinOff: false },
      { id: "wb-2", name: "Spare", plan: null, active: false, lapsed: "signed out", quota: null, addedAt: 2, checkin: null, checkinOff: false },
    ] }),
  plainSource("workbuddy-ai", "WorkBuddy AI", { models: [{ id: "gpt-5.5", name: "GPT-5.5", contextWindow: 1_000_000, reasoning: [] }] }),
  plainSource("copilot", "GitHub Copilot", { usageUrl: "https://github.com/settings/copilot/features" }),
  plainSource("gemini-cli", "Gemini CLI", {
    accounts: [{ id: "gc-1", name: "dev@corp.example", plan: "Standard", project: "", active: true, lapsed: null, addedAt: 1, checkin: null, checkinOff: false,
      quota: { used: 0, limit: 0, unit: "", paid: null, checkedAt: 1, error: "Google serves Gemini CLI's sign-in only to Gemini Code Assist Standard and Enterprise" } }] }),
  plainSource("antigravity", "Antigravity", { risk: "Google may suspend an Antigravity account it sees used outside Antigravity. Use one you can afford to lose." }),
  plainSource("kiro", "Kiro", { usageUrl: "https://app.kiro.dev/account/usage" }),
  plainSource("grok", "Grok", { protocol: "responses" }),
  plainSource("cursor", "Cursor", { usageUrl: "https://cursor.com/dashboard" }),
  plainSource("minimax-code", "MiniMax Code", { protocol: "anthropicMessages", checksIn: true, risk: "MiniMax may limit an account used outside MiniMax Code." }),
  plainSource("minimax-code-global", "MiniMax Code (Global)", { protocol: "anthropicMessages", checksIn: true, risk: "MiniMax may limit an account used outside MiniMax Code." }),
  plainSource("commandcode", "Command Code", { methods: [{ id: "browser", input: null }, { id: "cli", input: null }, { id: "key", input: "secret" }] }),
  plainSource("zcode", "ZCode", { protocol: "anthropicMessages", methods: [{ id: "zai", input: null }, { id: "bigmodel", input: null }, { id: "app", input: null },
    { id: "key-zai", input: "secret" }, { id: "key-bigmodel", input: "secret" }] }),
  plainSource("opencode-zen", "OpenCode Zen", { hasQuota: false, usageUrl: "https://opencode.ai/workspace",
    methods: [{ id: "import", input: null }, { id: "key", input: "secret" }, { id: "free", input: null }] }),
  plainSource("opencode-go", "OpenCode Go", { methods: [{ id: "import", input: null }, { id: "key", input: "secret" }] }),
  plainSource("qoder", "Qoder", { checksIn: true, methods: [{ id: "account", input: null }, { id: "pat", input: "secret" }] }),
  plainSource("qoder-cn", "Qoder CN", { checksIn: true, methods: [{ id: "account", input: null }, { id: "vpc", input: "text" }] }),
  plainSource("trae-cn", "Trae CN", { checksIn: true }),
  plainSource("trae-global", "Trae Global"),
] });
const installAccountSources = () => {
  let payload = accountSourcesFixture();
  controls.review.sourceCalls = [];
  controls.review.sourceSignIns = [];
  const record = (name: string, ...args: unknown[]) => controls.review.sourceCalls.push([name, ...args]);
  api.accountSources.list = async () => structuredClone(payload);
  api.accountSources.onSignInPrompt = async callback => {
    controls.review.sourcePrompt = callback;
    return () => { if (controls.review.sourcePrompt === callback) controls.review.sourcePrompt = undefined; };
  };
  api.accountSources.signIn = (kind: AccountSourceKind, attemptId: string, method?: string, input?: string) => new Promise((resolve, reject) => {
    record("signIn", kind, ...(method ? [method, input ?? null] : []));
    if (kind === "copilot") {
      setTimeout(() => controls.review.sourcePrompt?.({ attemptId, kind, url: "https://github.com/login/device", code: "ABCD-1234" }), 0);
    }
    controls.review.sourceSignIns.push({ kind, attemptId, reject, resolve: () => {
      payload = { ...payload, sources: payload.sources.map(source => source.kind === kind
        ? { ...source, exposed: true, accounts: [...source.accounts, { id: "new-1", name: "me@example.com", plan: "plus", active: source.accounts.length === 0, lapsed: null, quota: null, addedAt: 3, checkin: null, checkinOff: false }] }
        : source) };
      resolve(structuredClone(payload));
    } });
  });
  api.accountSources.cancelSignIn = async attemptId => { record("cancelSignIn", attemptId); return true; };
  api.accountSources.activate = async id => {
    record("activate", id);
    payload = { ...payload, sources: payload.sources.map(source => source.accounts.some(account => account.id === id)
      ? { ...source, accounts: source.accounts.map(account => ({ ...account, active: account.id === id })) } : source) };
    return structuredClone(payload);
  };
  api.accountSources.remove = async id => {
    record("remove", id);
    payload = { ...payload, sources: payload.sources.map(source => ({ ...source, accounts: source.accounts.filter(account => account.id !== id) })) };
    return structuredClone(payload);
  };
  api.accountSources.refreshQuota = async id => { record("refreshQuota", id); return structuredClone(payload); };
  api.accountSources.refreshModels = async kind => { record("refreshModels", kind); return structuredClone(payload); };
  api.accountSources.setProject = async (id, project) => {
    record("setProject", id, project);
    payload = { ...payload, sources: payload.sources.map(source => ({ ...source, accounts: source.accounts.map(account => account.id === id
      ? { ...account, project, quota: { used: 20, limit: 100, unit: "percent", paid: true, checkedAt: 2, error: null } } : account) })) };
    return structuredClone(payload);
  };
  api.accountSources.setExposed = async (kind, exposed) => {
    record("setExposed", kind, exposed);
    payload = { ...payload, sources: payload.sources.map(source => source.kind === kind ? { ...source, exposed } : source) };
    return structuredClone(payload);
  };
  api.accountSources.setCheckin = async (kind, on) => {
    record("setCheckin", kind, on);
    payload = { ...payload, sources: payload.sources.map(source => source.kind === kind ? { ...source, autoCheckin: on } : source) };
    return structuredClone(payload);
  };
  api.accountSources.setCheckinOff = async (id, off) => {
    record("setCheckinOff", id, off);
    payload = { ...payload, sources: payload.sources.map(source => ({ ...source, accounts: source.accounts.map(account => account.id === id ? { ...account, checkinOff: off } : account) })) };
    return structuredClone(payload);
  };
  api.accountSources.checkIn = async kind => { record("checkIn", kind); return structuredClone(payload); };
  api.accountSources.setHiddenModels = async (kind, hidden) => {
    record("setHiddenModels", kind, hidden);
    // Leaving every model out of a shared source stops sharing it.
    payload = { ...payload, sources: payload.sources.map(source => source.kind === kind ? { ...source, hiddenModels: hidden,
      exposed: source.exposed && source.models.some(model => !hidden.includes(model.id)) } : source) };
    return structuredClone(payload);
  };
};
// Like core/skills/hub.rs: one copy per skill, a link per agent; missing agents and taken names refuse.
const SKILL_APPS = ["codex", "claudeCode", "openCode", "pi", "cursor", "harness"];
const skillInstalled = new Set(["codex", "claudeCode", "cursor"]);
const skillSeed = (mode: string) => mode === "overview" ? {
  skills: ["one", "two", "three"].map(name => ({ name, title: name, description: "", apps: ["codex"], occupied: [] })),
  unmanaged: [],
  backups: ["b1", "b2"].map(id => ({ id, skillID: id, name: id, title: null, relativePath: id, backupPath: `C:/synthetic/app/skill-backups/${id}/skill`, createdAt: 1_700_000_000 })),
} : {
  skills: [
    { name: "sample", title: "Sample Skill", description: "Synthetic skill", apps: ["codex", "claudeCode"], occupied: [] },
    { name: "review", title: "Review", description: "Checks diffs", apps: ["codex"], occupied: ["cursor"] },
  ],
  unmanaged: [
    { name: "notes", title: "Notes", description: "Claude notes", path: "C:/synthetic/claudeCode/skills/notes", app: "claudeCode", legacyDisabled: false, external: false, inLibrary: false, conflict: false },
    { name: "old", title: "Old", description: "Turned off before", path: "C:/synthetic/app/disabled-skills/old", app: "codex", legacyDisabled: true, external: false, inLibrary: false, conflict: false },
    { name: "linked", title: "Linked", description: "cc-switch managed", path: "C:/synthetic/claudeCode/skills/linked", app: "claudeCode", legacyDisabled: false, external: true, inLibrary: false, conflict: false },
    { name: "review", title: "Review", description: "An older copy", path: "C:/synthetic/claudeCode/skills/review", app: "claudeCode", legacyDisabled: false, external: false, inLibrary: true, conflict: true },
  ],
  backups: [{ id: "backup-gone", skillID: "gone", name: "gone", title: null, relativePath: "gone", backupPath: "C:/synthetic/app/skill-backups/backup-gone/skill", createdAt: 1_700_000_000 }],
};
const skillView = (failures: any[] = []) => {
  const state = controls.review.skillState;
  return {
    library: "C:/synthetic/app/skills-library",
    skills: [...state.skills].sort((a: any, b: any) => a.name.localeCompare(b.name)).map((skill: any) => ({
      name: skill.name, title: skill.title, description: skill.description, path: `C:/synthetic/app/skills-library/${skill.name}`,
      cells: SKILL_APPS.map(app => {
        const state = !skillInstalled.has(app) ? "notInstalled" : skill.apps.includes(app) ? "on" : skill.occupied.includes(app) ? "occupied" : "off";
        return { app, on: state === "on", state };
      }),
    })),
    apps: SKILL_APPS.map(app => ({ app, dir: `C:/synthetic/${app}/skills`, installed: skillInstalled.has(app) })),
    unmanaged: structuredClone(state.unmanaged),
    backups: structuredClone(state.backups),
    failures,
  } as any;
};
const skillChange = async (operation: string, detail: object, change: () => any[]) => {
  controls.review.skillCalls.push({ operation, ...structuredClone(detail) });
  if (controls.review.skillHold) await new Promise(resolve => { controls.review.skillResolve = resolve; });
  return skillView(change());
};
const skillSwitch = (skill: any, app: string, on: boolean) => {
  if (on && !skillInstalled.has(app)) return [{ name: skill.name, app, error: "skills.errors.notInstalled" }];
  if (on && skill.occupied.includes(app)) return [{ name: skill.name, app, error: "skills.errors.occupied" }];
  skill.apps = on ? [...new Set([...skill.apps, app])] : skill.apps.filter((item: string) => item !== app);
  return [];
};
api.loadSkillLibrary = async () => { controls.review.skillReads = (controls.review.skillReads ?? 0) + 1; return skillView(); };
api.setSkills = async (names, app, on) => skillChange("set", { names, app, on }, () => controls.review.skillState.skills
  .filter((skill: any) => names.includes(skill.name))
  .flatMap((skill: any) => (app ? [app] : [...skillInstalled]).flatMap(item => skillSwitch(skill, item, on))));
api.pickFolder = async () => controls.review.pickedFolder ?? null;
api.addSkill = async (path, apps) => {
  const name = path.split(/[\\/]/).filter(Boolean).pop()!;
  if (controls.review.skillState.skills.some((skill: any) => skill.name === name)) throw "skills.errors.duplicate";
  return skillChange("add", { path, apps }, () => {
    controls.review.skillState.skills.push({ name, title: name, description: "Added", apps: apps.filter(app => skillInstalled.has(app)), occupied: [] });
    return [];
  });
};
api.importSkills = async items => skillChange("import", { items }, () => {
  const state = controls.review.skillState;
  for (const item of items) {
    state.unmanaged = state.unmanaged.filter((entry: any) => !(entry.name === item.name && entry.app === item.app && entry.legacyDisabled === item.legacyDisabled));
    const existing = state.skills.find((skill: any) => skill.name === item.name);
    if (existing) { if (!item.legacyDisabled) existing.apps = [...new Set([...existing.apps, item.app])]; }
    else state.skills.push({ name: item.name, title: item.name, description: "Imported", apps: item.legacyDisabled ? [] : [item.app], occupied: [] });
  }
  return [];
});
api.uninstallSkill = async name => skillChange("uninstall", { name }, () => {
  const state = controls.review.skillState;
  state.skills = state.skills.filter((skill: any) => skill.name !== name);
  state.backups.unshift({ id: `backup-${name}`, skillID: name, name, title: null, relativePath: name, backupPath: `C:/synthetic/app/skill-backups/backup-${name}/skill`, createdAt: 1_800_000_000 });
  return [];
});
api.restoreSkill = async id => {
  const backup = controls.review.skillState.backups.find((item: any) => item.id === id);
  if (controls.review.skillState.skills.some((skill: any) => skill.name === backup.name)) throw "skills.errors.duplicate";
  return skillChange("restore", { id }, () => { controls.review.skillState.skills.push({ name: backup.name, title: backup.name, description: "", apps: [], occupied: [] }); return []; });
};
api.deleteSkillBackup = async id => skillChange("deleteBackup", { id }, () => {
  controls.review.skillState.backups = controls.review.skillState.backups.filter((item: any) => item.id !== id);
  return [];
});
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
// Connection changes never migrate threads; a running Codex keeps its old
// connection until the user restarts it.
const connectionChange = () => {
  const restartRequired = !!controls.review.codexRunning;
  if (restartRequired) state = { ...state, router: { ...state.router, restartPending: true } };
  return { state, applied: null, restartRequired };
};
api.routing.setAccessMode = async mode => {
  controls.review.accessCalls.push(mode);
  controls.review.routingCalls.push({ operation: "accessMode", mode });
  if (controls.review.routingMutationFailure) throw controls.review.routingMutationFailure;
  state = { ...state, router: { ...state.router, accessMode: mode } };
  return connectionChange();
};
api.routing.setEnabled = async enabled => {
  controls.review.routingCalls.push({ operation: "enabled", enabled });
  if (controls.review.routingMutationFailure) throw controls.review.routingMutationFailure;
  state = { ...state, router: { ...state.router, enabled } };
  return connectionChange();
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
  state = { ...state, router: { ...state.router, restartPending: false } };
};
api.targets.configs = async () => [];
api.targets.launchPlan = async target => controls.review.launchPlans?.[target]
  ?? (target === "codex" ? { kind: "app", installed: true, running: true } : { kind: "terminal", command: target === "claudeCode" ? "claude" : target, installed: true });
api.targets.discardResume = async targetId => { controls.review.discardedResume = targetId; };
api.targets.launch = async (targetId, harnessProfile, folder) => {
  (controls.review.launchCalls ??= []).push({ targetId, harnessProfile, folder });
};
const keyFile = (id: string, name: string, path: string, exists: boolean, directory = false) =>
  ({ id, name, path, exists, directory });
api.targets.files = async () => [
  { client: "codex", files: [
    keyFile("config", "config.toml", "C:/synthetic/.codex/config.toml", true),
    keyFile("auth", "auth.json", "C:/synthetic/.codex/auth.json", true),
    keyFile("registry", "registry.json", "C:/synthetic/.codex/agentharbor/accounts/registry.json", false),
    keyFile("sessions", "sessions/", "C:/synthetic/.codex/sessions", true, true),
    keyFile("instructions", "AGENTS.md", "C:/synthetic/.codex/AGENTS.md", false),
  ] },
  { client: "claudeCode", files: [
    keyFile("settings", "settings.json", "C:/synthetic/.claude/settings.json", true),
    keyFile("userConfig", ".claude.json", "C:/synthetic/.claude.json", true),
    keyFile("sessions", "projects/", "C:/synthetic/.claude/projects", true, true),
    keyFile("instructions", "CLAUDE.md", "C:/synthetic/.claude/CLAUDE.md", false),
  ] },
  { client: "claudeDesktop", files: [
    keyFile("normalConfig", "claude_desktop_config.json", "C:/synthetic/Claude/claude_desktop_config.json", true),
    keyFile("thirdPartyConfig", "claude_desktop_config.json", "C:/synthetic/Claude-3p/claude_desktop_config.json", false),
    keyFile("profile", "profile.json", "C:/synthetic/Claude-3p/configLibrary/profile.json", false),
    keyFile("meta", "_meta.json", "C:/synthetic/Claude-3p/configLibrary/_meta.json", false),
  ] },
  { client: "opencode", files: [keyFile("config", "opencode.json", "C:/synthetic/.config/opencode/opencode.json", true)] },
  { client: "pi", files: [
    keyFile("models", "models.json", "C:/synthetic/.pi/agent/models.json", true),
    keyFile("auth", "auth.json", "C:/synthetic/.pi/agent/auth.json", false),
    keyFile("settings", "settings.json", "C:/synthetic/.pi/agent/settings.json", false),
    keyFile("sessions", "sessions/", "C:/synthetic/.pi/agent/sessions", false, true),
  ] },
  { client: "cursor", files: [
    keyFile("settings", "settings.json", "C:/synthetic/Cursor/User/settings.json", true),
    keyFile("cliConfig", "cli-config.json", "C:/synthetic/.cursor/cli-config.json", false),
    keyFile("certificate", "cursor-ca.crt", "C:/synthetic/.codex/agentharbor/cursor/cursor-ca.crt", true),
    keyFile("conversations", "conversations.db", "C:/synthetic/.codex/agentharbor/cursor/conversations.db", false),
  ] },
  { client: "harness", files: [
    keyFile("profiles", "profiles/", "C:/synthetic/.dsh/profiles", true, true),
    keyFile("credentials", ".credentials.yaml", "C:/synthetic/.dsh/.credentials.yaml", true),
    keyFile("homePatch", "cordis.patch.yml", "C:/synthetic/.dsh/cordis.patch.yml", false),
    keyFile("sessions", "sessions/", "C:/synthetic/.dsh/sessions", true, true),
  ] },
];
// Cursor injection: a settings record the switches change, as cursor_inject does.
const cursorFixture = (): CursorStatus => {
  const saved = controls.review.cursor ?? (controls.review.cursor = { ide: false, cli: false, certificate: "untrusted",
    sources: { relay: true, account: true, aggregate: true }, tabModel: null, commitModel: null, commitLocale: "english" });
  const active = saved.ide || saved.cli;
  const port = active ? 47331 : null;
  const models: CursorStatus["models"] = (controls.review.cursorModels ?? [
    { id: "relay-a/model-A", cursorId: "ah-3c2f1a9b0d", label: "Model A", sourceName: "Relay A", kind: "relay" as const },
    { id: "pool/harbor-pool", cursorId: "ah-81d0e4c7aa", label: "Harbor Pool", sourceName: "Harbor Pool", kind: "aggregate" as const },
  ]).filter((model: CursorStatus["models"][number]) => saved.sources[model.kind]);
  return { ideInstalled: true, cliInstalled: true, cliSupported: true, settingsPath: "C:/synthetic/Cursor/User/settings.json",
    ide: saved.ide, cli: saved.cli, running: active, port, url: port ? `http://127.0.0.1:${port}` : null,
    ideApplied: saved.ide, cliApplied: saved.cli, cliCommand: port ? `$env:CURSOR_API_ENDPOINT="http://127.0.0.1:${port}"; cursor-agent` : null,
    certificate: saved.certificate, certificatePath: "C:/synthetic/.codex/agentharbor/cursor/cursor-ca.crt", trustCommand: null,
    sources: saved.sources, models, tabModel: saved.tabModel, commitModel: saved.commitModel, commitLocale: saved.commitLocale,
    webSearchReady: false, routerRunning: active };
};
const cursorCall = (...call: unknown[]) => { (controls.review.cursorCalls ??= []).push(call); };
api.cursor.status = async () => cursorFixture();
api.cursor.setIde = async enabled => {
  cursorCall("ide", enabled);
  cursorFixture();
  if (enabled && controls.review.cursor.certificate !== "trusted") throw "cursor.errors.untrusted";
  controls.review.cursor.ide = enabled;
  return cursorFixture();
};
api.cursor.setCli = async enabled => { cursorCall("cli", enabled); cursorFixture(); controls.review.cursor.cli = enabled; return cursorFixture(); };
api.cursor.setSources = async sources => { cursorCall("sources", structuredClone(sources)); cursorFixture(); controls.review.cursor.sources = sources; return cursorFixture(); };
api.cursor.setTabModel = async model => { cursorCall("tab", model); cursorFixture(); controls.review.cursor.tabModel = model; return cursorFixture(); };
api.cursor.setCommitModel = async (model, locale) => {
  cursorCall("commit", model, locale);
  cursorFixture();
  Object.assign(controls.review.cursor, { commitModel: model, commitLocale: locale });
  return cursorFixture();
};
api.cursor.trustCertificate = async () => { cursorCall("trust"); cursorFixture(); controls.review.cursor.certificate = "trusted"; return cursorFixture(); };
api.cursor.distrustCertificate = async () => {
  cursorCall("distrust");
  cursorFixture();
  Object.assign(controls.review.cursor, { certificate: "untrusted", ide: false });
  return cursorFixture();
};
api.cursor.restartIde = async () => { cursorCall("restart"); };
api.cursor.openTerminal = async () => { cursorCall("terminal"); };
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
api.image.keyStatus = async site => Boolean(controls.review.imageKeys?.[site.trim()]);
api.image.keySites = async () => Object.keys(controls.review.imageKeys ?? {});
api.image.saveKey = async (site, key) => {
  if (controls.review.imageKeySaveHold) await new Promise(resolve => (controls.review.imageKeySaveResolvers ??= []).push(Object.assign(resolve, { site })));
  (controls.review.imageKeys ??= {})[site.trim()] = key;
  controls.review.imageKeySaveResolved = (controls.review.imageKeySaveResolved ?? 0) + 1;
  (controls.review.imageKeySaveResolvedSites ??= []).push(site);
};
api.image.deleteKey = async site => { delete (controls.review.imageKeys ??= {})[site.trim()]; };
api.image.generate = (request, jobId) => new Promise((resolve, reject) => {
  if (!request.providerId && request.apiKey.trim()) (controls.review.imageKeys ??= {})[request.baseUrl.trim()] = request.apiKey;
  controls.review.imageRequests.push(request);
  controls.review.imageResolvers.push((images = [pixel]) => resolve({ images, model: request.model, revisedPrompts: [] }));
  if (jobId) controls.review.imageCancels[jobId] = () => reject("image.errors.cancelled");
});
api.image.cancelGeneration = async jobId => {
  controls.review.cancelledJobs.push(jobId);
  controls.review.imageCancels[jobId]?.();
  return true;
};
api.image.saveToGallery = async (_dataUrls, meta) => {
  controls.review.gallerySaves += 1;
  (controls.review.galleryMetas ??= []).push(meta ?? null);
  if (controls.review.failGallerySave) throw new Error("image.errors.save");
  return { directory: "C:/synthetic", files: ["C:/synthetic/generated.png"] };
};
api.image.saveFile = async (path, dataUrl) => { (controls.review.imageFileSaves ??= []).push({ path, dataUrl }); };
api.image.listGallery = async () => ({ directory: "C:/synthetic", items: galleryItems });
// Like image_prompts.rs, the saved copy is what the next load returns.
const promptBook = (kind = "prompts") => (kind === "materials" ? "promptMaterials" : "promptLibrary");
api.image.loadPromptLibrary = async kind => structuredClone(controls.review[promptBook(kind)]);
api.image.loadPromptAssist = async () => structuredClone(controls.review.promptAssist);
api.image.savePromptAssistSettings = async settings => {
  controls.review.promptAssist.settings = { ...settings };
  return { ...settings };
};
// Each polish waits until the test answers or fails it.
api.image.optimizePrompt = (model, prompt, request, materials, references) => new Promise((resolve, reject) => {
  controls.review.assistRequests.push({ model, prompt, request, materials, references, resolve, reject });
});
// Like image_materials.rs: a save keeps each picture by id, an add appends to its group.
api.image.loadImageMaterials = async () => structuredClone(controls.review.imageMaterials);
api.image.saveImageMaterials = async library => {
  const known = new Map(controls.review.imageMaterials.groups.flatMap(group => group.items).map(item => [item.id, item.file]));
  const groups = library.groups.map(group => ({ ...group, items: group.items.map(item => {
    if (!known.has(item.id)) throw "image.errors.imageMaterialsInvalid";
    return { ...item, file: known.get(item.id) };
  }) }));
  controls.review.imageMaterials = { ...controls.review.imageMaterials, groups };
  return structuredClone(controls.review.imageMaterials);
};
api.image.addImageMaterials = async (groupId, images) => {
  const library = structuredClone(controls.review.imageMaterials);
  const group = library.groups.find(item => item.id === groupId);
  if (!group) throw "image.errors.imageMaterialsInvalid";
  for (const image of images) {
    const id = "img-" + (controls.review.imageMaterialSeq = (controls.review.imageMaterialSeq ?? 0) + 1);
    controls.review.imageMaterialData[id] = image.dataUrl;
    group.items.push({ id, name: image.name, note: "", file: id + ".png" });
  }
  controls.review.imageMaterials = library;
  return structuredClone(library);
};
api.image.loadImageMaterial = async (id, edge) => {
  (controls.review.imageMaterialLoads ??= []).push({ id, edge: edge ?? null });
  const data = controls.review.imageMaterialData[id];
  if (!data) throw "image.errors.notFound";
  return data;
};
// Like add_presets in image_prompts.rs: same-named groups join, what is there is skipped.
api.image.addPromptPresets = async kind => {
  const key = promptBook(kind);
  const preset = kind === "materials"
    ? { name: "服装", templates: [{ id: "p1", title: "JK 制服", note: "日系校园", prompt: "水手服，百褶裙", pinned: false }] }
    : { name: "成套穿搭", templates: [{ id: "p2", title: "芭蕾风仙女", note: "", prompt: "芭蕾风少女，粉色纱裙", pinned: false }] };
  const library = structuredClone(controls.review[key]);
  let group = library.groups.find(item => item.name === preset.name);
  const created = group ? 0 : 1;
  if (!group) library.groups.push(group = { id: "preset-" + key, name: preset.name, templates: [] });
  const fresh = preset.templates.filter(item => !group.templates.some(have => have.title === item.title && have.prompt === item.prompt));
  group.templates.push(...fresh);
  controls.review[key] = library;
  return { library: structuredClone(library), groups: created, templates: fresh.length, skipped: preset.templates.length - fresh.length };
};
api.image.savePromptLibrary = async (library, kind) => {
  if (controls.review.failPromptSave) throw "image.errors.promptLibrary";
  controls.review[promptBook(kind)] = structuredClone(library);
  if (promptBook(kind) === "promptLibrary") controls.review.promptSaves += 1;
  return structuredClone(library);
};
api.openPath = async path => {
  controls.review.openedPath = path;
  if (controls.review.failOpenPath) throw new Error("synthetic open path failure");
};
api.image.loadGalleryThumbnail = (path, edge) => {
  controls.review.thumbnailCalls.push(path);
  controls.review.thumbnailEdges.push(edge ?? null);
  return controls.review.holdThumbnails
    ? new Promise(resolve => controls.review.thumbnailResolvers.push(() => resolve(pixel)))
    : Promise.resolve(galleryArt(path));
};
api.image.loadGalleryImage = path => {
  controls.review.originalCalls.push(path);
  return controls.review.holdOriginal
    ? new Promise(resolve => { controls.review.resolveOriginal = () => resolve(pixel); })
    : Promise.resolve(controls.review.largeOriginal ? largeOriginal : pixel);
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
api.providers.probeSearch = async (id, model) => {
  controls.review.searchProbe = { id, model };
  return { model, protocol: "chatCompletions", suggestion: "openAi", attempts: [
    { format: "mimoChat", outcome: "rejected", status: 400, sources: 0, detail: "Param Incorrect" },
    { format: "openAiChat", outcome: "executed", status: 200, sources: 2, detail: "Rust 1.99 shipped." },
  ] };
};

function ProviderFixture({ mixed = false }: { mixed?: boolean }) {
  const [dialog, setDialog] = useState({ open: true, provider: controls.review.dialogProviderFixture ?? (mixed ? mixedProvider : provider("A")) });
  return <><button id="open-b" onClick={() => setDialog({ open: true, provider: provider("B") })}>Open B</button>
    <button id="reopen-saved" onClick={() => setDialog(current => ({ open: true, provider: { ...current.provider, ...controls.review.saved } }))}>Reopen saved</button>
    <ProviderDialog open={dialog.open} provider={dialog.provider}
      onOpenChange={open => setDialog(current => ({ ...current, open }))} onSaved={noop} /></>;
}

type FixtureMode = "sidebar" | "instructions" | "mcp-clients" | "skills" | "desktop-updater" | "desktop-exit" | "terminal-proxy" | "autostart" | "memory" | "maintenance" | "settings" | "routing-cache" | "thread-details" | "thread-details-missing" | "thread-details-unknown" | "thread-details-error" | "compact" | "grid" | "image" | "mcp" | "accounts" | "account-resets" | "account-resets-unavailable" | "thread-tree" | "groups" | "overview" | "usage" | "usage-page" | "provider" | "warnings" | "thread-actions" | "thread-queue-recovery" | "mixed" | "access" | "incompatible" | "native-invalid" | "test" | "clients" | "clients-existing" | "clients-conflict" | "clients-opencode" | "clients-opencode-stale" | "clients-versions" | "clients-versions-existing" | "recovery-missing" | "recovery-journal" | "recovery-official" | "recovery-long" | "recovery-router";

// The app shell's sidebar beside an inset page, portalled out of PageStage so
// the capsule is fixed to the viewport as in the app.
function SidebarFixture() {
  const [route, setRoute] = useState<Route>(controls.review.sidebarRoute ?? "relays");
  return createPortal(<AppAvatarProvider>
    <SidebarProvider className="fixed inset-0 overflow-hidden"
      style={{ zIndex: 50, "--sidebar-width": `${SIDEBAR_EXPANDED_WIDTH_PX}px`, "--sidebar-width-icon": `${SIDEBAR_COLLAPSED_WIDTH_PX}px` } as React.CSSProperties}>
      <AppSidebar activeRoute={route} onNavigate={next => { controls.review.navigatedTo = next; setRoute(next); }} onThemeChange={noop} />
      <SidebarInset data-sidebar-fixture-page>
        <div style={{ display: "flex", alignItems: "center", gap: 12, padding: 18 }}><SidebarTrigger /><strong>{route}</strong></div>
      </SidebarInset>
    </SidebarProvider>
  </AppAvatarProvider>, document.body);
}

function RoutingCacheFixture() {
  const [page, setPage] = useState("threads");
  return <>
    <button onClick={() => setPage("connections")}>Review connections</button>
    <button onClick={() => setPage("threads")}>Review threads</button>
    {page === "threads" ? <ThreadsPage /> : <NewApiPage />}
  </>;
}

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
/** The terminal proxy row, asked to scroll into view the way a direct client's link asks. */
function TerminalProxyFixture() {
  const [focus, setFocus] = useState(!!controls.review.terminalFocusRequested);
  const handled = useCallback(() => {
    controls.review.terminalFocusHandled = (controls.review.terminalFocusHandled ?? 0) + 1;
    setFocus(false);
  }, []);
  return <TerminalProxySettings currentProxy={{ mode: "manual", url: "http://127.0.0.1:7890" }} focusRequested={focus} onFocusHandled={handled} />;
}

function mount(mode: FixtureMode) {
  controls.review.routingCalls = [];
  if (!controls.review.keepCursor) {
    delete controls.review.cursor;
    delete controls.review.cursorModels;
    controls.review.cursorCalls = [];
  }
  localStorage.removeItem("agentharbor.mcp-targets");
  controls.review.mcpLibrary = mcpSeed();
  controls.review.mcpCalls = [];
  controls.review.mcpHold = false;
  if (!controls.review.keepAccountsTab) localStorage.removeItem("agentharbor.accounts-tab.v1");
  localStorage.removeItem("agentharbor.instruction-source");
  // The injection console opens on Codex unless a suite asks for another client.
  if (controls.review.injectClient) localStorage.setItem("agentharbor.inject-client.v1", controls.review.injectClient);
  else localStorage.removeItem("agentharbor.inject-client.v1");
  if (mode === "instructions") {
    for (const source of ["codex", "claudeCode"]) { localStorage.removeItem(`agentharbor.instruction-draft.${source}`); localStorage.removeItem(`agentharbor.instruction-presets.v1.${source}`); }
    controls.review.instructionCalls = [];
    const instructionFile = (content: string) => `User rules\n<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_START -->\n${content}\n<!-- AGENT_HARBOR_CUSTOM_INSTRUCTIONS_END -->\n`;
    controls.review.instructionStates = Object.fromEntries(["codex", "claudeCode"].map(source => [source, { current: { globalPath: source === "codex" ? "C:/synthetic/.codex/AGENTS.md" : "C:/synthetic/.claude/CLAUDE.md", fileExists: true, managedBlockPresent: true, protectionState: "ready", issueMessage: null, managedContent: `${source} rules`, fileContent: instructionFile(`${source} rules`), lastAppliedAt: null, lastTemplateCode: null, lastTemplateTitle: null }, history: [] }]));
    api.loadCustomInstructionState = async (source = "codex") => { controls.review.instructionCalls.push({ operation: "load", source }); if (controls.review.instructionLoadFailure === source) throw new Error("Synthetic read failure"); return { data: controls.review.instructionStates[source] } as any; };
    api.previewCustomInstructionApply = async (content, source = "codex") => { controls.review.instructionCalls.push({ operation: "preview", source, content }); const current = controls.review.instructionStates[source].current; return { data: { globalPath: current.globalPath, currentManagedContent: current.managedContent, nextManagedContent: content, resultingContent: `User rules\n${content}\n`, protectionState: "ready" } } as any; };
    api.applyCustomInstruction = async (params, source = "codex") => { controls.review.instructionCalls.push({ operation: "apply", target: source, ...params }); if (controls.review.instructionApplyHold) await new Promise(resolve => { controls.review.instructionApplyResolve = resolve; }); const state = controls.review.instructionStates[source]; state.history.unshift({ id: `entry-${state.history.length}`, action: "apply", source: "manual", createdAt: 1700000000 }); state.current.managedContent = params.content; state.current.fileContent = instructionFile(params.content); return { data: structuredClone(state) } as any; };
    api.clearCustomInstructionBlock = async (source = "codex") => { controls.review.instructionCalls.push({ operation: "clear", source }); const state = controls.review.instructionStates[source]; state.current.managedContent = ""; state.current.fileContent = "User rules\n"; state.current.managedBlockPresent = false; return { data: structuredClone(state) } as any; };
    api.rollbackCustomInstruction = async (id, source = "codex") => { controls.review.instructionCalls.push({ operation: "rollback", source, id }); return { data: structuredClone(controls.review.instructionStates[source]) } as any; };
  }
  localStorage.removeItem("threads_source");
  controls.review.skillState = skillSeed(mode);
  controls.review.skillCalls = [];
  controls.review.skillHold = false;
  controls.review.skillReads = 0;
  delete controls.review.pickedFolder;
  localStorage.removeItem("agentharbor.skill-targets");
  controls.review.conversationCalls = [];
  controls.review.conversationListResolvers = [];
  if (mode === "terminal-proxy") {
    controls.review.terminalSetCalls = [];
    controls.review.terminalTestCalls = [];
    controls.review.terminalTestResolvers = [];
    controls.review.terminalLoadResolvers = [];
    controls.review.terminalOpenCalls = 0;
    controls.review.terminalStatus = { supported: true, enabled: false, applied: false,
      url: null, noProxy: "localhost,127.0.0.1,::1", recoveryRequired: false, conflicts: [],
      preservedVariables: [], notificationSent: true, ...controls.review.terminalStatusPatch };
    api.terminalProxy.load = async () => {
      if (controls.review.terminalLoadFailure) throw "settings.terminalProxy.errors.environment";
      const snapshot = structuredClone(controls.review.terminalStatus);
      if (controls.review.terminalLoadHold) return new Promise(resolve => controls.review.terminalLoadResolvers.push(() => resolve(snapshot)));
      return snapshot;
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
  } else {
    // Direct clients show the terminal proxy's state.
    api.terminalProxy.load = async () => ({ supported: true, enabled: false, applied: false,
      url: null, noProxy: "localhost,127.0.0.1,::1", recoveryRequired: false, conflicts: [],
      preservedVariables: [], notificationSent: false, ...controls.review.terminalStatusPatch });
  }
  if (mode === "autostart") {
    controls.review.autostartCalls = [];
    controls.review.autostartStatus = { supported: true, enabled: false, blocked: false, ...controls.review.autostartPatch };
    api.autostart.status = async () => structuredClone(controls.review.autostartStatus);
    api.autostart.set = async enabled => {
      controls.review.autostartCalls.push(enabled);
      if (controls.review.autostartFailure) throw "settings.autostart.errors.write";
      controls.review.autostartStatus = { ...controls.review.autostartStatus, enabled, blocked: false };
      return structuredClone(controls.review.autostartStatus);
    };
  }
  if (mode === "memory" || mode === "maintenance") {
    // Synthetic memory lists; nothing asks Windows for consent.
    const gib = 1024 ** 3;
    const snapshot = (patch = {}) => ({ total: 16 * gib, inUse: 9 * gib, modified: gib / 4, standby: 5 * gib, free: 1.75 * gib, ...patch });
    controls.review.memoryCalls = { optimize: [], areas: [] };
    controls.review.memoryStatus = { supported: true, elevated: false, memory: snapshot(), ...controls.review.memoryPatch };
    controls.review.memoryAreas = ["workingSets", "systemFileCache", "modifiedFileCache", "lowPriorityStandbyList", "registryCache", "combinePages"];
    api.memory.status = async () => structuredClone(controls.review.memoryStatus);
    api.memory.areas = async () => structuredClone(controls.review.memoryAreas);
    api.memory.setAreas = async areas => {
      controls.review.memoryCalls.areas.push(areas);
      controls.review.memoryAreas = areas;
      return structuredClone(areas);
    };
    api.memory.optimize = async areas => {
      controls.review.memoryCalls.optimize.push(areas);
      if (controls.review.memoryHold) await new Promise(resolve => { controls.review.memoryResolve = resolve; });
      if (controls.review.memoryFailure) throw controls.review.memoryFailure;
      return { before: snapshot(), after: snapshot({ inUse: 7 * gib, standby: 3 * gib, free: 5.75 * gib }), failed: controls.review.memoryFailed ?? [] };
    };
    api.memory.onOptimized = async callback => {
      controls.review.emitMemoryOptimized = (outcome: unknown) => callback(outcome as any);
      return () => { delete controls.review.emitMemoryOptimized; };
    };
  }
  api.codexMenuOverflow = async () => structuredClone(controls.review.menuOverflow ?? { total: 12, limit: 100, cut: [] });
  installAccountSources();
  installGateway();
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
  state = { providers: mode === "image" ? [imageProvider, studioImageProvider] : mode === "groups" ? groupedProviders : mode === "overview" ? [...groupedProviders, provider("extra-1"), provider("extra-2")] : mode === "usage" ? [{ ...groupedProviders[0], balance: { remaining: 12.5, used: 7.5, unit: "USD", rawQuota: 12.5, quotaPerUnit: null, requestCount: 5, checkedAt: Math.floor(Date.now() / 1000), conversionAvailable: true } }] : [selectedProvider], router: { enabled: false, accessMode: mode === "native-invalid" ? "native" : "router", accountless: false, port: 43127, configDrifted: false }, proxyRunning: false, activeProviderId: mode === "image" || mode === "groups" || mode === "overview" || mode === "usage" ? null : selectedProvider.id };
  if (controls.review.providerFixtures) state.providers = controls.review.providerFixtures;
  if (controls.review.routerPatch) state.router = { ...state.router, ...controls.review.routerPatch };
  if (controls.review.statePatch) state = { ...state, ...controls.review.statePatch };
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
    Object.assign(clientStatuses[0], { conflict: false, conflictFields: [], stale: false });
    return { path: "C:/synthetic/config.json", backupPath: "C:/synthetic/config.backup" };
  };
  api.targets.releaseConfig = async () => { controls.review.releasedClientCount += 1; };
  controls.review.officialPreviewCalls = [];
  controls.review.officialSwitchCalls = [];
  controls.review.officialPreviewResolvers = [];
  api.targets.previewClaudeOfficial = async () => {
    const revision = `official-${controls.review.officialPreviewCalls.length + 1}`;
    controls.review.officialPreviewCalls.push(revision);
    const preview = {
      path: "C:/synthetic/.claude/settings.json", revision,
      removedFields: controls.review.officialNoChanges || controls.review.officialSessionOnly ? [] : ["env.ANTHROPIC_BASE_URL", "env.ANTHROPIC_AUTH_TOKEN", "model"],
      warnings: controls.review.officialWarnings ?? [],
      hasChanges: !controls.review.officialNoChanges,
    };
    if (controls.review.officialPreviewHold) await new Promise<void>(resolve => controls.review.officialPreviewResolvers.push(resolve));
    if (controls.review.officialPreviewFailure) throw "clients.errors.read";
    return preview;
  };
  api.targets.switchClaudeOfficial = async revision => {
    controls.review.officialSwitchCalls.push(revision);
    if (controls.review.officialApplyHold) await new Promise<void>(resolve => { controls.review.officialApplyResolve = resolve; });
    if (controls.review.officialApplyFailure) throw "clients.errors.storage";
    if (controls.review.officialStale) {
      controls.review.officialStale = false;
      throw "clients.errors.changed";
    }
    const claude = clientStatuses.find(client => client.targetId === "claudeCode");
    if (claude) Object.assign(claude, { configured: false, conflict: false, conflictFields: [], providerId: null, model: null, request: null });
    return { path: "C:/synthetic/.claude/settings.json", backupPath: "C:/synthetic/config.backup", warnings: controls.review.officialWarnings ?? [] };
  };
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
      const project = index < 90 ? "C:/work/Lighthouse" : index < 95 ? "D:/work/Lighthouse" : index < 99 ? "C:/research" : "";
      Object.assign(thread, { cwd: project, folderName: project.split("/").at(-1) ?? "", updatedAt: 100 - index, archived: index % 2 === 0 });
    });
    controls.review.threadItems = threads;
  }
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
  if (mode === "thread-tree") api.conversations.delete = async (target, keys) => { controls.review.conversationCalls.push({ operation: "delete", target, keys }); if (target === "claudeCode") controls.review.claudeThreads = controls.review.claudeThreads.filter((item: any) => !keys.includes(item.key)); return { data: { deleted: keys.length, queued: 0, catalogRowsDeleted: 0, rolloutFilesDeleted: keys.length, backupPath: "synthetic-backup" } } as any; };
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
      configuredProvider: controls.review.configuredProvider ?? null,
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
      byModel: [{ label: target === "claudeCode" ? "claude-fixture-model" : "relay_2d66f8bc24::gpt-6-astra", usage: { ...usage, total: 27401900 } }, { label: "gpt-6.1-sol", usage: { ...usage, total: 5900000 } }],
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
      {(mode.startsWith("thread-details") || mode === "thread-tree" || mode === "thread-actions" || mode === "terminal-proxy" || mode === "autostart" || mode === "skills" || mode === "groups" || mode === "access" || mode === "accounts" || mode === "image") && <ThreadToastProbe />}
      {mode === "sidebar" ? <SidebarFixture /> : mode === "routing-cache" ? <RoutingCacheFixture /> : mode === "desktop-updater" ? <DesktopUpdaterFixture /> : mode === "desktop-exit" ? <ClaudeDesktopRouterExitDialog /> : mode.startsWith("recovery-") ? <RecoveryFixture thread={threads[0] as ThreadSummary} /> :
        mode === "skills" ? <SkillsPage /> : mode === "terminal-proxy" ? <div className="mx-auto max-w-xl rounded-xl bg-card shadow-sm"><TerminalProxyFixture /></div> :
        mode === "autostart" ? <div className="mx-auto max-w-xl rounded-xl bg-card shadow-sm"><AutostartSetting /></div> :
        mode === "memory" ? <div className="mx-auto grid max-w-3xl gap-4 sm:grid-cols-2"><MemoryOptimizer /></div> :
        mode === "maintenance" ? <MaintenancePage /> :
        mode === "settings" ? <AppAvatarProvider><CustomBackgroundProvider><SettingsPage theme="light" onThemeChange={noop} accent={"default" as any} setAccent={noop} language="zh" setLanguage={noop}
          refreshInterval="5m" setRefreshInterval={noop} onCheckUpdate={async () => "up-to-date"} /></CustomBackgroundProvider></AppAvatarProvider> :
        mode === "image" ? <ImageWorkbenchPage /> : mode === "instructions" ? <CustomInstructionsPage /> : mode === "mcp-clients" ? <McpClientFixture /> : mode === "mcp" ? <McpPage /> : mode === "accounts" || mode.startsWith("account-resets") ? <AccountsPage refreshInterval="5m" /> : mode === "overview" ? <OverviewPage onNavigate={route => { controls.review.navigatedTo = route; }} onOpenProxySettings={() => { controls.review.proxySettingsOpened = true; }} /> : mode === "usage-page" || mode === "usage" ? <UsagePage refreshInterval="5m" onManageProviders={() => { controls.review.navigatedTo = "relays"; }} onOpenThreads={client => { controls.review.navigatedTo = `threads:${client}`; }} /> : mode === "provider" || mode === "mixed" ? <ProviderFixture mixed={mode === "mixed"} /> : mode.startsWith("thread-details") || mode === "thread-tree" || mode === "warnings" || mode === "thread-actions" || mode === "thread-queue-recovery" ? <ThreadsPage /> :
        mode === "access" || mode === "incompatible" || mode === "native-invalid" || mode === "groups" ? <><NewApiPage onOpenRelays={() => { controls.review.navigatedTo = "relays"; }} /><RelayModelsPage /></> :
        mode === "test" ? <ProviderTestDialog open provider={mixedProvider} onOpenChange={noop} onTested={noop} onFailed={noop} /> :
        mode.startsWith("clients") ? <div className="space-y-8">{(["claudeCode", "opencode", "pi"] as const).filter(target => clientStatuses.some(status => status.targetId === target) || (!clientStatuses.length && target === (openCode ? "opencode" : "claudeCode"))).map(target => <ClientConfigs key={target} target={target} providers={controls.review.providerFixtures ? state.providers : openCode ? [mixedProvider, provider("alternate")] : [selectedProvider]} />)}</div> :
        <VirtualThreadList groups={groups} collapsedGroups={new Set()} selected={new Set()} viewMode={mode}
          onToggleSelect={noop} onToggleGroup={noop} onToggleCollapse={noop} onExport={noop}
          onOpenPath={noop} onRecover={noop} onDelete={noop} onCopyPath={noop} copiedGroupPath={null} t={key => key} />}
    </PageStage></div>
  </TooltipProvider></QueryClientProvider>);
}

controls.review = { mount, darkScrimFloor, usageEntry, t: (key: string, options?: Record<string, unknown>) => i18n.t(key, options),
  setLanguage: (language: string) => i18n.changeLanguage(language),
  emitBalanceUpdate: (id: string, remaining: number) => {
    state = { ...state, providers: state.providers.map(provider => provider.id === id && provider.balance
      ? { ...provider, balance: { ...provider.balance, remaining } } : provider) };
    for (const callback of controls.review.balanceUpdateListeners ?? []) callback();
  },
  realBalanceQuery, clientDefaults, claudeOneMDefaults, mergeModelCatalog, shrinkReference, errors: { errorKey, errorMessage, formatInvokeError }, getState: () => state, pixel, accessCalls: [], thumbnailCalls: [], thumbnailEdges: [], originalCalls: [], thumbnailResolvers: [],
  modelResolutionFixture, modelResolutionRequests: [], modelResolutionResolvers: [], modelResolutionRejectors: [], holdModelResolutions: false, modelResolutionPatches: {},
  navigatedTo: null, proxySettingsOpened: false,
  imageRequests: [], imageResolvers: [], imageCancels: {}, cancelledJobs: [], gallerySaves: 0, failGallerySave: false,
  promptLibrary: { groups: [] }, promptSaves: 0, failPromptSave: false,
  promptMaterials: { groups: [
    { id: "m-outfit", name: "服装", templates: [
      { id: "m1", title: "JK 制服", note: "日系校园", prompt: "水手服，百褶裙", pinned: false },
      { id: "m3", title: "老钱静奢", note: "", prompt: "米白真丝衬衫，驼色阔腿西裤", pinned: false },
    ] },
    { id: "m-pose", name: "动作", templates: [{ id: "m2", title: "回眸", note: "", prompt: "回头望向镜头", pinned: false }] },
    { id: "m-angle", name: "视角", templates: [] },
  ] },
  promptAssist: { settings: { model: "", instruction: "", skipReference: false, materials: [] }, defaultInstruction: "默认润色指令", models: [
    { id: "relay-a/glm-5", label: "GLM 5", sourceName: "Relay A", kind: "relay" },
    { id: "codex/gpt-5.5", label: "GPT-5.5", sourceName: "Codex", kind: "account" },
    { id: "smart", label: "Smart", sourceName: "Smart", kind: "aggregate" },
  ] }, assistRequests: [],
  imageMaterials: { directory: "C:/synthetic/image-materials", groups: [
    { id: "ig-people", name: "人物", items: [{ id: "img-girl", name: "短发女生", note: "只参考脸和发型", file: "img-girl.png" }] },
  ] },
  imageMaterialData: { "img-girl": galleryArt("gallery-7") },
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
    deleteThread: i18n.t("threads.deleteThread"),
    deleteSelected: i18n.t("threads.deleteSelected", { count: 1 }),
    clearDeletionQueue: i18n.t("threads.clearDeletionQueue"),
    delete: i18n.t("common.delete"),
    openGallery: i18n.t("image.openGallery"),
    openGalleryFailed: i18n.t("image.openGalleryFailed"),
    manageProfiles: i18n.t("clients.manageProfiles"),
    deleteProfile: i18n.t("clients.deleteProfile", { name: "Regression profile" }),
  } };
mount("compact");
