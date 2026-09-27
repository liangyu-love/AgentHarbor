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
  UpdateInstallabilityPayload,
  DaemonRunPayload,
  DiagnosePayload,
  McpServerListPayload,
  McpServerMutationPayload,
  McpServerRemovePayload,
  SkillListPayload,
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
import { isTauriRuntime } from "@/lib/tauri-runtime";
import type { TargetId, ClientStatus, ClientConfigRequest, ClientConfigPreview, ClientConfigApplied } from "@/types/clients";
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
  save: (provider: NewApiProviderInput) => invoke<NewApiStateChange>("save_newapi_provider", { provider }),
  delete: (id: string) => invoke<NewApiStateChange>("delete_newapi_provider", { id }),
  setEnabled: (id: string, enabled: boolean) => invoke<NewApiStateChange>("set_newapi_provider_enabled", { id, enabled }),
  setNetwork: (id: string, mode: "proxy" | "direct") =>
    invoke<NewApiStateChange>("set_newapi_provider_network", { id, mode }),
  test: (id: string, model: string) => invoke<NewApiProvider>("test_newapi_provider", { id, model }),
  fetchModels: (providerId: string | null, baseUrl: string, apiKey: string, protocol: ModelProtocol) =>
    invoke<NewApiModel[]>("fetch_newapi_models", { providerId, baseUrl, apiKey, protocol }),
  queryBalance: (id: string) => invoke<NewApiProvider>("query_newapi_provider_balance", { id }),
  import: (path: string) => invoke<NewApiImportResult>("import_newapi_providers", { path }),
  export: (path: string) => invoke<void>("export_newapi_providers", { path }),
};

const routingApi = {
  setEnabled: (enabled: boolean) => invoke<NewApiStateChange>("set_newapi_smart_routing", { enabled }),
  setAccessMode: (mode: CodexAccessMode) => invoke<NewApiStateChange>("set_newapi_access_mode", { mode }),
  setAccountless: (enabled: boolean) => invoke<NewApiStateChange>("set_newapi_accountless", { enabled }),
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
  generate: (request: ImageGenerationRequest) =>
    invoke<ImageGenerationResponse>("generate_images", { request }),
  keyStatus: (baseUrl: string) => invoke<boolean>("image_key_status", { baseUrl }),
  saveKey: (baseUrl: string, apiKey: string) => invoke<void>("save_image_key", { baseUrl, apiKey }),
  saveFile: (path: string, dataUrl: string) => invoke<void>("save_image_file", { path, dataUrl }),
  saveToGallery: (dataUrls: string[]) =>
    invoke<SavedImagesPayload>("save_images_to_gallery", { dataUrls }),
  listGallery: () => invoke<GalleryListPayload>("list_gallery_images"),
  loadGalleryImage: (path: string) => invoke<string>("load_gallery_image", { path }),
  loadGalleryThumbnail: (path: string) => invoke<string>("load_gallery_thumbnail", { path }),
  deleteGalleryImage: (path: string) => invoke<void>("delete_gallery_image", { path }),
};

const targetsApi = {
  restart: (targetId: TargetId) => invoke<void>("restart_target", { targetId }),
  configs: () => invoke<ClientStatus[]>("list_client_configs"),
  previewConfig: (request: ClientConfigRequest) => invoke<ClientConfigPreview>("preview_client_config", { request }),
  applyConfig: (request: ClientConfigRequest, revision: string) => invoke<ClientConfigApplied>("apply_client_config", { request, revision }),
  restoreConfig: (targetId: TargetId) => invoke<ClientConfigApplied>("restore_client_config", { targetId }),
};

const conversationsApi = {
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
  };

const usageApi = {
  overview: () => invoke<TokenUsagePayload>("get_token_usage"),
};

const accountsApi = {
  list: () => invoke<CodexAccountListPayload>("list_codex_accounts"),
  refresh: () => invoke<CodexAccountListPayload>("refresh_codex_accounts"),
  login: () => invoke<CodexAccountListPayload>("login_codex_account"),
  switch: (accountKey: string) =>
    invoke<CodexAccountListPayload>("switch_codex_account", { accountKey }),
  delete: (accountKey: string) =>
    invoke<CodexAccountListPayload>("delete_codex_account", { accountKey }),
};

export const api = {
  providers: providersApi,
  routing: routingApi,
  image: imageApi,
  targets: targetsApi,
  conversations: conversationsApi,
  accounts: accountsApi,
  usage: usageApi,

  // Compatibility aliases for extensions and older frontend modules.
  loadNewApiState: providersApi.loadState,
  saveNewApiProvider: providersApi.save,
  deleteNewApiProvider: providersApi.delete,
  setNewApiProviderEnabled: providersApi.setEnabled,
  setNewApiProviderNetwork: providersApi.setNetwork,
  testNewApiProvider: providersApi.test,
  fetchNewApiModels: (providerId: string | null, baseUrl: string, apiKey: string) =>
    providersApi.fetchModels(providerId, baseUrl, apiKey, "responses"),
  queryNewApiProviderBalance: providersApi.queryBalance,
  setNewApiSmartRouting: routingApi.setEnabled,
  setNewApiAccountless: routingApi.setAccountless,
  importNewApiProviders: providersApi.import,
  exportNewApiProviders: providersApi.export,
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

  checkUpdateInstallability: () =>
    invoke<UpdateInstallabilityPayload>("check_update_installability"),

  runDaemonOnce: () =>
    invoke<CoreEnvelope<DaemonRunPayload>>("run_daemon_once"),

  diagnose: () =>
    invoke<CoreEnvelope<DiagnosePayload>>("diagnose"),

  restartCodex: () => targetsApi.restart("codex"),

  gracefulRestartForUpdate: () =>
    invoke<void>("graceful_restart_for_update"),

  loadMcpServers: () =>
    invoke<CoreEnvelope<McpServerListPayload>>("load_mcp_servers"),

  upsertMcpServer: (server: Omit<McpServerSummary, "sourcePath">) =>
    invoke<CoreEnvelope<McpServerMutationPayload>>("upsert_mcp_server", { ...server }),

  setMcpServerEnabled: (name: string, enabled: boolean) =>
    invoke<CoreEnvelope<McpServerMutationPayload>>("set_mcp_server_enabled", { name, enabled }),

  removeMcpServer: (name: string) =>
    invoke<CoreEnvelope<McpServerRemovePayload>>("remove_mcp_server", { name }),

  loadInstalledSkills: () =>
    invoke<CoreEnvelope<SkillListPayload>>("load_installed_skills"),

  loadSkillBackups: () =>
    invoke<CoreEnvelope<SkillBackupListPayload>>("load_skill_backups"),

  importSkill: (path: string) =>
    invoke<CoreEnvelope<SkillImportPayload>>("import_skill", { path }),

  removeSkill: (id: string) =>
    invoke<CoreEnvelope<SkillRemovePayload>>("remove_skill", { id }),

  restoreSkillBackup: (id: string) =>
    invoke<CoreEnvelope<SkillRestorePayload>>("restore_skill_backup", { id }),

  deleteSkillBackup: (id: string) =>
    invoke<CoreEnvelope<SkillDeleteBackupPayload>>("delete_skill_backup", { id }),

  loadCustomInstructionState: () =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("load_custom_instruction_state"),

  previewCustomInstructionApply: (content: string) =>
    invoke<CoreEnvelope<CustomInstructionPreviewPayload>>("preview_custom_instruction_apply", {
      content,
    }),

  applyCustomInstruction: (params: { content: string; templateCode?: string; templateTitle?: string; source?: string }) =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("apply_custom_instruction", { ...params }),

  clearCustomInstructionBlock: () =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("clear_custom_instruction_block"),

  rollbackCustomInstruction: (historyId: string) =>
    invoke<CoreEnvelope<CustomInstructionStatePayload>>("rollback_custom_instruction", { historyId }),

  isUpdaterConfigured: () => invoke<boolean>("is_updater_configured"),

  focusMainWindow: () =>
    invoke<void>("focus_main_window"),

  openPath: (path: string) =>
    invoke<void>("open_path", { path }),

  getSystemInfo: () =>
    invoke<{ os: string; osVersion: string; arch: string; hostname: string }>("get_system_info"),

  listCodexThreads: () => conversationsApi.list("codex"),

  exportCodexThreads: (keys: string[], targetDir: string, format: "markdown" | "jsonl") =>
    conversationsApi.export("codex", keys, targetDir, format),
};
