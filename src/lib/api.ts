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
} from "@/types";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import type { NewApiState, NewApiStateChange, NewApiImportResult, NewApiProvider, NewApiProviderInput, NewApiModel } from "@/types/newapi";

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauriRuntime()) {
    const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
    return tauriInvoke<T>(cmd, args);
  }
  throw new Error(`Command "${cmd}" is only available in Tauri runtime`);
}

export const api = {
  loadNewApiState: () => invoke<NewApiState>("load_newapi_state"),
  saveNewApiProvider: (provider: NewApiProviderInput) => invoke<NewApiStateChange>("save_newapi_provider", { provider }),
  deleteNewApiProvider: (id: string) => invoke<NewApiStateChange>("delete_newapi_provider", { id }),
  setNewApiProviderEnabled: (id: string, enabled: boolean) => invoke<NewApiStateChange>("set_newapi_provider_enabled", { id, enabled }),
  setNewApiActiveProvider: (id: string) => invoke<NewApiStateChange>("set_newapi_active_provider", { id }),
  setNewApiDirect: (id: string) => invoke<NewApiStateChange>("set_newapi_direct", { id }),
  testNewApiProvider: (id: string) => invoke<NewApiProvider>("test_newapi_provider", { id }),
  fetchNewApiModels: (providerId: string | null, baseUrl: string, apiKey: string) =>
    invoke<NewApiModel[]>("fetch_newapi_models", { providerId, baseUrl, apiKey }),
  queryNewApiProviderBalance: (id: string) => invoke<NewApiProvider>("query_newapi_provider_balance", { id }),
  setNewApiSmartRouting: (enabled: boolean) => invoke<NewApiStateChange>("set_newapi_smart_routing", { enabled }),
  setNewApiAccountless: (enabled: boolean) => invoke<NewApiStateChange>("set_newapi_accountless", { enabled }),
  importNewApiProviders: (path: string) => invoke<NewApiImportResult>("import_newapi_providers", { path }),
  exportNewApiProviders: (path: string) => invoke<void>("export_newapi_providers", { path }),
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

  restartCodex: () =>
    invoke<void>("restart_codex"),

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

  hasNotch: () =>
    invoke<boolean>("has_notch").catch(() => false),

  getHotspotEnabled: () =>
    invoke<boolean>("get_hotspot_enabled"),

  setHotspotEnabled: (enabled: boolean) =>
    invoke<boolean>("set_hotspot_enabled", { enabled }),

  focusMainWindow: () =>
    invoke<void>("focus_main_window"),

  hotspotReady: () =>
    invoke<void>("hotspot_ready"),

  openPath: (path: string) =>
    invoke<void>("open_path", { path }),

  getSystemInfo: () =>
    invoke<{ os: string; osVersion: string; arch: string; hostname: string }>("get_system_info"),

  listCodexThreads: () =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadListPayload>>("list_codex_threads"),

  exportCodexThreads: (keys: string[], targetDir: string, format: "markdown" | "jsonl") =>
    invoke<CoreEnvelope<import("@/types/threads").ThreadExportPayload>>("export_codex_threads", {
      keys,
      targetDir,
      format,
    }),
};
