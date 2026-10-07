export type ConversationSource = "codex" | "claudeCode";

export interface ThreadSummary {
  key: string;
  id: string;
  cwd: string;
  folderName: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  userMessages: number;
  agentMessages: number;
  fileSize: number;
  archived: boolean;
  source: string;
  cliVersion: string;
  filePath: string;
  modelProvider: string;
  model: string;
  tokensUsed: number;
  /** Cumulative prompt tokens including the cached subset; null = unknown. */
  inputTokens: number | null;
  cachedTokens: number | null;
  outputTokens: number | null;
  threadSource: "sqlite" | "rollout" | "claudeJsonl";
  rolloutStatus: "available" | "missing";
}

export interface ThreadListPayload {
  total: number;
  lastScanAt: number;
  items: ThreadSummary[];
  pendingDeletionKeys: string[];
  pendingDeletionRecoveryNeeded?: boolean;
}

export interface ThreadUsage {
  input: number;
  output: number;
  cached: number | null;
  reasoning: number | null;
  total: number;
}

export interface ThreadDiagnostics {
  invalidLines: number;
  oversizedLines: number;
  encryptedRecords: number;
  compactions: number;
  counterResets: number;
  lastUsageOnly: number;
  undatedUsage: number;
  inheritedSessions: number;
}

export interface ThreadDetails {
  snapshotBytes: number;
  snapshotId?: string;
  usage: ThreadUsage | null;
  byModel: { label: string; usage: ThreadUsage }[];
  byDate: { label: string; usage: ThreadUsage }[];
  tools: { name: string; calls: number }[];
  diagnostics: ThreadDiagnostics;
}

export interface ThreadMessagePage {
  messages: { role: string; timestamp: string; text: string; truncated: boolean }[];
  nextOffset: number | null;
  diagnostics: ThreadDiagnostics;
}

export interface ThreadExportPayload {
  exported: number;
  directory: string;
  files: string[];
}

export interface ThreadDeleteResult {
  deleted: number;
  queued: number;
  catalogRowsDeleted: number;
  rolloutFilesDeleted: number;
  backupPath: string;
}

export type ThreadRecoveryStatus =
  | "compatible"
  | "officialAccountUnverified"
  | "providerMismatch"
  | "providerConflict"
  | "missingCatalog"
  | "schemaUnsupported"
  | "insufficientMetadata"
  | "missingRollout"
  | "invalidRollout"
  | "invalidTargetProvider"
  | "journalBlocked"
  | "recoveryPending"
  | "unsupportedPlatform";

export interface ThreadRecoveryChange {
  target: string;
  path: string;
  fromProvider: string | null;
  toProvider: string;
}

export interface ThreadRecoveryCatalogField {
  name: string;
  value: string;
}

export interface ThreadRecoveryCoordination {
  journalPath: string;
  directoryPath: string;
  backupPath: string | null;
  detail: string;
}

export interface ThreadCatalogAnalysis {
  schemaSignature: string | null;
  ready: boolean;
  missingFields: string[];
  unsupportedColumns: string[];
  relation: "topLevel" | "userFork" | "child" | "ambiguous";
  mappingErrors: string[];
}

export interface ThreadRecoveryPreview {
  threadId: string;
  revision: string;
  configuredProvider: string;
  targetProvider: string;
  databaseProvider: string | null;
  rolloutProvider: string | null;
  rolloutPath: string | null;
  catalogAnalysis: ThreadCatalogAnalysis | null;
  catalogFields: ThreadRecoveryCatalogField[];
  coordination: ThreadRecoveryCoordination | null;
  status: ThreadRecoveryStatus;
  canApply: boolean;
  reason: string;
  changes: ThreadRecoveryChange[];
}

export interface ThreadRecoveryResult {
  changed: boolean;
  backupPath: string | null;
  preview: ThreadRecoveryPreview;
}

/** routed: uses the unified Router; direct: bypasses it; broken: provider no longer defined. */
export type ThreadRouteState = "routed" | "direct" | "broken";

export interface ThreadRouteOverview {
  routerActive: boolean;
  routerProviderThreads: number;
  states: Record<string, ThreadRouteState>;
}

/** router: move into the unified Router; restore: move back to the active non-Router provider. */
export type ThreadBatchTarget = "router" | "restore";

export interface ThreadBatchCandidate {
  threadId: string;
  revision: string;
  fromProvider: string | null;
}

export interface ThreadBatchSkip {
  threadId: string;
  status: ThreadRecoveryStatus;
  reason: string;
}

export interface ThreadBatchPreview {
  targetProvider: string;
  ready: ThreadBatchCandidate[];
  skipped: ThreadBatchSkip[];
  providers: Record<string, number>;
}

export interface ThreadBatchResult {
  changed: number;
  unchanged: number;
  failed: ThreadBatchSkip[];
  reopenError: string | null;
}
