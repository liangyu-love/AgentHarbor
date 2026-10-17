/**
 * Browser storage keys. The values are persisted on users' machines, so they must
 * never change; `index.html` reads `theme` before the bundle loads.
 */
export const storageKeys = {
  theme: "theme",
  accentColor: "accent_color",
  language: "app_language",
  refreshInterval: "refresh_interval",
  sidebarCollapsed: "sidebar_collapsed",
  threadsViewMode: "threads_view_mode",
  mcpSource: "agentharbor.mcp-source",
  instructionSource: "agentharbor.instruction-source",
  instructionPresets: "agentharbor.instruction-presets.v1",
  threadsSource: "threads_source",
  avatar: "agentharbor_avatar",
  wallpaperSettings: "agentharbor_wallpaper_settings_v1",
  imageWorkbenchConfig: "image-workbench-config",
  imagePromptGroup: "agentharbor.image-prompt-group",
  claudeCodeProfiles: "agentharbor.claude-code-profiles.v1",
  claudeDesktopProfiles: "agentharbor.claude-desktop-profiles.v1",
  relayGroupAliases: "newapi-relay-group-aliases",
  foldedSections: "agentharbor.folded-sections.v1",
  launchFolders: "agentharbor.launch-folders.v1",
  usageRange: "agentharbor.usage-range",
  usageMetric: "agentharbor.usage-metric",
} as const;
