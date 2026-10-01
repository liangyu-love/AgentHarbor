export interface NewApiBalance {
  entries?: { remaining: number; used: number | null; total: number | null; unit: string; valid: boolean; label?: string | null; resetsAt?: string | null }[];
  remaining: number;
  used: number | null;
  unit: string;
  rawQuota: number;
  quotaPerUnit: number | null;
  requestCount: number | null;
  checkedAt: number;
  conversionAvailable: boolean;
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
export interface NewApiProvider {
  id: string;
  name: string;
  baseUrl: string;
  userId: number | null;
  hasAccessToken: boolean;
  hasApiKey: boolean;
  protocol: ModelProtocol;
  model: string;
  models: NewApiModel[];
  contextWindow: number | null;
  autoCompactLimit: number | null;
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
  sameModelFailover?: boolean;
  accessMode: CodexAccessMode;
  accountless: boolean;
  port: number;
  // Another tool may rewrite Codex's config.toml and silently replace the router
  // injection; this reports that the file no longer points at us.
  configDrifted: boolean;
}

export interface RouteAttempt {
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
}
export interface RouteDiagnostic {
  id: string;
  timestamp: number;
  requestedModel: string;
  requested: Record<string, string | number>;
  attempts: RouteAttempt[];
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
export interface PriceInfo {
  currency: string;
  unit: string;
  input: number | null;
  output: number | null;
  cacheRead: number | null;
  cacheWrite: number | null;
  reasoning: number | null;
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
  autoCompactLimit: number | null;
  catalogProviders?: string[];
  priceCatalogProvider?: string | null;
  compression?: CompressionPolicy;
  networkMode?: "proxy" | "direct";
}
export interface CompressionSettingInput {
  compression: CompressionPolicy;
}
