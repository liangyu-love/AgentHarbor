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
  contextWindow: number | null;
  maxOutputTokens: number | null;
  endpointTypes: string[];
}
export interface NewApiProvider {
  id: string;
  name: string;
  baseUrl: string;
  userId: number | null;
  hasAccessToken: boolean;
  hasApiKey: boolean;
  model: string;
  contextWindow: number | null;
  autoCompactLimit: number | null;
  enabled: boolean;
  networkMode: "proxy" | "direct";
  models: NewApiModel[];
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
  model: string;
  contextWindow: number | null;
  autoCompactLimit: number | null;
  networkMode?: "proxy" | "direct";
}
