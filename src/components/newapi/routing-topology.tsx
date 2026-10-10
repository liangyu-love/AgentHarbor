import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { ArrowUpRight, RadioTower, RefreshCw, Waypoints } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { aggregationQueryKey } from "@/types/aggregation";
import type { ProviderState } from "./use-provider-state";
import { ClientGlyph, shortcutEntries, useSectionJump } from "./configuration-shortcuts";
import { CLIENTS, DESTINATIONS, useRoutingLinks, type Client, type Destination, type Link } from "./use-routing-links";

type Point = { x: number; y: number };
type Box = { left: number; right: number; top: number; bottom: number; cx: number; cy: number };
type Route = { id: string; d: string; link: Exclude<Link, "idle"> | "idle"; length: number };

const VERTICAL_BELOW = 720;

function curve(a: Point, b: Point, vertical: boolean) {
  if (vertical) {
    const dy = (b.y - a.y) / 2;
    return `M${a.x},${a.y} C${a.x},${a.y + dy} ${b.x},${b.y - dy} ${b.x},${b.y}`;
  }
  const dx = (b.x - a.x) / 2;
  return `M${a.x},${a.y} C${a.x + dx},${a.y} ${b.x - dx},${b.y} ${b.x},${b.y}`;
}

/** A direct connection bypasses the hub instead of crossing it. */
function bypass(a: Point, b: Point, hub: Box, vertical: boolean) {
  if (vertical) {
    const x = hub.right + 28;
    return `M${a.x},${a.y} C${a.x},${a.y + 40} ${x},${hub.cy - 50} ${x},${hub.cy} S${b.x},${b.y - 40} ${b.x},${b.y}`;
  }
  const y = hub.bottom + 26;
  return `M${a.x},${a.y} C${a.x + 60},${a.y} ${hub.cx - 90},${y} ${hub.cx},${y} S${b.x - 60},${b.y} ${b.x},${b.y}`;
}

function approximateLength(d: string) {
  const numbers = d.match(/-?\d+(\.\d+)?/g)?.map(Number) ?? [];
  let length = 0;
  for (let index = 2; index + 1 < numbers.length; index += 2) {
    length += Math.hypot(numbers[index] - numbers[index - 2], numbers[index + 1] - numbers[index - 1]);
  }
  return Math.max(length, 1);
}

function usePrefersReducedMotion() {
  const [reduced, setReduced] = useState(() => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false);
  useEffect(() => {
    const query = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    if (!query) return;
    const update = () => setReduced(query.matches);
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
  return reduced;
}

const linkTone: Record<Link, string> = {
  routed: "bg-primary shadow-[0_0_0_3px_hsl(var(--primary)/0.18)]",
  direct: "bg-emerald-500 shadow-[0_0_0_3px_rgb(16_185_129/0.18)]",
  idle: "bg-muted-foreground/30",
};

// The hero of the injection page: which client reaches which destination,
// and how. Every client node is also a shortcut to its configuration.
export function RoutingTopology({
  store,
  onOpenRelays,
  onRestart,
}: {
  store: ProviderState;
  onOpenRelays?: () => void;
  onRestart: () => void;
}) {
  const { t } = useTranslation();
  const { active, jump } = useSectionJump();
  const reduced = usePrefersReducedMotion();
  const gradient = useId().replace(/:/g, "");
  const aggregates = useQuery({ queryKey: aggregationQueryKey, queryFn: api.aggregation.state });

  const data = store.state.data;
  const router = data?.router;
  const providers = data?.providers ?? [];
  const enabledRelays = providers.filter((provider) => provider.enabled);
  const enabledAggregates = (aggregates.data?.models ?? []).filter((model) => model.enabled);

  const { links, uses } = useRoutingLinks(router);

  const routedClients = CLIENTS.filter((client) => links[client] === "routed");
  const hubActive = !!data?.proxyRunning && routedClients.length > 0;
  const reaches = (client: Client, destination: Destination) => links[client] === "routed" && uses[client].includes(destination);
  // A destination lights up only when a routed client actually sends to it.
  const destinationActive: Record<Destination, boolean> = {
    official: hubActive && CLIENTS.some((client) => reaches(client, "official")),
    relays: enabledRelays.length > 0 && (hubActive && CLIENTS.some((client) => reaches(client, "relays"))
      || CLIENTS.some((client) => links[client] === "direct")),
    aggregation: hubActive && enabledAggregates.length > 0 && CLIENTS.some((client) => reaches(client, "aggregation")),
  };
  const hubFeeds: Record<Destination, boolean> = {
    official: destinationActive.official,
    relays: hubActive && enabledRelays.length > 0 && CLIENTS.some((client) => reaches(client, "relays")),
    aggregation: destinationActive.aggregation,
  };

  // Light the hub up again whenever routing turns on.
  const [ignition, setIgnition] = useState(0);
  const wasActive = useRef(hubActive);
  useEffect(() => {
    if (hubActive && !wasActive.current) setIgnition((value) => value + 1);
    wasActive.current = hubActive;
  }, [hubActive]);

  // Measure node positions to draw the connections between them.
  const container = useRef<HTMLDivElement>(null);
  const nodes = useRef<Record<string, HTMLElement | null>>({});
  const [layout, setLayout] = useState<{ width: number; height: number; vertical: boolean; boxes: Record<string, Box> } | null>(null);
  useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    const measure = () => {
      const origin = element.getBoundingClientRect();
      const boxes: Record<string, Box> = {};
      for (const [key, node] of Object.entries(nodes.current)) {
        if (!node) continue;
        const rect = node.getBoundingClientRect();
        const left = rect.left - origin.left;
        const top = rect.top - origin.top;
        boxes[key] = { left, top, right: left + rect.width, bottom: top + rect.height,
          cx: left + rect.width / 2, cy: top + rect.height / 2 };
      }
      setLayout({ width: origin.width, height: origin.height, vertical: origin.width < VERTICAL_BELOW, boxes });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const routes = useMemo<Route[]>(() => {
    const hub = layout?.boxes.hub;
    if (!layout || !hub) return [];
    const { vertical, boxes } = layout;
    const out = (box: Box): Point => vertical ? { x: box.cx, y: box.bottom } : { x: box.right, y: box.cy };
    const into = (box: Box): Point => vertical ? { x: box.cx, y: box.top } : { x: box.left, y: box.cy };
    const result: Route[] = [];
    CLIENTS.forEach((client, index) => {
      const box = boxes[client];
      if (!box) return;
      // Fan the client lines into the hub instead of stacking them on one point.
      const spread = (index - (CLIENTS.length - 1) / 2) * 7;
      const target = vertical ? { x: hub.cx + spread, y: hub.top } : { x: hub.left + 2, y: hub.cy + spread };
      const link = links[client];
      if (link === "direct" && boxes.relays) {
        const d = bypass(out(box), into(boxes.relays), hub, vertical);
        result.push({ id: `${client}-relays`, d, link, length: approximateLength(d) });
      } else {
        const d = curve(out(box), target, vertical);
        result.push({ id: `${client}-hub`, d, link: link === "routed" ? "routed" : "idle", length: approximateLength(d) });
      }
    });
    DESTINATIONS.forEach((destination, index) => {
      const box = boxes[destination];
      if (!box) return;
      const spread = (index - 1) * 8;
      const source = vertical ? { x: hub.cx + spread, y: hub.bottom } : { x: hub.right - 2, y: hub.cy + spread };
      const d = curve(source, into(box), vertical);
      result.push({ id: `hub-${destination}`, d, link: hubFeeds[destination] ? "routed" : "idle",
        length: approximateLength(d) });
    });
    return result;
    // hubFeeds is derived from the same inputs listed here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, links, hubFeeds.official, hubFeeds.relays, hubFeeds.aggregation]);

  // Hovering or focusing a node highlights only the paths through it.
  const [hovered, setHovered] = useState<string | null>(null);
  const related = (routeId: string) => {
    if (!hovered) return true;
    const [from, to] = routeId.split("-") as [string, string];
    if (hovered === "hub") return from === "hub" || to === "hub";
    if ((CLIENTS as string[]).includes(hovered)) {
      const client = hovered as Client;
      return from === client || (from === "hub" && reaches(client, to as Destination));
    }
    const destination = hovered as Destination;
    return to === destination || (to === "hub" && reaches(from as Client, destination));
  };
  const highlight = (key: string) => ({
    onMouseEnter: () => setHovered(key),
    onMouseLeave: () => setHovered(null),
    onFocus: () => setHovered(key),
    onBlur: () => setHovered(null),
  });
  const linkLabel = (link: Link) => t(`newapi.topology.link.${link}`);
  const register = (key: string) => (node: HTMLElement | null) => { nodes.current[key] = node; };
  const vertical = layout?.vertical ?? false;

  const clientNode = (client: Client) => {
    const link = links[client];
    return <button key={client} ref={register(client)} type="button" data-route-shortcut={client} {...highlight(client)}
      aria-label={t(shortcutEntries[client].label)} title={t(shortcutEntries[client].label)}
      aria-current={active === client ? "location" : undefined} onClick={() => jump(client)}
      className={cn("route-node group relative z-10 flex min-w-0 items-center gap-2.5 rounded-xl border bg-card/85 px-2.5 py-2 text-left backdrop-blur-sm transition-[border-color,box-shadow,transform] duration-200 hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none motion-reduce:hover:translate-y-0",
        link === "routed" ? "border-primary/35 shadow-[0_8px_24px_-16px_hsl(var(--primary)/0.8)]" : "border-border/70",
        active === client && "ring-2 ring-primary/40")}>
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-background ring-1 ring-border/80">
        <ClientGlyph client={client} className="h-[18px] w-[18px]" />
      </span>
      <span className="route-node-text min-w-0 flex-1">
        <span className="block truncate font-display text-[13.5px] font-semibold tracking-tight text-foreground">{t(`newapi.topology.client.${client}`)}</span>
        <span className={cn("block truncate text-[11px]", vertical && "hidden",
          link === "routed" ? "text-primary" : link === "direct" ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground")}>
          {link === "routed"
            ? t("newapi.topology.routedTo", { targets: uses[client].map((destination) => t(`newapi.topology.dest.${destination}`)).join(" + ") })
            : linkLabel(link)}
        </span>
      </span>
      <span aria-hidden className={cn("route-node-dot h-2 w-2 shrink-0 rounded-full transition-colors", linkTone[link])} />
    </button>;
  };

  const destinationShell = (activeState: boolean) => cn(
    "route-node relative z-10 flex min-w-0 items-center gap-2.5 rounded-xl border bg-card/85 px-2.5 py-2 text-left backdrop-blur-sm transition-[border-color,box-shadow,transform] duration-200",
    activeState ? "border-primary/35 shadow-[0_8px_24px_-16px_hsl(var(--primary)/0.8)]" : "border-border/70",
  );
  const destinationIcon = (activeState: boolean) => cn("flex h-8 w-8 shrink-0 items-center justify-center rounded-lg ring-1",
    activeState ? "bg-primary/10 text-primary ring-primary/25" : "bg-background text-muted-foreground ring-border/80");

  const restartPending = !!router?.restartPending;

  return (
    <section aria-labelledby="routing-topology-title"
      className="route-surface route-rise relative overflow-hidden rounded-3xl border border-border/80 bg-card shadow-sm">
      <span aria-hidden className="poster-watermark right-[-0.03em] top-[-0.16em] text-[clamp(5rem,13vw,10rem)]">ROUTING</span>
      <div className="relative flex flex-wrap items-start justify-between gap-3 px-5 pt-5 sm:px-6">
        <div className="min-w-0">
          <h2 id="routing-topology-title" className="poster-title text-2xl">
            {t("newapi.topology.title")}
          </h2>
          <p className="mt-0.5 text-[13px] text-muted-foreground">{t("newapi.topology.subtitle")}</p>
        </div>
        <div className="flex flex-wrap items-center gap-1.5 text-[11px]">
          {(["routed", "direct", "idle"] as Link[]).map((link) => (
            <span key={link} className="inline-flex items-center gap-1.5 rounded-full border border-border/70 bg-background/70 px-2 py-0.5 text-muted-foreground">
              <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", linkTone[link])} />
              {linkLabel(link)}
            </span>
          ))}
        </div>
      </div>

      <nav aria-label={t("newapi.configShortcuts")}>
        <div ref={container} data-topology-layout={vertical ? "vertical" : "horizontal"}
          className={cn("relative grid gap-x-10 px-5 pb-6 pt-5 sm:px-6",
            vertical ? "grid-cols-1 gap-y-12" : "grid-cols-[minmax(0,12.5rem)_minmax(10rem,1fr)_minmax(0,13.5rem)] items-center gap-y-0")}>
          <svg aria-hidden className="pointer-events-none absolute inset-0 h-full w-full overflow-visible">
            <defs>
              <linearGradient id={`${gradient}-routed`} gradientUnits="userSpaceOnUse"
                x1="0" y1="0" x2={vertical ? 0 : layout?.width ?? 1} y2={vertical ? layout?.height ?? 1 : 0}>
                <stop offset="0%" stopColor="hsl(var(--primary))" />
                <stop offset="55%" stopColor="hsl(262 83% 66%)" />
                <stop offset="100%" stopColor="hsl(190 95% 50%)" />
              </linearGradient>
              <linearGradient id={`${gradient}-direct`} gradientUnits="userSpaceOnUse"
                x1="0" y1="0" x2={vertical ? 0 : layout?.width ?? 1} y2={vertical ? layout?.height ?? 1 : 0}>
                <stop offset="0%" stopColor="rgb(16 185 129)" />
                <stop offset="100%" stopColor="rgb(45 212 191)" />
              </linearGradient>
              <filter id={`${gradient}-glow`} x="-50%" y="-50%" width="200%" height="200%">
                <feGaussianBlur stdDeviation="3.2" />
              </filter>
            </defs>
            {routes.map((route) => {
              const live = route.link !== "idle";
              const stroke = `url(#${gradient}-${route.link === "direct" ? "direct" : "routed"})`;
              const duration = Math.max(1.6, route.length / 140);
              return (
                <g key={route.id} data-route={route.id} data-route-state={route.link}
                  className="transition-opacity duration-300"
                  opacity={related(route.id) ? 1 : 0.18}>
                  <path d={route.d} fill="none" className="stroke-border" strokeWidth={1.25}
                    strokeDasharray={live ? undefined : "3 6"} opacity={live ? 0.35 : 0.9} />
                  {live && <>
                    <path d={route.d} fill="none" stroke={stroke} strokeWidth={5} opacity={0.22} filter={`url(#${gradient}-glow)`} />
                    <path key={`draw-${ignition}`} d={route.d} fill="none" stroke={stroke} strokeWidth={1.75}
                      strokeLinecap="round" pathLength={1} className="route-draw" />
                    <path d={route.d} fill="none" stroke="white" strokeOpacity={0.75} strokeWidth={1.5}
                      strokeLinecap="round" className={route.link === "direct" ? "route-flow-slow" : "route-flow"} />
                    {!reduced && [0, 0.5].map((offset) => (
                      <circle key={`${offset}-${route.d}`} r={2.6} fill={stroke} filter={`url(#${gradient}-glow)`}>
                        <animateMotion dur={`${duration}s`} begin={`${-offset * duration}s`} repeatCount="indefinite" path={route.d} />
                      </circle>
                    ))}
                    {!reduced && [0, 0.5].map((offset) => (
                      <circle key={`core-${offset}-${route.d}`} r={1.4} fill="white">
                        <animateMotion dur={`${duration}s`} begin={`${-offset * duration}s`} repeatCount="indefinite" path={route.d} />
                      </circle>
                    ))}
                  </>}
                </g>
              );
            })}
          </svg>

          <div className={cn("grid min-w-0 gap-2", vertical && "grid-cols-3")}>
            {CLIENTS.map(clientNode)}
          </div>

          <div className="relative z-10 flex flex-col items-center gap-3">
            <div ref={register("hub")} {...highlight("hub")} className="relative flex h-36 w-36 items-center justify-center">
              <span aria-hidden className={cn("absolute -inset-6 rounded-full blur-2xl transition-opacity duration-700",
                hubActive ? "bg-[radial-gradient(circle,hsl(var(--primary)/0.35),hsl(262_83%_66%/0.18)_55%,transparent_75%)] opacity-100" : "opacity-0")} />
              {hubActive && <>
                <span aria-hidden className="route-hub-halo absolute inset-0 rounded-full border border-primary/40" />
                <span aria-hidden className="route-hub-halo absolute inset-0 rounded-full border border-primary/40" />
              </>}
              <span aria-hidden className={cn("absolute inset-0 rounded-full",
                hubActive ? "route-hub-ring" : "border border-dashed border-border")} />
              <div key={ignition} className={cn("relative flex h-[7.25rem] w-[7.25rem] flex-col items-center justify-center rounded-full border bg-card/95 text-center shadow-lg backdrop-blur",
                hubActive ? "route-hub-ignite border-primary/30 shadow-primary/20" : "border-border/80")}>
                <Waypoints aria-hidden className={cn("h-7 w-7 transition-colors", hubActive ? "text-primary" : "text-muted-foreground")} />
                <span className="mt-1 font-display text-[13px] font-bold tracking-tight text-foreground">AgentHarbor</span>
                <span className="font-mono text-[10px] text-muted-foreground">:{router?.port ?? 43127}</span>
              </div>
            </div>
            <div className="flex flex-wrap items-center justify-center gap-1.5 text-[11px]">
              <span data-router-status={hubActive ? "routing" : data?.proxyRunning ? "listening" : "stopped"}
                className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 font-medium",
                  hubActive ? "bg-primary/10 text-primary" : "bg-muted text-muted-foreground")}>
                <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", hubActive ? "bg-primary" : "bg-muted-foreground/50")} />
                {t(hubActive ? "newapi.topology.hubRouting" : data?.proxyRunning ? "newapi.topology.hubListening" : "newapi.topology.hubStopped")}
              </span>
              {restartPending && (
                <button type="button" onClick={onRestart} data-restart-pending
                  className="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2 py-0.5 font-medium text-amber-700 transition-colors hover:bg-amber-500/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring dark:text-amber-300">
                  <span aria-hidden className="route-pending-dot h-1.5 w-1.5 rounded-full bg-amber-500" />
                  {t("newapi.topology.restartPending")}
                  <RefreshCw aria-hidden className="h-3 w-3" />
                </button>
              )}
            </div>
          </div>

          <div className={cn("grid min-w-0 gap-2", vertical && "grid-cols-3")}>
            <div ref={register("official")} data-route-destination="official" {...highlight("official")} className={destinationShell(destinationActive.official)}>
              <span className={destinationIcon(destinationActive.official)}>
                <ClientGlyph client="codex" className="h-[18px] w-[18px]" />
              </span>
              <span className="route-node-text min-w-0 flex-1">
                <span className="block truncate text-[13px] font-medium">{t("newapi.topology.official")}</span>
                <span className={cn("block truncate text-[11px] text-muted-foreground", vertical && "hidden")}>
                  {t(router?.accountless ? "newapi.topology.officialOff" : "newapi.topology.officialAccount")}
                </span>
              </span>
            </div>
            <button ref={register("relays")} type="button" data-route-destination="relays" onClick={onOpenRelays} {...highlight("relays")}
              disabled={!onOpenRelays} title={t("newapi.providersSection")}
              className={cn(destinationShell(destinationActive.relays), "group hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:hover:translate-y-0 motion-reduce:hover:translate-y-0")}>
              <span className={destinationIcon(destinationActive.relays)}>
                <RadioTower aria-hidden className="h-4 w-4" />
              </span>
              <span className="route-node-text min-w-0 flex-1">
                <span className="block truncate text-[13px] font-medium">{t("newapi.topology.relays", { count: enabledRelays.length })}</span>
                <span className={cn("block truncate text-[11px] text-muted-foreground", vertical && "hidden")}>
                  {enabledRelays.length ? enabledRelays.slice(0, 3).map((provider) => provider.name).join(" · ") : t("newapi.topology.noRelays")}
                </span>
              </span>
              {onOpenRelays && <ArrowUpRight aria-hidden className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform group-hover:-translate-y-0.5 group-hover:translate-x-0.5", vertical && "hidden")} />}
            </button>
            <button ref={register("aggregation")} type="button" data-route-shortcut="aggregation" {...highlight("aggregation")}
              aria-label={t(shortcutEntries.aggregation.label)} title={t(shortcutEntries.aggregation.label)}
              aria-current={active === "aggregation" ? "location" : undefined} onClick={() => jump("aggregation")}
              className={cn(destinationShell(destinationActive.aggregation), "hover:-translate-y-0.5 hover:border-primary/40 hover:shadow-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:hover:translate-y-0",
                active === "aggregation" && "ring-2 ring-primary/40")}>
              <span className={destinationIcon(destinationActive.aggregation)}>
                <ClientGlyph client="aggregation" className="h-4 w-4" />
              </span>
              <span className="route-node-text min-w-0 flex-1">
                <span className="block truncate text-[13px] font-medium">{t("newapi.topology.aggregates", { count: enabledAggregates.length })}</span>
                <span className={cn("block truncate text-[11px] text-muted-foreground", vertical && "hidden")}>
                  {enabledAggregates.length ? enabledAggregates.slice(0, 2).map((model) => model.publicModelId).join(" · ") : t("newapi.topology.noAggregates")}
                </span>
              </span>
            </button>
          </div>
        </div>
      </nav>
    </section>
  );
}
