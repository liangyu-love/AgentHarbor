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
export interface NewApiModel {
  id: string;
  protocol?: ModelProtocol | null;
  displayName: string;
  contextWindow: number | null;
  maxOutputTokens: number | null;
  endpointTypes: string[];
  included: boolean;
  metadataSource: "upstream" | "local" | "catalog" | "manual";
  /** null/absent: unknown; []: the provider explicitly declares no effort levels. */
  supportedReasoningLevels?: { effort: string; description: string }[] | null;
  defaultReasoningLevel?: string | null;
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
}

export interface CapabilityField<T> { value: T; source: string; updatedAt: number | null }
export interface ModelCapability {
  providerId: string; providerName: string; displayName: string; upstreamModel: string; matchedModel: string;
  reasoning: CapabilityField<{ effort: string; description: string }[]>;
  defaultReasoning: string | null;
  context: CapabilityField<number | null>;
  output: CapabilityField<number | null>;
}
export interface ModelCapabilities { stale: boolean; error: string | null; updatedAt: number | null; models: ModelCapability[] }
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
export interface NewApiState {
  providers: NewApiProvider[];
  router: NewApiRouterView;
  proxyRunning: boolean;
  activeProviderId: string | null;
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
  networkMode?: "proxy" | "direct";
}
