import type { ModelProtocol } from "./newapi";

export type TargetId = "codex" | "claudeCode" | "opencode";
export interface ClientStatus {
  targetId: TargetId;
  name: string;
  path: string;
  protocols: ModelProtocol[];
  configured: boolean;
  conflict: boolean;
  error: string | null;
  providerId: string | null;
  model: string | null;
}
export interface ClientConfigRequest {
  targetId: TargetId;
  providerId: string;
  model: string;
  contextWindow: number;
  maxOutputTokens: number;
}
export interface ClientConfigPreview {
  path: string;
  revision: string;
  content: string;
}
export interface ClientConfigApplied {
  path: string;
  backupPath: string;
}
