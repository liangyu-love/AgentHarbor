import type { ModelProtocol } from "@/types/newapi";

/** Official account sources whose sign-in serves models through the local router. */
export type AccountSourceKind = "chatgpt-api" | "workbuddy" | "workbuddy-ai" | "copilot" | "gemini-cli" | "antigravity" | "kiro";

export interface AccountSourceQuota {
  used: number;
  limit: number;
  unit: string;
  paid: boolean | null;
  checkedAt: number;
  error: string | null;
}

export interface AccountSourceModel {
  id: string;
  name: string;
  contextWindow: number | null;
  maxOutputTokens?: number | null;
  reasoning: string[];
  /** The API this model is asked on, when it differs from its source's. */
  protocol?: ModelProtocol | null;
}

export interface AccountSourceAccount {
  id: string;
  name: string;
  plan: string | null;
  /** The Google Cloud project a Gemini CLI or Antigravity account's requests go to. */
  project?: string | null;
  active: boolean;
  /** Set when the vendor refused a refresh: sign in again. */
  lapsed: string | null;
  quota: AccountSourceQuota | null;
  addedAt: number;
}

export interface AccountSource {
  kind: AccountSourceKind;
  name: string;
  providerId: string;
  protocol: ModelProtocol;
  /** Shared with model inject: its router provider exists. */
  exposed: boolean;
  models: AccountSourceModel[];
  accounts: AccountSourceAccount[];
  usageUrl: string | null;
  /** Its accounts report an allowance. */
  hasQuota: boolean;
  /** Its model list is fetched with an account (and can be refreshed). */
  fetchesModels: boolean;
  /** A risk to accept before signing in. */
  risk?: string | null;
}

/** A sign-in waiting on the user: the page it opened and, for a device sign-in, the code to type there. */
export interface AccountSourceSignInPrompt {
  attemptId: string;
  kind: AccountSourceKind;
  url: string;
  code: string | null;
}

export interface AccountSourceListPayload {
  sources: AccountSource[];
}
