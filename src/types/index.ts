export type UsageSource = "local" | "api";
export type ApiProxyMode = "system" | "direct" | "manual";
export type ApiReachabilityStatus = "unknown" | "reachable" | "unreachable";
export type McpTransport = "stdio" | "http" | "sse" | "unknown";
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

export interface McpServerSummary {
  name: string;
  transport: McpTransport;
  enabled: boolean;
  sourcePath: string;
  command: string | null;
  args: string[];
  url: string | null;
  headers: Record<string, string>;
  environment: Record<string, string>;
}

export interface McpServerListPayload {
  items: McpServerSummary[];
  total: number;
  sourcePath: string;
  lastScanAt: number;
}

export interface McpServerMutationPayload {
  server: McpServerSummary;
  total: number;
  sourcePath: string;
}

export interface McpServerRemovePayload {
  removedName: string;
  total: number;
  sourcePath: string;
}

export interface InstalledSkillSummary {
  id: string;
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

export interface RelayUsageTotals {
  requestCount: number;
  successfulCount: number;
  reportedCount: number;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  /** Estimated cost in micro-USD (1e-6 USD). */
  costMicros: number;
  pricedCount: number;
  unpricedCount: number;
}

export interface RelayProviderUsage {
  providerId: string;
  providerName: string;
  totals: RelayUsageTotals;
}

export interface RelayModelUsage extends RelayProviderUsage {
  model: string;
}

export interface RelayUsageDay {
  date: string;
  totals: RelayUsageTotals;
}

export interface RelayUsageRecord {
  timestamp: number;
  date: string;
  providerId: string;
  providerName: string;
  model: string;
  protocol: string;
  statusCode: number;
  durationMs: number;
  firstTokenMs?: number | null;
  success: boolean;
  usageReported: boolean;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  totalTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  /** Estimated cost in USD at the rates fixed when the request started. */
  costUsd?: number | null;
  /** priced | partial | unpriced | legacy */
  pricingStatus: string;
  pricingSource: string;
  catalogRevision: string;
}

export interface RelayUsagePayload {
  generatedAt: number;
  today: RelayUsageTotals;
  week: RelayUsageTotals;
  allTime: RelayUsageTotals;
  byProvider: RelayProviderUsage[];
  byModel: RelayModelUsage[];
  daily: RelayUsageDay[];
  recent: RelayUsageRecord[];
}

export interface TokenUsageTotals {
  turnCount: number;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
}

export interface TokenUsageSource {
  sourceId: string;
  sourceName: string;
  available: boolean;
  fileCount: number;
  updatedFileCount: number;
  totals: TokenUsageTotals;
}

export interface TokenModelUsage {
  sourceId: string;
  sourceName: string;
  model: string;
  totals: TokenUsageTotals;
}

export interface TokenUsageDay {
  date: string;
  totals: TokenUsageTotals;
  byModel: TokenModelUsage[];
}

export interface RecentTokenUsage {
  timestamp: number;
  date: string;
  sourceId: string;
  sourceName: string;
  model: string;
  sessionId: string;
  inputTokens: number;
  outputTokens: number;
  reasoningTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
}

export interface LocalTokenUsagePayload {
  generatedAt: number;
  today: TokenUsageTotals;
  week: TokenUsageTotals;
  allTime: TokenUsageTotals;
  bySource: TokenUsageSource[];
  byModel: TokenModelUsage[];
  daily: TokenUsageDay[];
  recent: RecentTokenUsage[];
  warningCount: number;
  warnings: string[];
}

export interface TokenUsagePayload {
  generatedAt: number;
  local: LocalTokenUsagePayload;
  relay: RelayUsagePayload;
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
