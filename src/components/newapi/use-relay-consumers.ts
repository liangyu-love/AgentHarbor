import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { NewApiProvider, NewApiState } from "@/types/newapi";
import type { ModelTargetRef } from "@/types/aggregation";
import type { ShortcutTarget } from "./configuration-shortcuts";

export type RelayConsumer = Exclude<ShortcutTarget, "aggregation">;
/** Who sends traffic to one relay, and how Codex reaches it. */
export type RelayUse = { consumers: RelayConsumer[]; codex: "injected" | "direct" | null };

/** The router injects every enabled relay with a key and a model into Codex's menu. */
export const codexInjectable = (provider: NewApiProvider) => provider.enabled && (provider.hasApiKey || !!provider.accountSource) && !!provider.model;

const providerTargets = (targets: (ModelTargetRef | null | undefined)[]) =>
  targets.flatMap((target) => target?.kind === "provider" ? [target.providerId] : []);

// Derived from the same client queries as the routing topology, so the relay
// cards and the topology always agree on who uses which relay.
export function useRelayConsumers(state: NewApiState | undefined) {
  const clients = useQuery({ queryKey: queryKeys.clientConfigs, queryFn: api.targets.configs });
  const desktop = useQuery({ queryKey: queryKeys.claudeDesktopStatus, queryFn: api.claudeDesktop.status,
    refetchOnWindowFocus: false });
  return useMemo(() => {
    const result = new Map<string, RelayUse>();
    const entry = (id: string) => {
      let use = result.get(id);
      if (!use) result.set(id, use = { consumers: [], codex: null });
      return use;
    };
    const add = (id: string, consumer: RelayConsumer) => {
      const use = entry(id);
      if (!use.consumers.includes(consumer)) use.consumers.push(consumer);
    };

    const router = state?.router;
    if (router?.enabled && !router.configDrifted) {
      if (router.accessMode === "native") {
        if (state?.activeProviderId) {
          entry(state.activeProviderId).codex = "direct";
          add(state.activeProviderId, "codex");
        }
      } else if (!router.aggregateModelId) {
        for (const provider of state?.providers ?? []) {
          if (!codexInjectable(provider)) continue;
          entry(provider.id).codex = "injected";
          add(provider.id, "codex");
        }
      }
    }

    for (const status of clients.data ?? []) {
      if (!status.configured || status.targetId === "codex") continue;
      const request = status.request;
      const ids = !request ? [status.providerId]
        : request.accessMode === "native" ? [request.providerId]
        : request.modelSelection
          ? providerTargets([...request.modelSelection.availableTargets, request.modelSelection.defaultTarget])
          : request.providerIds;
      for (const id of ids) if (id) add(id, status.targetId as RelayConsumer);
    }

    const desktopStatus = desktop.data;
    if (desktopStatus?.isManaged) {
      if (desktopStatus.mode === "direct" && desktopStatus.providerId) add(desktopStatus.providerId, "claudeDesktop");
      if (desktopStatus.mode === "router") {
        for (const view of desktopStatus.routeViews ?? []) {
          if (!view.targetRef || view.targetRef.kind === "provider") add(view.providerId, "claudeDesktop");
        }
      }
    }
    return result;
  }, [state, clients.data, desktop.data]);
}

export type RelayConsumers = ReturnType<typeof useRelayConsumers>;
