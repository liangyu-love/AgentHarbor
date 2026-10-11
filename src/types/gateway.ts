import type { ModelProtocol } from "@/types/newapi";

export interface GatewayKey {
  id: string;
  name: string;
  /** The key with its middle hidden; the secret is fetched only on reveal or copy. */
  masked: string;
  createdAt: number;
}

export interface GatewayModel {
  id: string;
  kind: "relay" | "account" | "aggregate";
  sourceName: string;
  sourceId: string;
  modelId: string;
  label: string;
  protocol: ModelProtocol;
  contextWindow: number | null;
}

export interface GatewayState {
  port: number;
  running: boolean;
  keepRunning: boolean;
  keys: GatewayKey[];
  models: GatewayModel[];
}

export type GatewayProfile = "generic" | "anthropicSdk" | "opencode";
