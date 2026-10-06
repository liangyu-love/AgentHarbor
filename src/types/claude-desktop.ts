/**
 * Claude Desktop 配置的前端类型。
 *
 * 与 Claude Code 客户端配置**完全独立**：Desktop 是四文件事务，不复用
 * `ClientConfigRequest` / `ClientStatus`。
 */

export type DesktopAction =
  | "applyDirect"
  | "applyRouter"
  | "reapply"
  | "restorePrevious"
  | "switchOfficial"
  | "release"
  | "recoverRollback";

export type OneMMode = "auto" | "off";

export interface ClaudeDesktopPathsChoice {
  normalRoot?: string | null;
  thirdPartyRoot?: string | null;
}

export interface ClaudeDesktopModelChoice {
  modelId: string;
  labelOverride?: string | null;
  oneMMode: OneMMode;
}

export type ClaudeDesktopRouteRole = "sonnet" | "opus" | "haiku" | "fable";

export interface ClaudeDesktopRouteRoleView {
  role: ClaudeDesktopRouteRole;
  roleAlias: string;
}

export interface ClaudeDesktopRouteChoice extends ClaudeDesktopRouteRoleView {
  targetRef?: import("./aggregation").ModelTargetRef;
  providerId: string;
  modelId: string;
  labelOverride?: string | null;
  oneMMode: OneMMode;
}

export interface ClaudeDesktopRouteView extends ClaudeDesktopRouteRoleView {
  targetRef?: import("./aggregation").ModelTargetRef;
  providerId: string;
  providerName: string;
  modelId: string;
  label: string;
  oneM: boolean;
  protocol: string;
  available: boolean;
  reason: string | null;
}

export interface ClaudeDesktopRouterSummary {
  boundPort: number;
  routeRevision: number;
  gatewayTokenVersion: number;
  listenerReady: boolean;
}

export interface ClaudeDesktopActionRequest {
  providerId?: string | null;
  models?: ClaudeDesktopModelChoice[];
  routes?: ClaudeDesktopRouteChoice[];
  rotateGatewayToken?: boolean;
  paths?: ClaudeDesktopPathsChoice;
  rebaseline?: boolean;
}

export interface ClaudeDesktopPathsSummary {
  normalRoot: string;
  thirdPartyRoot: string;
  normalConfig: string;
  thirdPartyConfig: string;
  profilePath: string;
  metaPath: string;
  isolated: boolean;
}

export interface ClaudeDesktopDetection {
  platform: string;
  /** 探测失败时为 `unknown`；不编造版本。 */
  version: string;
  versionSource: string;
  installed: boolean;
  notes: string[];
}

export interface ClaudeDesktopPathCandidate {
  id: string;
  normalRoot: string;
  thirdPartyRoot: string;
  normalExists: boolean;
  thirdPartyExists: boolean;
  hasInstallEvidence: boolean;
}

export interface ClaudeDesktopModelCandidate {
  modelId: string;
  label: string;
  displayName: string;
  protocol: string;
  protocolSource: string;
  protocolVerified: boolean;
  included: boolean;
  supports1m: boolean;
  contextWindow: number | null;
  contextSource: string;
  selectable: boolean;
  excludeReason: string | null;
}

export interface ClaudeDesktopEndpoint {
  profileBaseUrl: string;
  messagesUrl: string;
  modelsUrl: string;
  plaintext: boolean;
}

export interface ClaudeDesktopProviderCandidate {
  providerId: string;
  name: string;
  enabled: boolean;
  hasInferenceKey: boolean;
  eligible: boolean;
  reason: string | null;
  endpoint: ClaudeDesktopEndpoint | null;
  models: ClaudeDesktopModelCandidate[];
}

export interface ClaudeDesktopCandidates {
  routeAggregates?: ClaudeDesktopAggregateCandidate[];
  paths: ClaudeDesktopPathsSummary | null;
  /** 多渠道候选；由用户明确选择，不按字典序自动挑。 */
  pathCandidates: ClaudeDesktopPathCandidate[];
  selectedPathId: string | null;
  detector: ClaudeDesktopDetection;
  providers: ClaudeDesktopProviderCandidate[];
  routeProviders?: ClaudeDesktopProviderCandidate[];
  routeRoles?: ClaudeDesktopRouteRoleView[];
  selectedProviderId: string | null;
}

export interface ClaudeDesktopAggregateCandidate {
  aggregateId: string; name: string; enabled: boolean; eligible: boolean; reason: string | null;
  model: ClaudeDesktopModelCandidate;
}

export interface ClaudeDesktopManagedModel {
  modelId: string;
  label: string;
  oneM: boolean;
}

export interface ClaudeDesktopActionView {
  action: DesktopAction;
  enabled: boolean;
  disabledReason: string | null;
}

export interface ClaudeDesktopStatus {
  detection: ClaudeDesktopDetection;
  paths: ClaudeDesktopPathsSummary | null;
  mode: "unknown" | "official" | "foreignThirdParty" | "direct" | "router";
  managementState:
    | "managed"
    | "unmanaged"
    | "stale"
    | "orphaned"
    | "conflict"
    | "recoveryRequired";
  isManaged: boolean;
  providerId: string | null;
  providerName: string | null;
  providerMissing: boolean;
  models: ClaudeDesktopManagedModel[];
  /** 保留用户的自动/关闭与显示名选择，重新应用时不可从有效值反推。 */
  modelChoices: ClaudeDesktopModelChoice[];
  routeChoices?: ClaudeDesktopRouteChoice[];
  routeViews?: ClaudeDesktopRouteView[];
  router?: ClaudeDesktopRouterSummary | null;
  staleReasons: string[];
  conflicts: string[];
  recoveryState: string | null;
  pendingRecovery: boolean;
  warnings: string[];
  lastAppliedAt: number | null;
  restartRequired: boolean;
  /** Desktop 应用当前是否在运行（进程探测，不是从文件推断）。 */
  appRunning: boolean;
  /** 是否已确认可自动启动；false 时界面不提供重启。 */
  restartCapable: boolean;
  launchMethod: string | null;
  actions: ClaudeDesktopActionView[];
  backupCount: number;
}

export interface ClaudeDesktopRestartResult {
  launchMethod: string;
  running: boolean;
  restartCapable: boolean;
}

export interface ClaudeDesktopExitRequest {
  intent: "quit" | "restart";
  error?: string | null;
}

export interface ClaudeDesktopManagedChange {
  path: string;
  before: string | null;
  after: string | null;
}

export interface ClaudeDesktopFileChange {
  role: string;
  path: string;
  kind: "create" | "update" | "delete" | "noop";
  managedChanges: ClaudeDesktopManagedChange[];
  note: string | null;
}

export interface ClaudeDesktopPreview {
  revision: string;
  action: DesktopAction;
  providerId: string | null;
  providerName: string | null;
  models: ClaudeDesktopManagedModel[];
  routeViews?: ClaudeDesktopRouteView[];
  router?: ClaudeDesktopRouterSummary | null;
  messagesUrl: string | null;
  modelsUrl: string | null;
  files: ClaudeDesktopFileChange[];
  warnings: string[];
  conflicts: string[];
  requiresRestart: boolean;
  externalProfileCount: number;
  managedEntryPresent: boolean;
  currentAppliedId: string | null;
  appliesTo: string;
}

export interface ClaudeDesktopApplied {
  transactionId: string;
  changedPaths: string[];
  backupId: string | null;
  managementState: string;
  requiresRestart: boolean;
  warnings: string[];
}

/** 后端返回的结构化错误（JSON 字符串）解码后的形态。 */
export interface ClaudeDesktopError {
  code: string;
  message: string;
  fileRole: string | null;
  modelId: string | null;
  retryable: boolean;
  suggestedAction: string | null;
  params: string[];
}

export type ClaudeDesktopBearerStatus =
  | "notChecked"
  | "modelsAccessible"
  | "authRejected"
  | "endpointUnavailable"
  | "networkFailed"
  | "invalidResponse"
  | "rateLimited";

export interface ClaudeDesktopProbeRequest {
  providerId: string;
  modelIds?: string[];
  paths?: ClaudeDesktopPathsChoice;
  /** 请求 Messages 兼容性核对；会向上游发一次 max_tokens:1 的请求。 */
  checkMessages?: boolean;
}

export type ClaudeDesktopMessagesStatus =
  | "notChecked"
  | "messagesAccessible"
  | "authRejected"
  | "endpointUnavailable"
  | "requestRejected"
  | "rateLimited"
  | "networkFailed"
  | "serverError"
  | "invalidResponse";

export interface ClaudeDesktopMessagesReport {
  url: string;
  model: string;
  status: ClaudeDesktopMessagesStatus;
  httpStatus: number | null;
  /** 上游自报的模型；用于识别中转是否改写模型名。 */
  responseModel: string | null;
  stopReason: string | null;
  checkedAt: number;
  note: string | null;
}

/** 检测结果：列表检测始终有；Messages 核对只在显式请求时有。 */
export interface ClaudeDesktopProbeOutcome {
  bearer: ClaudeDesktopBearerReport;
  messages: ClaudeDesktopMessagesReport | null;
}

export interface ClaudeDesktopBearerReport {
  providerId: string;
  url: string;
  status: ClaudeDesktopBearerStatus;
  httpStatus: number | null;
  egress: string;
  modelsVisible: string[];
  modelsMissing: string[];
  listedModels: number | null;
  checkedAt: number;
  fingerprint: string;
  note: string | null;
}
