import type { ModelProtocol } from "./newapi";
import type { ModelSelection } from "./aggregation";

export type TargetId = "codex" | "claudeCode" | "opencode" | "pi";
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
  /** Claude Code CLI version when it could be detected. */
  clientVersion?: string | null;
  /** Managed fields that no longer match what the last apply wrote. */
  conflictFields?: string[];
}
export type ClientAccessMode = "native" | "router";
/** One key file of a client on this machine; existence only, never contents. */
export interface ClientKeyFile {
  /** Stable role, e.g. `config`, `sessions`, `normalConfig`. */
  id: string;
  name: string;
  path: string;
  directory: boolean;
  exists: boolean;
}
export interface ClientFiles {
  client: "codex" | "claudeCode" | "claudeDesktop" | "opencode" | "pi";
  files: ClientKeyFile[];
}
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
  modelSelection?: ModelSelection;
  aggregateIngressProtocol?: ModelProtocol;
  targetId: TargetId;
  providerId: string;
  model: string;
  /** Explicit user-entered budget; null/absent follows the backend resolver. */
  contextWindow?: number | null;
  /** Explicit user-entered output cap; null/absent follows the backend resolver. */
  maxOutputTokens?: number | null;
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
  /** i18n keys for non-blocking notices about the fields being taken over. */
  warnings?: string[];
}
export interface ClientConfigApplied {
  path: string;
  backupPath: string;
  warnings?: string[];
}
export interface ClaudeOfficialPreview {
  path: string;
  revision: string;
  removedFields: string[];
  warnings: string[];
  hasChanges: boolean;
}
