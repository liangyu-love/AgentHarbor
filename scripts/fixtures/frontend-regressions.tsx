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
import { ClientConfigs } from "@/components/newapi/client-configs";
import type { NewApiProvider, NewApiState } from "@/types/newapi";
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
const mixedProvider: NewApiProvider = { ...provider("mixed"), protocol: "chatCompletions", model: "responses-model", models: [
  { ...model("responses-model"), protocol: "responses" },
  { ...model("chat-model"), protocol: null, endpointTypes: ["chat_completions"] },
  { ...model("anthropic-model"), protocol: "anthropicMessages", endpointTypes: ["anthropic_messages"] },
  { ...model("inherited-model"), endpointTypes: [] },
] };
const galleryItems = Array.from({ length: 100 }, (_, i) => ({
  name: "gallery-" + i + ".png", path: "C:/synthetic/gallery-" + i + ".png", size: 100, modifiedAt: i,
}));
const pixel = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+nmQAAAABJRU5ErkJggg==";
let state: NewApiState = { providers: [imageProvider], router: { enabled: false, accessMode: "router", accountless: false, port: 0, configDrifted: false }, proxyRunning: false, activeProviderId: null };
api.providers.loadState = async () => state;
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
api.providers.test = async (id, selectedModel) => {
  controls.review.tested = { id, model: selectedModel };
  return { ...mixedProvider, testStatus: "ok" };
};
api.image.keyStatus = async () => false;
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

type FixtureMode = "compact" | "grid" | "image" | "provider" | "warnings" | "mixed" | "access" | "incompatible" | "native-invalid" | "test" | "clients" | "clients-existing" | "recovery-missing" | "recovery-journal" | "recovery-official";

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
  const selectedProvider = mode === "incompatible" || mode === "native-invalid" ? { ...mixedProvider, model: "chat-model" } : mixedProvider;
  state = { providers: mode === "image" ? [imageProvider] : [selectedProvider], router: { enabled: false, accessMode: mode === "native-invalid" ? "native" : "router", accountless: false, port: 43127, configDrifted: false }, proxyRunning: false, activeProviderId: mode === "image" ? null : selectedProvider.id };
  if (mode === "incompatible") state.providers.push(provider("alternate"));
  api.targets.configs = async () => mode === "clients" || mode === "clients-existing" ? [{ targetId: "claudeCode", name: "Claude Code", path: "C:/synthetic/settings.json", protocols: ["anthropicMessages"], configured: mode === "clients-existing", conflict: false, error: null, providerId: mode === "clients-existing" ? "mixed" : null, model: mode === "clients-existing" ? "responses-model" : null }] : [];
  const threads = Array.from({ length: 100 }, (_, i) => ({ key: "t" + i, id: "t" + i,
    cwd: "C:/synthetic", folderName: "synthetic", title: "Review thread " + i,
    createdAt: 0, updatedAt: 0, userMessages: 1, agentMessages: 1, fileSize: 100,
    archived: false, source: "", cliVersion: "", filePath: "C:/synthetic/t" + i }));
  if (mode.startsWith("recovery-")) {
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
    data: { total: 1, lastScanAt: 0, items: threads.slice(0, 1) } });
  const groups = Array.from({ length: 10 }, (_, i) => ({ key: "g" + i, cwd: "C:/synthetic/g" + i,
    folderName: "Workspace " + i, items: threads.slice(i * 10, i * 10 + 10), latestAt: 0,
    totalMessages: 20, totalSize: 1000, userMessages: 10, agentMessages: 10 }));
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  root.render(<QueryClientProvider client={queryClient}><TooltipProvider>
    <div style={{ position: "relative", height: 600 }}><PageStage state="active">
      {mode.startsWith("recovery-") ? <ThreadRecoveryDialog thread={threads[0] as any} onOpenChange={noop}
          onRecovered={() => { controls.review.recoveredCalls += 1; }} /> :
        mode === "image" ? <ImageWorkbenchPage /> : mode === "provider" || mode === "mixed" ? <ProviderFixture mixed={mode === "mixed"} /> : mode === "warnings" ? <ThreadsPage /> :
        mode === "access" || mode === "incompatible" || mode === "native-invalid" ? <NewApiPage /> :
        mode === "test" ? <ProviderTestDialog open provider={mixedProvider} onOpenChange={noop} onTested={noop} onFailed={noop} /> :
        mode === "clients" || mode === "clients-existing" ? <ClientConfigs providers={[mixedProvider]} /> :
        <VirtualThreadList groups={groups} collapsedGroups={new Set()} selected={new Set()} viewMode={mode}
          onToggleSelect={noop} onToggleGroup={noop} onToggleCollapse={noop} onExport={noop}
          onOpenPath={noop} onCopyPath={noop} copiedGroupPath={null} t={key => key} />}
    </PageStage></div>
  </TooltipProvider></QueryClientProvider>);
}

controls.review = { mount, getState: () => state, accessCalls: [], thumbnailCalls: [], originalCalls: [], thumbnailResolvers: [],
  restartCalls: [], recoveryApplyCalls: 0, recoveredCalls: 0, openedPath: null,
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
    recoveryTitle: i18n.t("threads.recoveryTitle"), recoveryCatalogConfirm: i18n.t("threads.recoveryCatalogConfirm"),
    recoveryMissingStatus: i18n.t("threads.recoveryStatus.missingCatalog"),
    recoveryJournalStatus: i18n.t("threads.recoveryStatus.journalBlocked"),
    recoveryOfficialStatus: i18n.t("threads.recoveryStatus.officialAccountUnverified"),
    recoveryCoordinationTitle: i18n.t("threads.recoveryCoordinationTitle"),
    recoveryOpenDirectory: i18n.t("threads.recoveryOpenDirectory"),
  } };
mount("compact");
