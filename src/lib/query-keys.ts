/**
 * TanStack Query keys shared across pages. Invalidation matches by prefix, so a
 * page that refreshes `runtimeState` also refreshes `runtimeStateDisplay`.
 */
export const queryKeys = {
  runtimeState: ["runtime-state"],
  runtimeStateDisplay: ["runtime-state", "display"],
  providers: ["newapi-state"],
  clientConfigs: ["client-configs"],
  codexAccounts: ["codex-accounts"],
  tokenUsage: ["token-usage"],
  mcpServers: ["mcp-servers"],
  installedSkills: ["installed-skills"],
  skillBackups: ["skill-backups"],
  customInstructionState: ["custom-instructions", "state"],
  customInstructionTemplates: ["custom-instructions", "templates"],
  imageGallery: ["image-gallery"],
  codexThreads: ["threads", "codex"],
  systemInfo: ["system-info"],
} as const;
