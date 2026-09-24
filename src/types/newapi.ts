export interface NewApiBalance {
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
  displayName: string;
  contextWindow: number | null;
  maxOutputTokens: number | null;
  endpointTypes: string[];
  included: boolean;
  metadataSource: "upstream" | "local" | "catalog" | "manual";
}
export type ModelProtocol = "responses" | "chatCompletions" | "anthropicMessages";
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
  testStatus: "untested" | "ok" | "failed";
  testAt: number | null;
  createdAt: number;
  updatedAt: number;
}
export interface NewApiRouterView {
  enabled: boolean;
  accountless: boolean;
  port: number;
  // Another tool may rewrite Codex's config.toml and silently replace the router
  // injection; this reports that the file no longer points at us.
  configDrifted: boolean;
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
