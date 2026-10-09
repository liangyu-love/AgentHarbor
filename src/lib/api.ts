import type {
  CoreEnvelope,
  CoreSnapshotPayload,
  McpServerSummary,
  CleanPayload,
  RebuildRegistryPayload,
  AutoSwitchConfigPayload,
  ApiProxyMode,
  ApiModePayload,
  ApiProxyDetectPayload,
  ApiProxyTestPayload,
  TerminalProxyStatus,
  TerminalProxyProbe,
  UpdateInstallabilityPayload,
  DiagnosePayload,
  McpServerListPayload,
  McpServerMutationPayload,
  McpServerRemovePayload,
  SkillListPayload,
  SkillSource,
  InstalledSkillSummary,
  SkillBackupListPayload,
  SkillImportPayload,
  SkillRemovePayload,
  SkillRestorePayload,
  SkillDeleteBackupPayload,
  CustomInstructionPreviewPayload,
  CustomInstructionStatePayload,
  CodexAccountListPayload,
  TokenUsagePayload,
} from "@/types";
import { claudeDesktopApi } from "@/lib/claude-desktop";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { errorMessage } from "@/lib/invoke-error";
import type { TargetId, ClientStatus, ClientConfigRequest, ClientConfigPreview, ClientConfigApplied, ClaudeOfficialPreview } from "@/types/clients";
import type { CodexAccessMode, ModelProtocol, NewApiState, NewApiStateChange, NewApiImportResult, NewApiProvider, NewApiProviderInput, NewApiModel } from "@/types/newapi";

export interface ImageGenerationRequest {
  providerId: string | null;
  baseUrl: string;
  apiKey: string;
  model: string;
  prompt: string;
  size: string;
  quality: string;
  count: number;
  imageData?: string | null;
}

export interface ImageGenerationResponse {
  images: string[];
  model: string;
  revisedPrompts: string[];
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauriRuntime()) {
    const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
    return tauriInvoke<T>(cmd, args);
  }
  throw new Error(`Command "${cmd}" is only available in Tauri runtime`);
}

export type { TargetId } from "@/types/clients";

const providersApi = {
  loadState: () => invoke<NewApiState>("load_newapi_state"),
  revealApiKey: (id: string) => invoke<string>("reveal_newapi_provider_api_key", { id }),
  save: (provider: NewApiProviderInput) => invoke<NewApiStateChange>("save_newapi_provider", { provider }),
  delete: (id: string) => invoke<NewApiStateChange>("delete_newapi_provider", { id }),
  setEnabled: (id: string, enabled: boolean) => invoke<NewApiStateChange>("set_newapi_provider_enabled", { id, enabled }),
  setNetwork: (id: string, mode: "proxy" | "direct") =>
    invoke<NewApiStateChange>("set_newapi_provider_network", { id, mode }),
  test: async (id: string, model: string, onProgress?: (event: import("@/types/newapi").ProviderProbeEvent) => void) => {
    const { Channel } = await import("@tauri-apps/api/core");
    const onEvent = new Channel<import("@/types/newapi").ProviderProbeEvent>();
    onEvent.onmessage = event => onProgress?.(event);
    return invoke<NewApiProvider>("test_newapi_provider", { id, model, onEvent });
  },
  saveBalanceScript: (id: string, expectedBaseUrl: string, script: string) =>
    invoke<NewApiProvider>("set_newapi_balance_script", { id, expectedBaseUrl, script }),
  setBalanceSharing: (id: string, expectedBaseUrl: string, shared: boolean) =>
    invoke<NewApiProvider>("set_newapi_balance_sharing", { id, expectedBaseUrl, shared }),
  setBalanceRefreshInterval: (id: string, expectedBaseUrl: string, intervalMinutes: number) =>
    invoke<NewApiProvider>("set_newapi_balance_refresh_interval", { id, expectedBaseUrl, intervalMinutes }),
  onBalanceUpdated: async (callback: () => void): Promise<() => void> => {
    const { listen } = await import("@tauri-apps/api/event");
    return listen("provider-balances-updated", callback);
  },
  fetchModels: (providerId: string | null, baseUrl: string, apiKey: string, protocol: ModelProtocol) =>
    invoke<NewApiModel[]>("fetch_newapi_models", { providerId, baseUrl, apiKey, protocol }),
  previewModelResolutions: (draft: import("@/types/newapi").ProviderModelResolutionDraft) =>
    invoke<import("@/types/newapi").ProviderModelResolutions>("preview_newapi_model_resolutions", { draft }),
  queryBalance: async (id: string): Promise<NewApiProvider> => {
    try {
      return await invoke<NewApiProvider>("query_newapi_provider_balance", { id });
    } catch (error) {
      // Retry transport failures once; auth, malformed data and unsupported sites are final.
      if (!["newapi.errors.network", "newapi.errors.timeout"].includes(errorMessage(error))) throw error;
      return invoke<NewApiProvider>("query_newapi_provider_balance", { id });
    }
  },
  saveBalanceCredentials: (id: string, expectedBaseUrl: string, userId: number | null, accessToken: string) =>
    invoke<NewApiProvider>("set_newapi_balance_credentials", { id, expectedBaseUrl, userId, accessToken }),
  import: (path: string) => invoke<NewApiImportResult>("import_newapi_providers", { path }),
  export: (path: string) => invoke<void>("export_newapi_providers", { path }),
};

const routingApi = {
  capabilities: () => invoke<import("@/types/newapi").ModelCapabilities>("load_model_capabilities"),
  previewCapabilityClear: (providerId: string, modelId: string, field: import("@/types/newapi").ClearCapabilityField) =>
    invoke<import("@/types/newapi").ClearCapabilityPreview>("preview_newapi_capability_clear", { providerId, modelId, field }),
  applyCapabilityClear: (preview: import("@/types/newapi").ClearCapabilityPreview) =>
    invoke<import("@/types/newapi").ClearCapabilityApplyResult>("apply_newapi_capability_clear", {
      providerId: preview.before.providerId, modelId: preview.before.upstreamModel,
      field: preview.field, configRevision: preview.configRevision,
      catalogRevision: preview.catalogRevision, officialRevision: preview.officialRevision,
    }),
  diagnostics: () => invoke<import("@/types/newapi").RouteDiagnostic[]>("load_route_diagnostics"),
  catalogStatus: () => invoke<import("@/types/newapi").ModelsDevCatalogStatus>("load_models_dev_status"),
  refreshCatalog: () =>
    invoke<import("@/types/newapi").ModelsDevCatalogStatus>("refresh_models_dev_catalog"),
  setWebSearchProvider: (providerId: string | null) =>
    invoke<NewApiStateChange>("set_router_web_search_provider", { providerId }),
  setCompression: (policy: import("@/types/newapi").CompressionPolicy) =>
    invoke<NewApiStateChange>("set_newapi_compression", { policy }),
  setModelPrice: (
    providerId: string,
    modelId: string,
    price: import("@/types/newapi").ModelPriceOverrides | null,
  ) => invoke<NewApiStateChange>("set_newapi_model_price", { providerId, modelId, price }),
  setEnabled: (enabled: boolean, force = false) => invoke<NewApiStateChange>("set_newapi_smart_routing", { enabled, force }),
  setAccessMode: (mode: CodexAccessMode, force = false) => invoke<NewApiStateChange>("set_newapi_access_mode", { mode, force }),
  subagents: () => invoke<import("@/types/newapi").CodexSubagentView>("load_codex_subagents"),
  saveSubagents: (settings: import("@/types/newapi").CodexSubagentSettings | null) =>
    invoke<NewApiStateChange>("save_codex_subagents", { settings }),
  setAccountless: (enabled: boolean) => invoke<NewApiStateChange>("set_newapi_accountless", { enabled }),
};

const aggregationApi = {
  state: () => invoke<import("@/types/aggregation").AggregationState>("load_aggregation_state"),
  save: (model: import("@/types/aggregation").AggregateModel) => invoke<import("@/types/aggregation").AggregationState>("save_aggregate_model", { model }),
  delete: (id: string) => invoke<import("@/types/aggregation").AggregationState>("delete_aggregate_model", { id }),
  setExternalEnabled: (enabled: boolean) => invoke<import("@/types/aggregation").AggregationState>("set_aggregation_external_enabled", { enabled }),
  selectCodex: (aggregateId: string | null) => invoke<NewApiStateChange>("select_codex_aggregate", { aggregateId }),
  exportConnection: (request: { ingressProtocol: ModelProtocol; aggregateIds: string[]; selectedAggregateId: string | null;
    consumerProfile: "generic" | "anthropicSdk" | "opencode"; revealCredential: boolean }) =>
    invoke<import("@/types/aggregation").ConnectionExportResult>("export_aggregation_connection", request),
};

export interface SavedImagesPayload {
  directory: string;
  files: string[];
}

export interface GalleryItem {
  name: string;
  path: string;
  size: number;
  modifiedAt: number;
}

export interface GalleryListPayload {
  directory: string;
  items: GalleryItem[];
  hasMore: boolean;
}

const imageApi = {
  generate: (request: ImageGenerationRequest, jobId?: string) =>
    invoke<ImageGenerationResponse>("generate_images", { request, jobId }),
  cancelGeneration: (jobId: string) => invoke<boolean>("cancel_image_generation", { jobId }),
  keyStatus: (baseUrl: string) => invoke<boolean>("image_key_status", { baseUrl }),
  saveKey: (baseUrl: string, apiKey: string) => invoke<void>("save_image_key", { baseUrl, apiKey }),
  keySites: () => invoke<string[]>("list_image_key_sites"),
  deleteKey: (baseUrl: string) => invoke<void>("delete_image_key", { baseUrl }),
  saveFile: (path: string, dataUrl: string) => invoke<void>("save_image_file", { path, dataUrl }),
  saveToGallery: (dataUrls: string[]) =>
    invoke<SavedImagesPayload>("save_images_to_gallery", { dataUrls }),
  setGalleryDirectory: (directory: string) =>
    invoke<string>("set_gallery_directory", { directory }),
  listGallery: () => invoke<GalleryListPayload>("list_gallery_images"),
  loadGalleryImage: (path: string) => invoke<string>("load_gallery_image", { path }),
  loadGalleryThumbnail: (path: string) => invoke<string>("load_gallery_thumbnail", { path }),
  deleteGalleryImage: (path: string) => invoke<void>("delete_gallery_image", { path }),
};

const targetsApi = {
  restart: (targetId: TargetId) => invoke<void>("restart_target", { targetId }),
  configs: () => invoke<ClientStatus[]>("list_client_configs"),
  previewConfig: (request: ClientConfigRequest, replaceConflicts = false) => invoke<ClientConfigPreview>("preview_client_config", { request, replaceConflicts }),
  applyConfig: (request: ClientConfigRequest, revision: string, replaceConflicts = false) => invoke<ClientConfigApplied>("apply_client_config", { request, revision, replaceConflicts }),
  restoreConfig: (targetId: TargetId, replaceConflicts = false) => invoke<ClientConfigApplied>("restore_client_config", { targetId, replaceConflicts }),
  previewClaudeOfficial: () => invoke<ClaudeOfficialPreview>("preview_claude_code_official_config"),
  switchClaudeOfficial: (revision: string) => invoke<ClientConfigApplied>("switch_claude_code_official_config", { revision }),
  releaseConfig: (targetId: TargetId) => invoke<void>("release_client_config", { targetId }),
  reapplyConfig: (targetId: TargetId, rebaseline: boolean) =>
    invoke<ClientConfigApplied>("reapply_client_config", { targetId, rebaseline }),
};

const conversationsApi = {
  details: (targetId: TargetId, key: string, threadId: string) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadDetails>>("get_conversation_details", { targetId, key, threadId }),
  messages: (targetId: TargetId, key: string, threadId: string, snapshotBytes: number, offset: number, snapshotId?: string) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadMessagePage>>("get_conversation_messages", { targetId, key, threadId, snapshotBytes, offset, snapshotId }),
  list: (targetId: TargetId) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadListPayload>>("list_conversations", { targetId }),
  export: (targetId: TargetId, keys: string[], targetDir: string, format: "markdown" | "jsonl") =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadExportPayload>>("export_conversations", {
      targetId,
      keys,
      targetDir,
      format,
      }),
  delete: (targetId: TargetId, keys: string[]) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadDeleteResult>>("delete_conversations", {
      targetId,
      keys,
    }),
  cancelPendingDeletions: (targetId: TargetId) =>
    invoke<CoreEnvelope<number>>("cancel_pending_conversation_deletions", { targetId }),
  previewRecovery: (threadId: string) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadRecoveryPreview>>(
      "preview_codex_thread_recovery",
      { threadId },
    ),
  applyRecovery: (threadId: string, expectedRevision: string) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadRecoveryResult>>(
      "apply_codex_thread_recovery",
      { threadId, expectedRevision },
    ),
  adoptRouter: (threadId: string, expectedRevision: string) =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadRecoveryResult>>(
      "adopt_codex_thread_router",
      { threadId, expectedRevision },
    ),
  routeOverview: () =>
    invoke<import("@/types/threads").ThreadRouteOverview>("codex_thread_route_overview"),
  previewBatch: (
    target: import("@/types/threads").ThreadBatchTarget,
    selection: { threadIds?: string[]; fromProvider?: string },
  ) =>
    invoke<import("@/types/threads").ThreadBatchPreview>("preview_codex_thread_batch", {
      threadIds: selection.threadIds ?? [],
      fromProvider: selection.fromProvider ?? null,
      target,
    }),
  applyBatch: (
    target: import("@/types/threads").ThreadBatchTarget,
    candidates: import("@/types/threads").ThreadBatchCandidate[],
  ) =>
    invoke<import("@/types/threads").ThreadBatchResult>("apply_codex_thread_batch", {
      candidates,
      target,
    }),
  };

const usageApi = {
  overview: () => invoke<TokenUsagePayload>("get_token_usage"),
};

const accountsApi = {
  list: () => invoke<CodexAccountListPayload>("list_codex_accounts"),
  refresh: () => invoke<CodexAccountListPayload>("refresh_codex_accounts"),
  login: (attemptId: string) => invoke<CodexAccountListPayload>("login_codex_account", { attemptId }),
  cancelLogin: (attemptId: string) => invoke<boolean>("cancel_codex_account_login", { attemptId }),
  switch: (accountKey: string) =>
    invoke<CodexAccountListPayload>("switch_codex_account", { accountKey }),
  delete: (accountKey: string) =>
    invoke<CodexAccountListPayload>("delete_codex_account", { accountKey }),
};

export const api = {
  aggregation: aggregationApi,
  providers: providersApi,
  routing: routingApi,
  claudeDesktop: claudeDesktopApi,
  image: imageApi,
  targets: targetsApi,
  conversations: conversationsApi,
  accounts: accountsApi,
  usage: usageApi,

  loadSnapshot: (localOnly = false) =>
    invoke<CoreEnvelope<CoreSnapshotPayload>>("load_snapshot", { localOnly }),

  clean: () =>
    invoke<CoreEnvelope<CleanPayload>>("clean"),

  rebuildRegistry: () =>
    invoke<CoreEnvelope<RebuildRegistryPayload>>("rebuild_registry"),

  setAutoSwitch: (enabled: boolean) =>
    invoke<CoreEnvelope<AutoSwitchConfigPayload>>("set_auto_switch", { enabled }),

  configureAutoSwitch: (threshold5hPercent?: number, thresholdWeeklyPercent?: number) =>
    invoke<CoreEnvelope<AutoSwitchConfigPayload>>("configure_auto_switch", {
      threshold5hPercent,
      thresholdWeeklyPercent,
    }),

  setApiProxyConfig: (mode: ApiProxyMode, url?: string) =>
    invoke<CoreEnvelope<ApiModePayload>>("set_api_proxy_config", { mode, url }),

  getUsageRefreshInterval: () =>
    invoke<string>("get_usage_refresh_interval"),

  setUsageRefreshInterval: (interval: string) =>
    invoke<string>("set_usage_refresh_interval", { interval }),

  testApiProxyConfig: (mode: ApiProxyMode, url?: string) =>
    invoke<CoreEnvelope<ApiProxyTestPayload>>("test_api_proxy_config", { mode, url }),

  detectApiProxyConfig: () =>
    invoke<CoreEnvelope<ApiProxyDetectPayload>>("detect_api_proxy_config"),

  terminalProxy: {
    load: () => invoke<TerminalProxyStatus>("load_terminal_proxy_config"),
    set: (enabled: boolean, url: string, noProxy: string) =>
      invoke<TerminalProxyStatus>("set_terminal_proxy_config", { enabled, url, noProxy }),
    test: (url: string) => invoke<TerminalProxyProbe[]>("test_terminal_proxy_config", { url }),
    openTerminal: () => invoke<void>("open_proxy_terminal"),
  },

  checkUpdateInstallability: () =>
    invoke<UpdateInstallabilityPayload>("check_update_installability"),

  diagnose: () =>
    invoke<CoreEnvelope<DiagnosePayload>>("diagnose"),

  loadMcpServers: (targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<McpServerListPayload>>("load_mcp_servers", { targetId }),

  upsertMcpServer: (server: Omit<McpServerSummary, "sourcePath">, targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<McpServerMutationPayload>>("upsert_mcp_server", { ...server, targetId }),

  setMcpServerEnabled: (name: string, enabled: boolean, targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<McpServerMutationPayload>>("set_mcp_server_enabled", { name, enabled, targetId }),

  removeMcpServer: (name: string, targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<McpServerRemovePayload>>("remove_mcp_server", { name, targetId }),

  loadInstalledSkills: (source?: SkillSource) =>
    invoke<CoreEnvelope<SkillListPayload>>("load_installed_skills", { source }),

  setSkillEnabled: (id: string, enabled: boolean, source?: SkillSource) =>
    invoke<CoreEnvelope<InstalledSkillSummary>>("set_skill_enabled", { id, enabled, source }),
  resolveSkillConflict: (id: string, keepEnabled: boolean, source?: SkillSource) =>
    invoke<CoreEnvelope<InstalledSkillSummary>>("resolve_skill_conflict", { id, keepEnabled, source }),

  loadSkillBackups: (source?: SkillSource) =>
    invoke<CoreEnvelope<SkillBackupListPayload>>("load_skill_backups", { source }),

  importSkill: (path: string, source?: SkillSource) =>
    invoke<CoreEnvelope<SkillImportPayload>>("import_skill", { path, source }),

  removeSkill: (id: string, source?: SkillSource) =>
    invoke<CoreEnvelope<SkillRemovePayload>>("remove_skill", { id, source }),

  restoreSkillBackup: (id: string, source?: SkillSource) =>
    invoke<CoreEnvelope<SkillRestorePayload>>("restore_skill_backup", { id, source }),

  deleteSkillBackup: (id: string, source?: SkillSource) =>
    invoke<CoreEnvelope<SkillDeleteBackupPayload>>("delete_skill_backup", { id, source }),

  loadCustomInstructionState: (targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("load_custom_instruction_state", { targetId }),

  previewCustomInstructionApply: (content: string, targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<CustomInstructionPreviewPayload>>("preview_custom_instruction_apply", {
      content, targetId,
    }),

  applyCustomInstruction: (params: { content: string; templateCode?: string; templateTitle?: string; source?: string; expectedContent?: string }, targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("apply_custom_instruction", { ...params, targetId }),

  clearCustomInstructionBlock: (targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("clear_custom_instruction_block", { targetId }),

  rollbackCustomInstruction: (historyId: string, targetId: "codex" | "claudeCode" = "codex") =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("rollback_custom_instruction", { historyId, targetId }),

  isUpdaterConfigured: () => invoke<boolean>("is_updater_configured"),

  openPath: (path: string) =>
    invoke<void>("open_path", { path }),

  getSystemInfo: () =>
    invoke<{ os: string; osVersion: string; arch: string; hostname: string }>("get_system_info"),
};
