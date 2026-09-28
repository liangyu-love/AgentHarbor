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
  avatar: "agentharbor_avatar",
  wallpaperSettings: "agentharbor_wallpaper_settings_v1",
  imageWorkbenchConfig: "image-workbench-config",
  imagePromptFavorites: "image-prompt-favorites",
  claudeCodeProfiles: "agentharbor.claude-code-profiles.v1",
  relayGroupAliases: "newapi-relay-group-aliases",
} as const;
