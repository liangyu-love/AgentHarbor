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
  request?: ClientConfigRequest | null;
}
export type ClientAccessMode = "native" | "router";
export interface ClaudeModels {
  sonnet: string;
  opus: string;
  fable: string;
  haiku: string;
  subagent: string;
}
export interface ClaudeOneM {
  startup: boolean;
  sonnet: boolean;
  opus: boolean;
  fable: boolean;
  haiku: boolean;
  subagent: boolean;
}
export interface ClientConfigRequest {
  targetId: TargetId;
  providerId: string;
  model: string;
  contextWindow: number;
  maxOutputTokens: number;
  accessMode: ClientAccessMode;
  providerIds: string[];
  providerModelIds?: Record<string, string[]>;
  claudeModels: ClaudeModels;
  claudeModelIds?: string[] | null;
  claudeOneM?: ClaudeOneM | null;
  authField: "authToken" | "apiKey";
}
export interface ClientConfigPreview {
  path: string;
  revision: string;
  content: string;
  removedFields?: string[];
}
export interface ClientConfigApplied {
  path: string;
  backupPath: string;
  warning?: string;
}
