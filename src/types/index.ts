export type UsageSource = "local" | "api";
export type ApiProxyMode = "system" | "direct" | "manual";
export type ApiReachabilityStatus = "unknown" | "reachable" | "unreachable";
export type CustomInstructionProtectionState = "ready" | "unmanaged" | "protected";
export type CustomInstructionHistoryAction = "apply" | "clear" | "rollback";

export interface CoreWarning {
  code: string;
  message: string;
}

export interface AppPathState {
  codexHome: string;
  accountsPath: string;
  authPath: string;
  registryPath: string;
  sessionsPath: string;
  authExists: boolean;
  registryExists: boolean;
  sessionsExists: boolean;
  appDataDir?: string;
  claudeHome?: string;
  claudeProjectsExists?: boolean;
}

export interface AutoSwitchStatusPayload {
  supported: boolean;
  enabled: boolean;
  threshold5hPercent: number;
  thresholdWeeklyPercent: number;
}

export interface ApiConfigPayload {
  proxy: ApiProxyConfigPayload;
}

export interface ApiProxyConfigPayload {
  mode: ApiProxyMode;
  url: string | null;
}

export interface TerminalProxyStatus {
  supported: boolean;
  enabled: boolean;
  applied: boolean;
  url: string | null;
  noProxy: string;
  recoveryRequired: boolean;
  conflicts: string[];
  preservedVariables: string[];
  notificationSent: boolean;
}

export interface AutostartStatus {
  supported: boolean;
  enabled: boolean;
  /** Turned off outside the app (Windows Task Manager's Startup apps). */
  blocked: boolean;
}

/** What a memory optimization releases, in the order the backend runs them. */
export type MemoryArea =
  | "workingSets"
  | "systemFileCache"
  | "modifiedFileCache"
  | "modifiedList"
  | "standbyList"
  | "lowPriorityStandbyList"
  | "registryCache"
  | "combinePages";

/** Physical memory by page list, in bytes. */
export interface MemorySnapshot {
  total: number;
  inUse: number;
  modified: number;
  standby: number;
  free: number;
}

export interface MemoryStatus {
  supported: boolean;
  /** AgentHarbor runs as administrator: optimizing shows no consent prompt. */
  elevated: boolean;
  memory: MemorySnapshot | null;
}

export interface MemoryOptimizeOutcome {
  before: MemorySnapshot;
  after: MemorySnapshot;
  failed: MemoryArea[];
}

export interface TerminalProxyProbe {
  service: string;
  reachable: boolean;
  statusCode: number | null;
}

export interface ApiConnectivityPayload {
  usageStatus: ApiReachabilityStatus;
  usageLastError: string | null;
}

export interface UpdateInstallabilityPayload {
  canInstall: boolean;
  code: string;
  executablePath: string | null;
  bundlePath: string | null;
  translocated: boolean;
  quarantined: boolean;
}


export interface AppStatusPayload {
  paths: AppPathState;
  lastScanAt: number;
  usageSource: UsageSource;
  autoSwitch: AutoSwitchStatusPayload;
  api: ApiConfigPayload;
  apiConnectivity: ApiConnectivityPayload;
}

export interface CoreSnapshotPayload {
  status: AppStatusPayload;
}

export interface CustomInstructionCurrentState {
  globalPath: string;
  fileExists: boolean;
  managedBlockPresent: boolean;
  protectionState: CustomInstructionProtectionState;
  issueMessage: string | null;
  managedContent: string;
  fileContent: string;
  lastAppliedAt: number | null;
  lastTemplateCode: string | null;
  lastTemplateTitle: string | null;
}

export interface CustomInstructionHistoryEntry {
  id: string;
  createdAt: number;
  action: CustomInstructionHistoryAction;
  source: string;
  templateCode: string | null;
  templateTitle: string | null;
}

export interface CustomInstructionStatePayload {
  current: CustomInstructionCurrentState;
  history: CustomInstructionHistoryEntry[];
}

export interface CustomInstructionPreviewPayload {
  globalPath: string;
  protectionState: CustomInstructionProtectionState;
  issueMessage: string | null;
  currentManagedContent: string;
  nextManagedContent: string;
  resultingContent: string;
}

/** The agents whose MCP servers the MCP page manages. */
export type McpApp = "codex" | "claudeCode" | "openCode" | "pi" | "cursor" | "harness" | "claudeDesktop";
export type McpKind = "stdio" | "http" | "sse";

/** One server as the library keeps it, whatever agent it is written to. */
export interface McpSpec {
  transport: McpKind;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  url?: string;
  headers?: Record<string, string>;
}

export interface McpServerEntry {
  name: string;
  spec: McpSpec;
  note: string;
  /** The agents it is on. */
  apps: McpApp[];
}

export type McpCellState = "synced" | "missing" | "differs" | "off" | "outside" | "unsupported" | "notInstalled" | "unreadable";

export interface McpCell {
  app: McpApp;
  on: boolean;
  state: McpCellState;
  /** i18n key: why the agent cannot hold the server. */
  reason?: string;
}

export interface McpServerView {
  name: string;
  spec: McpSpec;
  note: string;
  cells: McpCell[];
}

export interface McpAppView {
  app: McpApp;
  path: string;
  installed: boolean;
  error?: string;
  /** Servers in the agent's configuration the library does not have. */
  unmanaged: string[];
}

export interface McpFailure {
  name: string;
  app: McpApp;
  error: string;
}

export interface McpLibraryView {
  servers: McpServerView[];
  apps: McpAppView[];
  failures: McpFailure[];
  added: string[];
}

export interface PastedMcpServer {
  /** Empty when the paste did not name it. */
  name: string;
  spec: McpSpec;
}

export type SkillSource = "codex" | "claudeCode";

export interface InstalledSkillSummary {
  id: string;
  enabled: boolean;
  conflict?: boolean;
  conflictDirectoryPath?: string | null;
  readOnly?: boolean;
  name: string;
  title: string | null;
  summary: string | null;
  relativePath: string;
  directoryPath: string;
  skillFilePath: string;
  updatedAt: number | null;
}

export interface SkillListPayload {
  items: InstalledSkillSummary[];
  total: number;
  rootPath: string;
  lastScanAt: number;
}

export interface SkillBackupSummary {
  id: string;
  skillID: string;
  name: string;
  title: string | null;
  relativePath: string;
  backupPath: string;
  createdAt: number;
}

export interface SkillBackupListPayload {
  items: SkillBackupSummary[];
  total: number;
  rootPath: string;
  lastScanAt: number;
}

export interface SkillImportPayload {
  skill: InstalledSkillSummary;
  replacedExisting: boolean;
  backup: SkillBackupSummary | null;
}

export interface SkillRemovePayload {
  removedSkillID: string;
  backup: SkillBackupSummary;
  remainingInstalledCount: number;
}

export interface SkillRestorePayload {
  restoredSkill: InstalledSkillSummary;
  backup: SkillBackupSummary;
  rollbackBackup: SkillBackupSummary | null;
}

export interface SkillDeleteBackupPayload {
  deletedBackupID: string;
  remainingBackupCount: number;
}

export interface CleanPayload {
  authBackupsRemoved: number;
  registryBackupsRemoved: number;
  staleEntriesRemoved: number;
}

export interface RebuildRegistryPayload {
  accountCount: number;
  activeAccountKey: string | null;
  registryUpdated: boolean;
}

export interface AutoSwitchConfigPayload {
  autoSwitch: AutoSwitchStatusPayload;
}

export interface ApiModePayload {
  api: ApiConfigPayload;
}

export interface ApiProxyTestPayload {
  code: string;
  reachable: boolean;
  statusCode: number | null;
  message: string;
}

export interface ApiProxyDetectPayload {
  found: boolean;
  mode: ApiProxyMode | null;
  url: string | null;
  probe: ApiProxyTestPayload;
}


export interface DiagnosePayload {
  paths: AppPathState;
  coreVersion: string;
  platform: { os: string; arch: string };
  registryState: { accountCount: number };
  sessionState: { latestRolloutFound: boolean };
  apiState: {
    usageAttemptCount: number;
    usageSuccessCount: number;
    nameAttemptCount: number;
    nameSuccessCount: number;
    lastUsageFailure: string | null;
    lastUsageFailureAccount: string | null;
    lastNameFailure: string | null;
    lastNameFailureAccount: string | null;
  };
}

export interface CoreEnvelope<T> {
  schemaVersion: number;
  success: boolean;
  code: string;
  message: string;
  warnings: CoreWarning[];
  data: T;
}

export interface AccountRateLimitWindow {
  usedPercent: number;
  remainingPercent: number;
  windowMinutes: number | null;
  resetsAt: number | null;
}

export type CodexAccountUsageStatus = "unknown" | "healthy" | "expired" | "error";

export interface AccountResetCredit {
  status: string;
  grantedAt: number | null;
  expiresAt: number | null;
}

export interface AccountResetCredits {
  availableCount: number;
  credits: AccountResetCredit[] | null;
}

export interface CodexAccountSummary {
  accountKey: string;
  email: string;
  displayName: string;
  plan: string;
  authMode: string;
  isActive: boolean;
  createdAt: number;
  lastUsedAt: number | null;
  lastUsageAt: number | null;
  primaryWindow: AccountRateLimitWindow | null;
  secondaryWindow: AccountRateLimitWindow | null;
  resetCredits?: AccountResetCredits | null;
  usageStatus: CodexAccountUsageStatus;
  usageError: string | null;
}

export interface CodexAccountListPayload {
  activeAccountKey: string | null;
  refreshedAt: number;
  items: CodexAccountSummary[];
}

export interface CacheUsageTotals {
  reportedCount: number;
  promptTokens: number;
  readTokens: number;
  writeTokens: number;
}

export type UsageRange = "today" | "week" | "month";
export type UsageChannel = "official" | "relay" | "aggregate";

export interface UsageTotals {
  requestCount: number;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  /** Input, cache writes and output: what the models processed anew. */
  newTokens: number;
  /** Cost in micro-USD (1e-6 USD). */
  costMicros: number;
  pricedCount: number;
  unpricedCount: number;
  partialCount: number;
  coverageUnknownCount: number;
  /** The part of the cost priced afterwards at current rates. */
  estimatedMicros: number;
  estimatedCount: number;
  cacheSavingsMicros: number;
  cacheUsage: CacheUsageTotals;
}

export interface UsageRequestFacts {
  statusCode: number;
  success: boolean;
  durationMs: number;
  firstTokenMs?: number | null;
  servedModel?: string | null;
  modelMatchStatus: string;
  credentialStatus: string;
  credentialFingerprint?: string | null;
  /** Joined with the transcript turn of the same call. */
  matched: boolean;
}

/** One model call, from whichever records saw it. */
export interface UsageEntry {
  timestamp: number;
  client: string;
  channel: UsageChannel;
  providerId?: string | null;
  providerName?: string | null;
  model: string;
  requestedModel?: string | null;
  project?: string | null;
  sessionId?: string | null;
  /** Input excludes cache reads and writes; output includes reasoning. */
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  usageReported: boolean;
  cacheReported: boolean;
  costUsd?: number | null;
  /** priced | partial | unpriced | legacy */
  pricingStatus: string;
  estimated: boolean;
  pricingSource?: string | null;
  multiplier?: number | null;
  cacheSavingsUsd?: number | null;
  request?: UsageRequestFacts | null;
}

export interface UsageGroup {
  key: string;
  label: string;
  detail?: string | null;
  client?: string | null;
  channel?: UsageChannel | null;
  providerId?: string | null;
  totals: UsageTotals;
}

export interface UsagePart {
  key: string;
  newTokens: number;
  costMicros: number;
}

export interface UsageBucket {
  /** An hour ("09") of today, or a date ("2026-10-17"). */
  key: string;
  start: number;
  totals: UsageTotals;
  byClient: UsagePart[];
  byChannel: UsagePart[];
  byModel: UsagePart[];
}

export interface TranscriptSource {
  sourceId: "codex" | "claude";
  available: boolean;
  fileCount: number;
  updatedFileCount: number;
}

export interface UsagePayload {
  generatedAt: number;
  range: UsageRange;
  start: number;
  previousStart: number;
  today: UsageTotals;
  current: UsageTotals;
  previous: UsageTotals;
  allTime: UsageTotals;
  buckets: UsageBucket[];
  byClient: UsageGroup[];
  byChannel: UsageGroup[];
  byModel: UsageGroup[];
  byProject: UsageGroup[];
  bySession: UsageGroup[];
  recent: UsageEntry[];
  sources: {
    transcripts: TranscriptSource[];
    relaySince?: number | null;
    relayCount: number;
    matchedCount: number;
  };
  warningCount: number;
  warnings: string[];
}

// ---------------------------------------------------------------------------
// Analytics
// ---------------------------------------------------------------------------

export interface DailyActivity {
  date: string;
  sessionCount: number;
  totalFileSize: number;
  activityLevel: number;
}

export interface TodaySummary {
  sessionCount: number;
  totalFileSize: number;
  activeMinutesEstimate: number;
}

export interface SessionStats {
  totalSessions: number;
  totalSizeBytes: number;
  activeDays: number;
  avgSessionsPerActiveDay: number;
  mostActiveDate: string | null;
  mostActiveCount: number;
}

export interface UsageAnalyticsPayload {
  today: TodaySummary;
  sessionStats: SessionStats;
  dailyActivity: DailyActivity[];
}

export interface QuotaHistoryPoint {
  timestamp: number;
  accountKey: string;
  primaryUsedPercent: number | null;
  secondaryUsedPercent: number | null;
}

export interface QuotaHistoryPayload {
  points: QuotaHistoryPoint[];
}

// ---------------------------------------------------------------------------
// Session analytics (new 4 endpoints)
// ---------------------------------------------------------------------------

export type AnalyticsRange = "today" | "week" | "month";

export interface TokenDaySeries {
  date: string;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  cumulative: number;
}

export interface TokenAnalyticsPayload {
  totalTokens: number;
  avgPerSession: number;
  inputPct: number;
  outputPct: number;
  reasoningPct: number;
  inputTotal: number;
  outputTotal: number;
  reasoningTotal: number;
  series: TokenDaySeries[];
}

export interface ToolRankItem {
  name: string;
  count: number;
}

export interface ToolAnalyticsPayload {
  totalCalls: number;
  distinctCount: number;
  searchCount: number;
  editCount: number;
  topTools: ToolRankItem[];
}

export interface ChangeDaySeries {
  date: string;
  commands: number;
  writeOps: number;
  readOps: number;
}

export interface ChangeAnalyticsPayload {
  totalCommands: number;
  writeCommands: number;
  readCommands: number;
  otherCommands: number;
  series: ChangeDaySeries[];
}
