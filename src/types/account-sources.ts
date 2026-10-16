import type { ModelProtocol } from "@/types/newapi";

/** Official account sources whose sign-in serves models through the local router. */
export type AccountSourceKind = "chatgpt-api" | "workbuddy" | "workbuddy-ai" | "copilot" | "gemini-cli" | "antigravity" | "kiro" | "grok" | "cursor"
  | "qoder" | "qoder-cn" | "trae-cn" | "trae-global" | "minimax-code" | "minimax-code-global" | "zcode" | "opencode-zen" | "opencode-go" | "commandcode";

/** A way to sign in to a source; `input` asks the user for something first. */
export interface AccountSourceMethod {
  id: string;
  /** A key or token (`secret`) or a name or address (`text`) to type first. */
  input: "secret" | "text" | null;
}

/** What came of a daily check-in: claimed now, in already, or why none. */
export type AccountSourceCheckinOutcome = "claimed" | "done" | "ineligible" | "inactive" | "failed" | "captcha" | "own-app";

export interface AccountSourceCheckin {
  /** The Beijing day it counts in, "2026-10-15". */
  day: string;
  at: number;
  outcome: AccountSourceCheckinOutcome;
  credit: number;
  streak: number;
  message: string | null;
  validUntil: number | null;
  offline: boolean;
  tries: number;
}

export interface AccountSourceQuota {
  used: number;
  limit: number;
  unit: string;
  paid: boolean | null;
  checkedAt: number;
  error: string | null;
  /** Credits left, for an account whose balance has no cycle limit. */
  remaining?: number | null;
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
  /** What came of its last daily check-in. */
  checkin: AccountSourceCheckin | null;
  /** Left out of its source's daily check-in. */
  checkinOff: boolean;
}

export interface AccountSource {
  kind: AccountSourceKind;
  name: string;
  providerId: string;
  protocol: ModelProtocol;
  /** Shared with model inject: its router provider exists. */
  exposed: boolean;
  /** Models left out of the share with model inject (and Codex's menu). */
  hiddenModels?: string[];
  models: AccountSourceModel[];
  accounts: AccountSourceAccount[];
  usageUrl: string | null;
  /** Its accounts report an allowance. */
  hasQuota: boolean;
  /** Its model list is fetched with an account (and can be refreshed). */
  fetchesModels: boolean;
  /** A risk to accept before signing in. */
  risk?: string | null;
  /** The ways to sign in to it; the first is offered first. */
  methods: AccountSourceMethod[];
  /** It has a daily check-in. */
  checksIn: boolean;
  /** Its accounts are checked in each day. */
  autoCheckin: boolean;
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
  /** The Beijing day check-ins count in: a check-in of this day is today's. */
  today: string;
}
