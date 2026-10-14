/**
 * TanStack Query keys shared across pages. Invalidation matches by prefix, so a
 * page that refreshes `runtimeState` also refreshes `runtimeStateDisplay`.
 */
export const queryKeys = {
  runtimeState: ["runtime-state"],
  runtimeStateDisplay: ["runtime-state", "display"],
  providers: ["newapi-state"],
  codexSubagents: ["codex-subagents"],
  // Written by applyChange only (never fetched). Outside the `newapi-state` prefix so provider
  // invalidations cannot touch it; shared so every page shows the latest config write.
  providerApplied: ["provider-applied-config"],
  clientConfigs: ["client-configs"],
  clientFiles: ["client-files"],
  claudeDesktopStatus: ["claude-desktop", "status"],
  cursorStatus: ["cursor", "status"],
  claudeDesktopCandidates: ["claude-desktop", "candidates"],
  codexAccounts: ["codex-accounts"],
  accountSources: ["account-sources"],
  gateway: ["gateway"],
  tokenUsage: ["token-usage"],
  mcpServers: ["mcp-servers"],
  installedSkills: ["installed-skills"],
  skillBackups: ["skill-backups"],
  customInstructionState: ["custom-instructions", "state"],
  customInstructionTemplates: ["custom-instructions", "templates"],
  imageGallery: ["image-gallery"],
  codexThreads: ["threads", "codex"],
  conversations: (source: "codex" | "claudeCode") => ["threads", source],
  threadDetails: (key: string, source: "codex" | "claudeCode" = "codex") => ["threads", source, "details", key],
  threadMessages: (key: string, snapshot: number, offset: number, source: "codex" | "claudeCode" = "codex") => ["threads", source, "messages", key, snapshot, offset],
  systemInfo: ["system-info"],
  terminalProxy: ["terminal-proxy"],
  autostart: ["autostart"],
} as const;
