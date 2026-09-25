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
  threadSource: "sqlite" | "rollout";
  rolloutStatus: "available" | "missing";
}

export interface ThreadListPayload {
  total: number;
  lastScanAt: number;
  items: ThreadSummary[];
}

export interface ThreadExportPayload {
  exported: number;
  directory: string;
  files: string[];
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
