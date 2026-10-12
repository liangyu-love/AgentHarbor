import type { ClientProjection, CompressionPolicy, ModelProtocol, ModelRef } from "./newapi";

export type ModelTargetRef = { kind: "provider"; providerId: string; modelId: string }
  | { kind: "aggregate"; aggregateId: string };
export interface ModelSelection { defaultTarget?: ModelTargetRef | null; availableTargets: ModelTargetRef[] }
export interface AggregateMember { id: string; providerId: string; actualModelId: string; enabled: boolean; priority: number; weight: number }
export interface AggregateModel {
  id: string; publicModelId: string; displayName: string; enabled: boolean; members: AggregateMember[];
  canonicalRef?: ModelRef | null;
  strategy: "roundRobin" | "weightedRoundRobin"; affinity: "session" | "statelessRotation";
  retryPolicy: { maxAttempts: number; failureThreshold: number }; compression: CompressionPolicy;
  /** Seconds a streamed reply may stay silent before another member is asked; 0 waits. */
  firstTokenTimeoutSecs?: number;
  /** Its gateway id prefix; empty uses the gateway's shared one. */
  gatewayPrefix?: string | null;
}
export interface AggregateView extends AggregateModel {
  available: boolean; reason: string | null; projection: ClientProjection | null; routeRevision: string;
  memberStates: { memberId: string; providerName: string; protocol: ModelProtocol | null; available: boolean;
    reason: string | null; projection: ClientProjection | null }[];
}
export interface AggregationState { models: AggregateView[]; externalEnabled: boolean; listenerReady: boolean }
export interface AgentConnectionExport {
  schemaVersion: number; consumerProfile: { id: string; ingressProtocol: ModelProtocol; modelsFormat: "openAi" | "anthropic" | "internalCatalog" };
  baseUrl: string; requestPath: string; modelsPath: string | null; selectedModelId: string | null;
  modelCatalog: { targetRef: ModelTargetRef; publicModelId: string; label: string; capabilities: unknown; limits: unknown }[];
  authScheme: unknown; localCredentialRef: unknown; listenerReady: boolean; runtimeRequirement: string;
  configRevision: string; catalogRevision: string; tokenVersion: string;
}
export interface ConnectionExportResult { connection: AgentConnectionExport; configText: string }
export const aggregationQueryKey = ["model-aggregation"] as const;
/** One member's routing state right now; display only. */
export interface MemberLive { memberId: string; inFlight: number; failures: number; coolingMs: number | null; tripped: boolean }
export interface AggregateLive { aggregateId: string; preferredMemberId: string | null; preferredIdleSecs: number | null; members: MemberLive[] }
export const aggregationLiveQueryKey = ["model-aggregation-live"] as const;
