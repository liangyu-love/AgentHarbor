export interface NewApiBalance {
  entries?: { remaining: number; used: number | null; total: number | null; unit: string; valid: boolean; label?: string | null; resetsAt?: string | null }[];
  commandCode?: CommandCodeBalanceDetails | null;
  remaining: number;
  used: number | null;
  unit: string;
  rawQuota: number;
  quotaPerUnit: number | null;
  requestCount: number | null;
  checkedAt: number;
  conversionAvailable: boolean;
}
export interface CommandCodeBalanceDetails {
  account: { name: string | null; userName: string | null } | null;
  plan: {
    id: string | null;
    name: string | null;
    status: string | null;
    currentPeriodStart: string | null;
    currentPeriodEnd: string | null;
  } | null;
  usage: {
    totalCount: number | null;
    completedCount: number | null;
    failedCount: number | null;
    /** Official percentage, 0..100. */
    successRate: number | null;
    /** Official USD cost, distinct from credits. */
    totalCost: number | null;
    totalCredits: number | null;
    totalTokensIn: number | null;
    totalTokensOut: number | null;
    periodBasis: string | null;
  } | null;
}
export interface ModelRef {
  providerId: string;
  modelId: string;
}
export interface ModelPriceOverrides {
  input?: number | null;
  output?: number | null;
  cacheRead?: number | null;
  cacheWrite?: number | null;
}
export interface ModelOverrides {
  contextWindow?: number | null;
  inputLimit?: number | null;
  maxOutputTokens?: number | null;
  /** null/absent: inherit; []: explicitly no levels. */
  reasoning?: { effort: string; description: string }[] | null;
  defaultEffort?: string | null;
  price?: ModelPriceOverrides | null;
}
export type CompressionPolicy =
  | { mode: "inherit" }
  | { mode: "percent"; percent: number }
  | { mode: "tokens"; tokens: number }
  | { mode: "clientDefault" };
export interface NewApiModel {
  id: string;
  protocol?: ModelProtocol | null;
  displayName: string;
  /**
   * Legacy upstream declaration or migration source. Effective values come
   * from the resolver; `overrides` carries explicit user values.
   */
  contextWindow: number | null;
  maxOutputTokens: number | null;
  endpointTypes: string[];
  included: boolean;
  metadataSource: "upstream" | "local" | "catalog" | "manual" | "legacy" | "";
  /** null/absent: unknown; []: the provider explicitly declares no effort levels. */
  supportedReasoningLevels?: { effort: string; description: string }[] | null;
  defaultReasoningLevel?: string | null;
  overrides?: ModelOverrides;
  catalogRef?: ModelRef | null;
  priceRef?: ModelRef | null;
  compression?: CompressionPolicy;
}
export type ModelProtocol = "responses" | "chatCompletions" | "anthropicMessages";
export type CodexAccessMode = "router" | "native";

export interface CodexSubagentSettings {
  model: string | null;
  reasoningEffort: string | null;
}

export interface CodexSubagentView {
  settings: CodexSubagentSettings;
  managed: boolean;
  applied: boolean;
  models: { id: string; label: string; reasoningEfforts: string[] }[];
  inheritedReasoningEfforts: string[];
}
export interface NewApiProvider {
  id: string;
  name: string;
  baseUrl: string;
  userId: number | null;
  hasAccessToken: boolean;
  hasApiKey: boolean;
  webSearchCapable?: boolean;
  protocol: ModelProtocol;
  model: string;
  models: NewApiModel[];
  contextWindow: number | null;
  catalogProviders: string[];
  priceCatalogProvider: string | null;
  compression: CompressionPolicy;
  enabled: boolean;
  networkMode: "proxy" | "direct";
  modelsFetchedAt: number | null;
  balance: NewApiBalance | null;
  balanceSource: string | null;
  balanceScript?: string;
  balanceSharedFrom?: string | null;
  balanceRefreshIntervalMinutes?: number;
  testStatus: "untested" | "ok" | "failed";
  testAt: number | null;
  createdAt: number;
  updatedAt: number;
}
export interface ProviderProbeEvent {
  stage: "request" | "headers" | "firstByte" | "content" | "retry" | "complete" | "failed";
  elapsedMs: number;
  status: number | null;
  text: string;
}
export interface NewApiRouterView {
  enabled: boolean;
  aggregateModelId?: string | null;
  webSearchProviderId?: string | null;
  accessMode: CodexAccessMode;
  accountless: boolean;
  port: number;
  // Another tool may rewrite Codex's config.toml and silently replace the router
  // injection; this reports that the file no longer points at us.
  configDrifted: boolean;
}

export interface RouteAttempt {
  attemptId: string;
  providerId: string;
  providerName: string;
  upstreamModel: string;
  matchedModel: string;
  protocol: ModelProtocol;
  sent: Record<string, string | number>;
  status: number | null;
  failureStage: string | null;
  error: string | null;
  durationMs: number;
  /** Catalog revision, resolver match kind and pricing status at dispatch. */
  catalogRevision: string;
  matchKind: MatchKind;
  pricingStatus: string;
  pricingSource: string;
  selectionReason: string;
  retryReason: string | null;
  aggregateId?: string | null;
  routeRevision?: string | null;
  memberId?: string | null;
  configuredProtocol?: string | null;
  effectiveProtocol?: string | null;
  compatibilityPlan?: string | null;
  credential: CredentialSnapshot;
  modelObservation: ServedModelObservation;
}

export interface ServedModelObservation {
  servedModel: string | null;
  modelMatchStatus: "same" | "declared_different" | "unknown";
  modelMatchBasis: string | null;
  unknownReason: string | null;
  conflictingModel: string | null;
}

export interface CredentialSnapshot {
  kind: string;
  status: "recorded" | "not_recorded" | "not_sent";
  reason: string | null;
  versionFingerprint: string | null;
  accountId: string | null;
  accountStatus: "recorded" | "not_recorded";
  label: string | null;
}
export interface RouteDiagnostic {
  id: string;
  timestamp: number;
  requestedModel: string;
  clientSource?: string | null;
  resolvedModel?: string | null;
  routeRevision?: number | null;
  requested: Record<string, string | number>;
  attempts: RouteAttempt[];
  skippedMembers?: { memberId: string; reason: string }[];
  stage: string;
  done: boolean;
  status: number | null;
  failureStage: string | null;
  error: string | null;
  durationMs: number;
  firstTokenMs: number | null;
}

/** Resolver output: one field with its provenance. */
export interface ResolvedField<T> {
  value: T | null;
  source: string;
  sourceRef: ModelRef | null;
  updatedAt: string | null;
}
export interface ConflictValue {
  modelRef: ModelRef;
  value: string;
}
export interface FieldConflict {
  field: string;
  candidates: ConflictValue[];
}
export interface MatchStep {
  modelId: string;
  step: string;
  matched: ModelRef[];
  note: string;
}
export type MatchKind =
  | "explicit"
  | "exact"
  | "normalized"
  | "consensus"
  | "ambiguous"
  | "invalid"
  | "unmatched";
export interface ReasoningOptionInfo {
  kind: string;
  values: string[];
  min: number | null;
  max: number | null;
}
export interface CatalogHint {
  source: ModelRef;
  canonicalRaw: string | null;
  canonicalTarget: ModelRef | null;
  canonicalStatus: "absent" | "invalid" | "missingTarget" | "verified" | "self" | "declaredAlias" | "sourceMissing";
  reasoningOptionsPresence: "declared" | "absent" | "unknown";
  reasoningOptions: ReasoningOptionInfo[];
  interleaved: boolean | { field: string } | null;
}
export type ClearCapabilityField =
  | "contextWindow" | "inputLimit" | "outputLimit" | "reasoning" | "defaultEffort"
  | "price" | "catalogRef" | "priceRef" | "compression"
  | "providerContextWindow" | "providerCatalogProviders"
  | "providerPriceCatalogProvider" | "providerCompression";
export interface ClearCapabilityPreview {
  field: ClearCapabilityField;
  hasOverride: boolean;
  configRevision: string;
  catalogRevision: string;
  officialRevision: string;
  before: ModelCapability;
  after: ModelCapability;
  scopeCount: number;
  affectedModels: { modelId: string; displayName: string }[];
}
export interface ClearCapabilityApplyResult {
  applied: NewApiStateChange | null;
  refreshedPreview: ClearCapabilityPreview | null;
}
export interface CostRates {
  input: number | null;
  output: number | null;
  cacheRead: number | null;
  cacheWrite: number | null;
  reasoning: number | null;
  inputAudio: number | null;
  outputAudio: number | null;
}
export interface CostTier extends CostRates {
  tier: { type: string; size: number | null };
}
export interface PricingSnapshot extends CostRates {
  contextOver200k: CostRates | null;
  tiers: CostTier[];
  currency: string;
  source: string;
  revision: string;
  matchKind: string;
  complete: boolean;
  sourceRef: ModelRef | null;
}
export interface PriceInfo extends CostRates {
  currency: string;
  unit: string;
  contextOver200k: CostRates | null;
  tiers: CostTier[];
  source: string;
  sourceRef: ModelRef | null;
  updatedAt: string | null;
  complete: boolean;
  note: string | null;
}
export interface CompressionPlan {
  mode: string;
  percent: number | null;
  tokens: number | null;
  source: string;
  budget: number | null;
  budgetSource: string;
  threshold: number | null;
  state: string;
  note: string | null;
}
export interface ClientProjection {
  contextWindow: number;
  contextSource: string;
  contextIsFallback: boolean;
  budget: number;
  budgetSource: string;
  inputLimit: number | null;
  outputLimit: number | null;
  outputSource: string;
  reasoning: { effort: string; description: string }[];
  reasoningSource: string;
  defaultReasoning: string | null;
  compression: CompressionPlan;
}
/** Read-only resolution of an editor draft; credentials are never transmitted. */
export interface ProviderModelResolutionDraft {
  providerId: string | null;
  baseUrl: string;
  hasDraftApiKey: boolean;
  protocol: ModelProtocol;
  contextWindow: number | null;
  catalogProviders: string[];
  compression: CompressionPolicy;
  models: NewApiModel[];
}
export interface ProviderModelResolution {
  modelId: string;
  matchKind: MatchKind;
  matched: ModelRef | null;
  context: ResolvedField<number>;
  inputLimit: ResolvedField<number>;
  output: ResolvedField<number>;
  projection: ClientProjection;
}
export interface ProviderModelResolutions {
  models: ProviderModelResolution[];
}
export interface ModelCapability {
  providerId: string;
  providerName: string;
  displayName: string;
  upstreamModel: string;
  matchedModel: string;
  matchKind: MatchKind;
  matched: ModelRef | null;
  steps: MatchStep[];
  conflicts: FieldConflict[];
  catalogHints: CatalogHint[];
  reasoning: ResolvedField<{ effort: string; description: string }[]>;
  reasoningSupports: boolean | null;
  reasoningOptions: ReasoningOptionInfo[];
  defaultReasoning: string | null;
  defaultReasoningSource: string;
  context: ResolvedField<number>;
  inputLimit: ResolvedField<number>;
  output: ResolvedField<number>;
  price: PriceInfo;
  projection: ClientProjection;
  /** Whether the projected threshold is the one clients currently use. */
  compressionApplied: boolean | null;
  /** catalog | config | none */
  compressionAppliedSource: string;
  compressionAppliedThreshold: number | null;
  compressionSharedThreshold: number | null;
  compressionLimitingModels: string[];
  compressionScope: "nativeShared" | "inactive" | "routerModel";
  compressionSharedApplied: boolean | null;
  compressionSharedPlan: SharedCompressionPlan | null;
  compressionConfigActive: boolean;
  compressionWrittenThreshold: number | null;
}
export interface SharedCompressionPlan {
  threshold: number | null;
  limitingModels: string[];
  blockingModels: string[];
  state: "computed" | "invalid" | "clientDefault" | "unknown";
}
export interface ModelsDevCatalogStatus {
  schemaVersion: number;
  sourceUrl: string;
  fetchedAt: number | null;
  lastCheckedAt: number | null;
  stale: boolean;
  origin: "runtime" | "embedded" | string;
  revision: string;
  providers: number;
  models: number;
  error: string | null;
}
export interface ModelCapabilities {
  /** Codex official snapshot status. */
  stale: boolean;
  error: string | null;
  updatedAt: number | null;
  catalog: ModelsDevCatalogStatus;
  models: ModelCapability[];
}
export interface NewApiState {
  providers: NewApiProvider[];
  router: NewApiRouterView;
  proxyRunning: boolean;
  activeProviderId: string | null;
  compression: CompressionPolicy;
}
export interface NewApiAppliedConfig {
  path: string;
  backupPath: string | null;
}
export interface NewApiStateChange {
  state: NewApiState;
  applied: NewApiAppliedConfig | null;
}
export interface NewApiImportResult extends NewApiStateChange {
  imported: number;
  skipped: number;
}
export interface NewApiProviderInput {
  id: string | null;
  name: string;
  baseUrl: string;
  userId: number | null;
  accessToken: string;
  apiKey: string;
  protocol: ModelProtocol;
  model: string;
  models: NewApiModel[];
  contextWindow: number | null;
  /** Legacy fixed threshold; only sent by very old callers. */
  autoCompactLimit?: number | null;
  catalogProviders?: string[];
  priceCatalogProvider?: string | null;
  compression?: CompressionPolicy;
  networkMode?: "proxy" | "direct";
}
export interface CompressionSettingInput {
  compression: CompressionPolicy;
}
