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
import { OverviewPage } from "@/components/overview/overview-page";
import { TokenUsagePanel } from "@/components/accounts/accounts-page";
import { ClientConfigs } from "@/components/newapi/client-configs";
import type { NewApiProvider, NewApiState } from "@/types/newapi";
import type { TokenUsagePayload } from "@/types";
import type { ClientConfigRequest, ClientStatus } from "@/types/clients";
import { api } from "@/lib/api";
import i18n from "@/lib/i18n";

const controls = window as any;
controls.__TAURI_INTERNALS__ = {};
const root = createRoot(document.getElementById("root")!);
const noop = () => {};
const model = (id: string) => ({ id, displayName: id, contextWindow: null, maxOutputTokens: null,
  endpointTypes: ["responses"], included: true, metadataSource: "upstream" as const });
const provider = (id: string) => ({ id, name: "Provider " + id, model: "model-" + id,
  models: [model("model-" + id)], hasApiKey: true, baseUrl: "https://" + id + ".invalid",
  enabled: true, protocol: "responses" as const, userId: null, hasAccessToken: false,
  contextWindow: null, autoCompactLimit: null, networkMode: "direct" as const,
  modelsFetchedAt: null, balance: null, balanceSource: null, testStatus: "untested" as const,
  testAt: null, createdAt: 0, updatedAt: 0 });
const imageProvider = { ...provider("image"), model: "gpt-image-2", models: [model("gpt-image-2")] };
const groupedProviders: NewApiProvider[] = [
  { ...provider("group-a"), name: "Relay Alpha", baseUrl: "https://relay.invalid/v1", model: "alpha-model" },
  { ...provider("group-b"), name: "Relay Beta", baseUrl: "https://relay.invalid/v1/", model: "beta-model" },
  { ...provider("group-c"), name: "Relay Gamma", baseUrl: "https://relay.invalid/v1", model: "gamma-model" },
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
    byProvider: [{ providerId: "group-a", providerName: "Relay Alpha", totals: relayTotals(330_000, 310_000, 20_000, 5) }],
    byModel: [{ providerId: "group-a", providerName: "Relay Alpha", model: "gpt-6-luna", totals: relayTotals(330_000, 310_000, 20_000, 5) }],
    daily: Array.from({ length: 14 }, (_, index) => ({ date: `2026-09-${String(index + 14).padStart(2, "0")}`, totals: relayTotals(index === 13 ? 63_000 : index * 1_200, index === 13 ? 62_988 : index * 1_000, index === 13 ? 12 : index * 200, index === 13 ? 1 : 0) })),
    recent: [{ timestamp: Math.floor(Date.now() / 1000), date: "2026-09-27", providerId: "group-a", providerName: "Relay Alpha", model: "gpt-6-luna", protocol: "responses", statusCode: 200, durationMs: 1140, firstTokenMs: 320, success: true, usageReported: true, inputTokens: 62_988, outputTokens: 12, reasoningTokens: 0, totalTokens: 63_000 }],
  },
};
let state: NewApiState = { providers: [imageProvider], router: { enabled: false, accessMode: "router", accountless: false, port: 0, configDrifted: false }, proxyRunning: false, activeProviderId: null };
api.providers.loadState = async () => state;
api.loadSnapshot = async () => ({ data: { status: {
  paths: { codexHome: "C:/synthetic/codex", authExists: true, registryExists: true, sessionsExists: true },
  api: { proxy: { mode: "manual", url: "http://127.0.0.1:7890" } },
  autoSwitch: { supported: true, enabled: false, threshold5hPercent: 15, thresholdWeeklyPercent: 10, serviceState: "stopped", serviceLabel: "" },
} } }) as any;
api.loadMcpServers = async () => ({ data: { items: Array.from({ length: 6 }, (_, i) => ({ id: "mcp-" + i, enabled: true })) } }) as any;
api.loadInstalledSkills = async () => ({ data: { total: 36, items: [] } }) as any;
api.loadSkillBackups = async () => ({ data: { total: 0, items: [] } }) as any;
api.loadCustomInstructionState = async () => ({ data: { current: { managedBlockPresent: false, lastTemplateTitle: null } } }) as any;
api.getSystemInfo = async () => ({ os: "Windows", arch: "x64" }) as any;
api.providers.queryBalance = id => new Promise(resolve => {
  controls.review.balanceRequests.push(id);
  controls.review.balanceResolvers.push(() => resolve({ ...mixedProvider, id, balance: { remaining: 9, used: null, unit: "USD", rawQuota: 9,
    quotaPerUnit: null, requestCount: null, checkedAt: Date.now(), conversionAvailable: true }, balanceSource: "newapi" }));
});
api.routing.setAccessMode = async mode => {
  controls.review.accessCalls.push(mode);
  state = { ...state, router: { ...state.router, accessMode: mode } };
  return { state, applied: null };
};
api.routing.setEnabled = async enabled => {
  state = { ...state, router: { ...state.router, enabled } };
  return { state, applied: null };
};
api.targets.restart = async target => {
  controls.review.restartCalls.push(target);
};
api.targets.configs = async () => [];
api.targets.previewConfig = async request => {
  controls.review.previewed = request;
  return { path: "C:/synthetic/config.json", revision: "mock-revision", content: JSON.stringify({ model: request.model }) };
};
api.targets.applyConfig = async (request, revision) => {
  controls.review.appliedClient = { request, revision };
  return { path: "C:/synthetic/config.json", backupPath: "C:/synthetic/config.backup" };
};
api.providers.test = async (id, selectedModel) => {
  controls.review.tested = { id, model: selectedModel };
  return { ...mixedProvider, testStatus: "ok" };
};
api.image.keyStatus = async () => false;
api.image.generate = request => new Promise(resolve => {
  controls.review.imageRequests.push(request);
  controls.review.imageResolvers.push((images = [pixel]) => resolve({ images, model: request.model, revisedPrompts: [] }));
});
api.image.saveToGallery = async () => {
  controls.review.gallerySaves += 1;
  if (controls.review.failGallerySave) throw new Error("image.errors.save");
  return { directory: "C:/synthetic", files: ["C:/synthetic/generated.png"] };
};
api.image.listGallery = async () => ({ directory: "C:/synthetic", items: galleryItems });
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
api.providers.save = async input => {
  controls.review.saved = input;
  return { state, applied: null };
};

function ProviderFixture({ mixed = false }: { mixed?: boolean }) {
  const [dialog, setDialog] = useState({ open: true, provider: mixed ? mixedProvider : provider("A") });
  return <><button id="open-b" onClick={() => setDialog({ open: true, provider: provider("B") })}>Open B</button>
    <ProviderDialog open={dialog.open} provider={dialog.provider}
      onOpenChange={open => setDialog(current => ({ ...current, open }))} onSaved={noop} /></>;
}

type FixtureMode = "compact" | "grid" | "image" | "groups" | "overview" | "usage" | "provider" | "warnings" | "thread-actions" | "mixed" | "access" | "incompatible" | "native-invalid" | "test" | "clients" | "clients-existing" | "clients-opencode" | "clients-versions" | "clients-versions-existing" | "recovery-missing" | "recovery-journal" | "recovery-official";

function recoveryPreview(mode: FixtureMode) {
  const base = {
    threadId: "t0", revision: "recovery-revision", configuredProvider: "agentharbor_native",
    targetProvider: "agentharbor_native", databaseProvider: "openai", rolloutProvider: "openai",
    rolloutPath: "C:/synthetic/rollout-t0.jsonl", catalogAnalysis: null, catalogFields: [],
    coordination: null, status: "providerMismatch", canApply: true,
    reason: "Synthetic recovery preview", changes: [{ target: "database", path: "C:/synthetic/state.sqlite",
      fromProvider: "openai", toProvider: "agentharbor_native" }],
  };
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

function mount(mode: FixtureMode) {
  const selectedProvider = mode.startsWith("clients-versions") ? versionedProvider
    : mode === "incompatible" || mode === "native-invalid" ? { ...mixedProvider, model: "chat-model" } : mixedProvider;
  state = { providers: mode === "image" ? [imageProvider] : mode === "groups" ? groupedProviders : mode === "overview" ? [...groupedProviders, provider("extra-1"), provider("extra-2")] : mode === "usage" ? [{ ...groupedProviders[0], balance: { remaining: 12.5, used: 7.5, unit: "USD", rawQuota: 12.5, quotaPerUnit: null, requestCount: 5, checkedAt: Math.floor(Date.now() / 1000), conversionAvailable: true } }] : [selectedProvider], router: { enabled: false, accessMode: mode === "native-invalid" ? "native" : "router", accountless: false, port: 43127, configDrifted: false }, proxyRunning: false, activeProviderId: mode === "image" || mode === "groups" || mode === "overview" || mode === "usage" ? null : selectedProvider.id };
  if (mode === "incompatible") state.providers.push(provider("alternate"));
  const savedClient: ClientConfigRequest | null = mode === "clients-versions-existing" ? {
    targetId: "claudeCode", providerId: "versions", model: "claude-opus-4-8", contextWindow: 200000,
    maxOutputTokens: 8192, accessMode: "native", providerIds: ["versions"], authField: "authToken",
    claudeModels: { sonnet: "", opus: "claude-opus-4-8", fable: "", haiku: "", subagent: "" },
    claudeModelIds: ["claude-opus-5-5", "claude-opus-4-8"],
  } : mode === "clients-existing" ? {
    targetId: "claudeCode", providerId: "mixed", model: "responses-model", contextWindow: 128000,
    maxOutputTokens: 8192, accessMode: "router", providerIds: ["mixed"], authField: "apiKey",
    claudeModels: { sonnet: "chat-model", opus: "", fable: "", haiku: "", subagent: "" },
  } : null;
  const openCode = mode === "clients-opencode";
  const clientStatuses: ClientStatus[] = mode.startsWith("clients") ? [{
    targetId: openCode ? "opencode" : "claudeCode", name: openCode ? "OpenCode" : "Claude Code",
    path: "C:/synthetic/config.json", protocols: openCode ? ["responses", "chatCompletions", "anthropicMessages"] : ["anthropicMessages"],
    configured: !!savedClient, conflict: false, error: null, providerId: savedClient?.providerId ?? null,
    model: savedClient?.model ?? null, request: savedClient,
  }] : [];
  api.targets.configs = async () => clientStatuses;
  const threads = Array.from({ length: 100 }, (_, i) => ({ key: "t" + i, id: "t" + i,
    cwd: "C:/synthetic", folderName: "synthetic", title: "Review thread " + i,
    createdAt: 0, updatedAt: 0, userMessages: 1, agentMessages: 1, fileSize: 100,
    archived: false, source: "", cliVersion: "", filePath: "C:/synthetic/t" + i,
    modelProvider: "agentharbor_native", model: "gpt-6-sol", tokensUsed: 10,
    threadSource: "sqlite", rolloutStatus: "available" }));
  if (mode === "thread-actions") {
    controls.review.recoveryApplyCalls = 0;
    controls.review.deleteCalls = [];
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
      return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
        data: { deleted: keys.length, catalogRowsDeleted: keys.length, rolloutFilesDeleted: keys.length,
          backupPath: "C:/synthetic/thread-delete-backups/fixture" } };
    };
  } else if (mode.startsWith("recovery-")) {
    api.conversations.previewRecovery = async () => ({ schemaVersion: 1, success: true, code: "ok", message: "", warnings: [], data: recoveryPreview(mode) as any });
    api.conversations.applyRecovery = async () => {
      controls.review.recoveryApplyCalls += 1;
      const preview = recoveryPreview(mode) as any;
      return { schemaVersion: 1, success: true, code: "ok", message: "", warnings: [],
        data: { changed: true, backupPath: "C:/synthetic/recovery/backup.json", preview } };
    };
    api.openPath = async path => { controls.review.openedPath = path; };
  }
  api.conversations.list = async () => ({ schemaVersion: 1, success: true, code: "ok", message: "",
    warnings: [{ code: "thread_scan_skipped", message: "Skipped synthetic-corrupt-session.jsonl: invalid UTF-8" }],
    data: { total: mode === "thread-actions" ? 2 : 1, lastScanAt: 0,
      items: threads.slice(0, mode === "thread-actions" ? 2 : 1) } });
  const groups = Array.from({ length: 10 }, (_, i) => ({ key: "g" + i, cwd: "C:/synthetic/g" + i,
    folderName: "Workspace " + i, items: threads.slice(i * 10, i * 10 + 10), latestAt: 0,
    totalMessages: 20, totalSize: 1000, userMessages: 10, agentMessages: 10 }));
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  root.render(<QueryClientProvider client={queryClient}><TooltipProvider>
    <div style={{ position: "relative", height: 600 }}><PageStage state="active">
      {mode.startsWith("recovery-") ? <ThreadRecoveryDialog thread={threads[0] as any} onOpenChange={noop}
          onRecovered={() => { controls.review.recoveredCalls += 1; }} /> :
        mode === "image" ? <ImageWorkbenchPage /> : mode === "overview" ? <OverviewPage onNavigate={route => { controls.review.navigatedTo = route; }} onOpenProxySettings={() => { controls.review.proxySettingsOpened = true; }} /> : mode === "usage" ? <TokenUsagePanel data={usageFixture} loading={false} error={null} onRetry={noop} onManageProviders={noop} /> : mode === "provider" || mode === "mixed" ? <ProviderFixture mixed={mode === "mixed"} /> : mode === "warnings" || mode === "thread-actions" ? <ThreadsPage /> :
        mode === "access" || mode === "incompatible" || mode === "native-invalid" || mode === "groups" ? <NewApiPage /> :
        mode === "test" ? <ProviderTestDialog open provider={mixedProvider} onOpenChange={noop} onTested={noop} onFailed={noop} /> :
        mode.startsWith("clients") ? <ClientConfigs providers={openCode ? [mixedProvider, provider("alternate")] : [selectedProvider]} /> :
        <VirtualThreadList groups={groups} collapsedGroups={new Set()} selected={new Set()} viewMode={mode}
          onToggleSelect={noop} onToggleGroup={noop} onToggleCollapse={noop} onExport={noop}
          onOpenPath={noop} onRecover={noop} onDelete={noop} onCopyPath={noop} copiedGroupPath={null} t={key => key} />}
    </PageStage></div>
  </TooltipProvider></QueryClientProvider>);
}

controls.review = { mount, getState: () => state, pixel, accessCalls: [], thumbnailCalls: [], originalCalls: [], thumbnailResolvers: [],
  navigatedTo: null, proxySettingsOpened: false,
  imageRequests: [], imageResolvers: [], gallerySaves: 0, failGallerySave: false,
  balanceRequests: [], balanceResolvers: [],
  restartCalls: [], recoveryApplyCalls: 0, recoveredCalls: 0, deleteCalls: [], openedPath: null,
  holdThumbnails: false, holdOriginal: false, labels: {
    fetchModels: i18n.t("newapi.fetchModels"), cancel: i18n.t("common.cancel"),
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
    clientSelectAllModels: i18n.t("clients.selectAllModels"),
    recoveryTitle: i18n.t("threads.recoveryTitle"), recoveryCatalogConfirm: i18n.t("threads.recoveryCatalogConfirm"),
    recoveryMissingStatus: i18n.t("threads.recoveryStatus.missingCatalog"),
    recoveryJournalStatus: i18n.t("threads.recoveryStatus.journalBlocked"),
    recoveryOfficialStatus: i18n.t("threads.recoveryStatus.officialAccountUnverified"),
    recoveryCoordinationTitle: i18n.t("threads.recoveryCoordinationTitle"),
    recoveryOpenDirectory: i18n.t("threads.recoveryOpenDirectory"),
    repairAll: i18n.t("threads.repairAll"), deleteThread: i18n.t("threads.deleteThread"),
    deleteSelected: i18n.t("threads.deleteSelected", { count: 1 }),
  } };
mount("compact");
