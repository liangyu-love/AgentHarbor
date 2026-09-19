export interface NewApiConnection {
  baseUrl: string;
  userId: number | null;
  hasAccessToken: boolean;
  hasApiKey: boolean;
  model: string;
  contextWindow: number | null;
  autoCompactLimit: number | null;
}
export interface NewApiConnectionInput extends Omit<NewApiConnection, "hasAccessToken" | "hasApiKey"> {
  accessToken: string;
  apiKey: string;
}
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
export interface NewApiAppliedConfig { path: string; backupPath: string | null }
