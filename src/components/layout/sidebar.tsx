import { useTranslation } from "react-i18next";
import {
  LayoutDashboard,
  Wallet,
  ChartColumn,
  FileCode2,
  Server,
  Sparkles,
  Image as ImageIcon,
  MessagesSquare,
  Wrench,
  Settings,
  UsersRound,
  RadioTower,
  Sun,
  Moon,
  type LucideIcon,
} from "lucide-react";

import { type CSSProperties, useEffect, useLayoutEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useThemeValue, type Theme } from "@/hooks/use-theme";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { AppAvatarImage, useAppAvatar } from "@/hooks/use-app-avatar";
import { cn } from "@/lib/utils";
import type { Route } from "@/types/navigation";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from "@/components/ui/sidebar";
import { AnimatedSegmentedControl } from "@/components/ui/animated-segmented-control";
import { CLIENTS, useRoutingLinks } from "@/components/newapi/use-routing-links";

export const appNavItems: {
  route: Route;
  icon: LucideIcon;
  labelKey: string;
}[] = [
  { route: "overview", icon: LayoutDashboard, labelKey: "nav.overview" },
  { route: "accounts", icon: UsersRound, labelKey: "nav.accounts" },
  { route: "usage", icon: ChartColumn, labelKey: "nav.usage" },
  { route: "newapi", icon: Wallet, labelKey: "newapi.title" },
  { route: "relays", icon: RadioTower, labelKey: "newapi.providersSection" },
  { route: "imageWorkbench", icon: ImageIcon, labelKey: "image.title" },
  { route: "threads", icon: MessagesSquare, labelKey: "nav.threads" },
  { route: "customInstructions", icon: FileCode2, labelKey: "nav.customInstructions" },
  { route: "mcp", icon: Server, labelKey: "nav.mcp" },
  { route: "skills", icon: Sparkles, labelKey: "nav.skills" },
  { route: "maintenance", icon: Wrench, labelKey: "nav.maintenance" },
  { route: "settings", icon: Settings, labelKey: "nav.settings" },
];

const hiddenNavRoutes = new Set<Route>([]);
const navItemCount = appNavItems.filter((item) => !hiddenNavRoutes.has(item.route)).length;

/** Navigation reads as a table of contents: what you connect, what you work on, what you extend. */
const navGroups: { id: string; routes: Route[] }[] = [
  { id: "home", routes: ["overview", "usage"] },
  { id: "connect", routes: ["newapi", "relays", "accounts"] },
  { id: "work", routes: ["threads", "imageWorkbench"] },
  { id: "extend", routes: ["customInstructions", "mcp", "skills"] },
  { id: "system", routes: ["maintenance", "settings"] },
];

// The frame keeps 12px either side of the capsule: 172px open, 52px closed.
export const SIDEBAR_EXPANDED_WIDTH_PX = 196;
export const SIDEBAR_COLLAPSED_WIDTH_PX = 76;

const navButtonClassName =
  "app-nav-button relative overflow-hidden !rounded-[12px] group-data-[state=expanded]:!px-3 group-data-[state=expanded]:!py-2 group-data-[state=expanded]:gap-2.5 " +
  "md:group-data-[collapsible=icon]/sidebar:translate-x-0 hover:bg-sidebar-accent/70 " +
  // The gliding indicator paints the selection; the button itself stays clear.
  "data-[active=true]:!bg-transparent data-[active=true]:!text-foreground data-[active=true]:!shadow-none data-[active=true]:!ring-0 data-[active=true]:!outline-none " +
  // Keyboard focus only; mouse click should not leave a hollow accent ring.
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/35 focus-visible:ring-offset-1 focus-visible:ring-offset-card " +
  "transition-colors duration-150";

const footerNavButtonClassName = cn(
  navButtonClassName,
  "!h-8 min-h-0 shrink-0 group-data-[state=expanded]:!py-1.5",
);

const iconInactiveClass =
  "size-4 shrink-0 text-sidebar-foreground/80 group-hover/menu-item:text-sidebar-accent-foreground";

// Staggers the arrival of each row, header to footer.
const arrival = (index: number) => ({ "--nav-index": index } as CSSProperties);

type IndicatorFrame = { x: number; y: number; width: number; height: number; instant: boolean };

/** Places the highlight under the selected item. A new page glides there;
 *  first paint and resizes (window, expand/collapse) follow at once. Layout
 *  offsets, not client rects, so the rows' arrival transform cannot skew it.
 *  A callback ref, since the mobile sheet mounts its items only when opened. */
function useNavIndicator(activeRoute: Route) {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const [frame, setFrame] = useState<IndicatorFrame | null>(null);
  const measure = (instant: boolean) => {
    const button = container?.querySelector<HTMLElement>(`[data-nav-route="${activeRoute}"]`);
    if (!container || !button) { setFrame(null); return; }
    let x = 0, y = 0;
    for (let node: HTMLElement | null = button; node && node !== container; node = node.offsetParent as HTMLElement | null) {
      x += node.offsetLeft; y += node.offsetTop;
    }
    setFrame({ x, y, width: button.offsetWidth, height: button.offsetHeight, instant });
  };
  const measureRef = useRef(measure);
  measureRef.current = measure;
  const placedIn = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    if (!container) return;
    measure(placedIn.current !== container);
    placedIn.current = container;
  }, [activeRoute, container]);
  useEffect(() => {
    if (!container || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measureRef.current(true));
    observer.observe(container);
    return () => observer.disconnect();
  }, [container]);
  return { containerRef: setContainer, frame };
}

interface AppSidebarProps {
  activeRoute: Route;
  onNavigate: (route: Route) => void;
  onThemeChange: (theme: Theme) => void;
}

function ThemeGlyph({ resolved }: { resolved: "light" | "dark" }) {
  if (resolved === "dark") {
    return <Moon className={iconInactiveClass} strokeWidth={1.75} />;
  }
  return <Sun className={iconInactiveClass} strokeWidth={1.75} />;
}

function SidebarThemeToggle({
  resolvedTheme,
  onThemeChange,
  lightLabel,
  darkLabel,
  tooltipLabel,
}: {
  resolvedTheme: "light" | "dark";
  onThemeChange: (theme: Theme) => void;
  lightLabel: string;
  darkLabel: string;
  tooltipLabel: string;
}) {
  const tabs: { value: "light" | "dark"; label: string; icon: typeof Sun }[] = [
    { value: "light", label: lightLabel, icon: Sun },
    { value: "dark", label: darkLabel, icon: Moon },
  ];

  return (
    <SidebarMenuItem className="app-nav-item px-2 group-data-[collapsible=icon]/sidebar:px-0" style={arrival(navItemCount + 2)}>
      <div className="group-data-[collapsible=icon]/sidebar:hidden">
        <div className="rounded-full border border-sidebar-border/80 bg-sidebar-accent/45 p-1 dark:bg-white/[0.04]">
          <AnimatedSegmentedControl
            items={tabs}
            value={resolvedTheme}
            onValueChange={(nextTheme) => onThemeChange(nextTheme as "light" | "dark")}
            equalWidth
            className="gap-1"
            indicatorClassName="rounded-full bg-white shadow-[0_2px_10px_rgba(15,23,42,0.08)] dark:bg-white/[0.10]"
            itemClassName="h-8 gap-1.5 whitespace-nowrap rounded-full px-2.5 text-[13px] font-medium [&_svg]:size-4"
            activeItemClassName="text-foreground dark:text-white"
            inactiveItemClassName="text-sidebar-foreground/72 hover:text-sidebar-foreground dark:text-sidebar-foreground/72 dark:hover:text-sidebar-foreground"
          />
        </div>
      </div>

      <SidebarMenuButton
        tooltip={tooltipLabel}
        className={cn(footerNavButtonClassName, "hidden group-data-[collapsible=icon]/sidebar:flex")}
        onClick={() => onThemeChange(resolvedTheme === "dark" ? "light" : "dark")}
      >
        <ThemeGlyph resolved={resolvedTheme} />
        <span>{tooltipLabel}</span>
      </SidebarMenuButton>
    </SidebarMenuItem>
  );
}

export function AppSidebar({
  activeRoute,
  onNavigate,
  onThemeChange,
}: AppSidebarProps) {
  const { t } = useTranslation();
  const { isMobile, setOpenMobile } = useSidebar();
  const resolvedTheme = useThemeValue();
  const avatar = useAppAvatar();

  const handleNavigate = (route: Route) => {
    onNavigate(route);
    if (isMobile) setOpenMobile(false);
  };

  const themeLabel =
    resolvedTheme === "dark" ? t("nav.sidebarDarkTheme") : t("nav.sidebarLightTheme");
  // Routing at a glance from any page.
  const routing = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState, staleTime: 30_000 });
  // Any routed client counts, the same rule as the overview and the topology.
  const { links } = useRoutingLinks(routing.data?.router);
  const routeState = !routing.data?.proxyRunning ? "stopped"
    : CLIENTS.some((client) => links[client] === "routed") ? "routing" : "listening";
  const routeLabel = t(routeState === "routing" ? "newapi.topology.hubRouting"
    : routeState === "listening" ? "newapi.topology.hubListening" : "newapi.topology.hubStopped");

  const { containerRef, frame } = useNavIndicator(activeRoute);
  let navIndex = 0;

  return (
    <Sidebar collapsible="icon" variant="capsule">
      <SidebarHeader className="!p-0">
        <div className="app-nav-item flex w-full items-center gap-3 px-2.5 pb-1 pt-3 group-data-[collapsible=icon]/sidebar:justify-center group-data-[collapsible=icon]/sidebar:px-0"
          style={arrival(0)}>
          <button type="button" onClick={avatar.chooseImage} disabled={avatar.busy}
            aria-label={t("avatar.choose")} title={t("avatar.choose")}
            className="relative h-[35px] w-[35px] shrink-0 rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-50">
            <AppAvatarImage className="h-full w-full select-none rounded-full object-cover" />
            <span
              aria-hidden
              className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full bg-emerald-500 ring-2 ring-card"
            />
          </button>
          <button type="button" onClick={() => handleNavigate("overview")}
            className="app-nav-label flex min-w-0 flex-col items-start rounded-md text-left leading-tight hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring group-data-[collapsible=icon]/sidebar:hidden">
            <span className="truncate font-display text-[16px] font-bold tracking-tight text-sidebar-foreground">
              AgentHarbor
            </span>
            <span className="poster-kicker truncate text-[9.5px] text-sidebar-foreground/55">
              Codex Assistant
            </span>
          </button>
        </div>
      </SidebarHeader>
      <SidebarContent ref={containerRef}
        className="relative gap-0 pb-1 pt-2 group-data-[collapsible=icon]:!overflow-y-auto group-data-[collapsible=icon]:overflow-x-hidden">
        <span aria-hidden data-nav-indicator className="app-nav-indicator" data-instant={frame?.instant ? "" : undefined}
          style={frame ? { width: frame.width, height: frame.height, transform: `translate(${frame.x}px, ${frame.y}px)` } : { opacity: 0 }} />
        {navGroups.map((group) => (
          <SidebarGroup key={group.id} className="app-nav-group py-1">
            <SidebarMenu>
              {group.routes.filter((route) => !hiddenNavRoutes.has(route)).map((route) => {
                const item = appNavItems.find((entry) => entry.route === route);
                if (!item) return null;
                const { icon: Icon, labelKey } = item;
                const isActive = activeRoute === route;
                return (
                  <SidebarMenuItem key={route} className="app-nav-item" style={arrival(++navIndex)}>
                    <SidebarMenuButton
                      isActive={isActive}
                      tooltip={t(labelKey)}
                      className={navButtonClassName}
                      data-nav-route={route}
                      onClick={() => handleNavigate(route)}
                    >
                      <Icon
                        strokeWidth={1.75}
                        className={cn(
                          "size-4 shrink-0",
                          isActive
                            ? "text-primary"
                            : "text-sidebar-foreground/70 group-hover/menu-item:text-sidebar-accent-foreground",
                        )}
                      />
                      <span
                        className={cn(
                          "app-nav-label truncate transition-colors duration-150",
                          isActive && "font-semibold text-foreground",
                        )}
                      >
                        {t(labelKey)}
                      </span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                );
              })}
            </SidebarMenu>
          </SidebarGroup>
        ))}
      </SidebarContent>

      <SidebarFooter className="relative px-0 pb-3 pt-2 before:absolute before:inset-x-3 before:top-0 before:h-px before:bg-sidebar-border">
        <SidebarMenu className="!gap-1">
          <SidebarMenuItem className="app-nav-item px-2 group-data-[collapsible=icon]/sidebar:px-0" style={arrival(navItemCount + 1)}>
            <SidebarMenuButton tooltip={routeLabel} className={footerNavButtonClassName}
              onClick={() => handleNavigate("newapi")} data-sidebar-route-status={routeState}>
              {/* Not a span: closed, the button hides its direct spans (the labels). */}
              <i aria-hidden data-route-light className="relative flex size-4 shrink-0 items-center justify-center">
                <span className={cn("h-2 w-2 rounded-full",
                  routeState === "routing" ? "bg-emerald-500" : routeState === "listening" ? "bg-amber-500" : "bg-sidebar-foreground/35")} />
                {routeState === "routing" && <span className="absolute h-2 w-2 animate-ping rounded-full bg-emerald-500/60 motion-reduce:animate-none" />}
              </i>
              <span className="app-nav-label truncate text-[12px]">{routeLabel}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
          <SidebarThemeToggle
            resolvedTheme={resolvedTheme}
            onThemeChange={onThemeChange}
            lightLabel={t("settings.light")}
            darkLabel={t("settings.dark")}
            tooltipLabel={themeLabel}
          />
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
  );
}
