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
  /** What aggregates' gateway ids start with, unless one names its own. */
  aggregatePrefix: string;
  /** The kinds of model the gateway serves. */
  sources: ModelSources;
  /** How many models of each kind there are, served or not. */
  kindCounts: Record<GatewayModel["kind"], number>;
}

/** The kinds of model a consumer is offered, each turned on or off on its own. */
export interface ModelSources {
  relay: boolean;
  account: boolean;
  aggregate: boolean;
}

export type GatewayProfile = "generic" | "anthropicSdk" | "opencode";
