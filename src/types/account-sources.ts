import type { ModelProtocol } from "@/types/newapi";

/** Official account sources whose sign-in serves models through the local router. */
export type AccountSourceKind = "chatgpt-api" | "workbuddy" | "workbuddy-ai";

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
  reasoning: string[];
}

export interface AccountSourceAccount {
  id: string;
  name: string;
  plan: string | null;
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
}

export interface AccountSourceListPayload {
  sources: AccountSource[];
}
