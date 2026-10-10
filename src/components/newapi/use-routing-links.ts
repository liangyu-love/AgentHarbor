import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import type { ModelTargetRef } from "@/types/aggregation";
import type { NewApiRouterView } from "@/types/newapi";
import type { ShortcutTarget } from "./configuration-shortcuts";

/** How a client currently reaches the relays. */
export type Link = "routed" | "direct" | "idle";
export type Client = Exclude<ShortcutTarget, "aggregation">;
export type Destination = "official" | "relays" | "aggregation";

export const CLIENTS: Client[] = ["codex", "claudeCode", "claudeDesktop", "opencode", "pi"];
export const DESTINATIONS: Destination[] = ["official", "relays", "aggregation"];

/** The status dot of a connection, shared by the topology and the client console. */
export const linkTone: Record<Link, string> = {
  routed: "bg-primary shadow-[0_0_0_3px_hsl(var(--primary)/0.18)]",
  direct: "bg-emerald-500 shadow-[0_0_0_3px_rgb(16_185_129/0.18)]",
  idle: "bg-muted-foreground/30",
};

/** Where a routed client's model targets point; legacy configs only name relays. */
function targetDestinations(targets: (ModelTargetRef | null | undefined)[]): Destination[] {
  const kinds = new Set(targets.filter((target): target is ModelTargetRef => !!target).map((target) => target.kind));
  const result: Destination[] = [];
  if (kinds.has("provider") || kinds.size === 0) result.push("relays");
  if (kinds.has("aggregate")) result.push("aggregation");
  return result;
}

// Each client's connection and the destinations it actually reaches. Shared by
// the routing topology and the dashboard so both always agree.
export function useRoutingLinks(router: NewApiRouterView | undefined) {
  const clients = useQuery({ queryKey: queryKeys.clientConfigs, queryFn: api.targets.configs });
  const desktop = useQuery({ queryKey: queryKeys.claudeDesktopStatus, queryFn: api.claudeDesktop.status,
    refetchOnWindowFocus: false });
  return useMemo(() => {
    const links = {} as Record<Client, Link>;
    const uses = {} as Record<Client, Destination[]>;
    links.codex = !router?.enabled ? "idle"
      : router.accessMode === "native" ? "direct"
      : router.configDrifted ? "idle" : "routed";
    // A selected aggregate replaces Codex's whole menu; otherwise the relays
    // are injected next to the official account models.
    uses.codex = links.codex === "direct" ? ["relays"]
      : router?.aggregateModelId ? ["aggregation"]
      : router?.accountless ? ["relays"] : ["official", "relays"];
    for (const target of ["claudeCode", "opencode", "pi"] as const) {
      const status = clients.data?.find((client) => client.targetId === target);
      const request = status?.request;
      links[target] = !status?.configured ? "idle" : request?.accessMode === "router" ? "routed" : "direct";
      uses[target] = links[target] === "routed"
        ? targetDestinations([...(request?.modelSelection?.availableTargets ?? []), request?.modelSelection?.defaultTarget])
        : ["relays"];
    }
    const desktopStatus = desktop.data;
    links.claudeDesktop = !desktopStatus?.isManaged ? "idle"
      : desktopStatus.mode === "router" ? "routed" : desktopStatus.mode === "direct" ? "direct" : "idle";
    uses.claudeDesktop = links.claudeDesktop === "routed"
      ? targetDestinations((desktopStatus?.routeViews ?? []).map((view) => view.targetRef))
      : ["relays"];
    return { links, uses };
  }, [router, clients.data, desktop.data]);
}
