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
  SidebarGroupLabel,
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

/** Navigation reads as a table of contents: what you connect, what you work on, what you extend. */
const navGroups: { kicker: string; routes: Route[] }[] = [
  { kicker: "Home", routes: ["overview", "usage"] },
  { kicker: "Connect", routes: ["newapi", "relays", "accounts"] },
  { kicker: "Work", routes: ["threads", "imageWorkbench"] },
  { kicker: "Extend", routes: ["customInstructions", "mcp", "skills"] },
  { kicker: "System", routes: ["maintenance", "settings"] },
];

export const SIDEBAR_EXPANDED_WIDTH_PX = 176 * 1.05;
export const SIDEBAR_COLLAPSED_WIDTH_PX = 64;

const SIDEBAR_LOGO_TOP_OFFSET_PX = -21;

const navButtonClassName =
  "app-nav-button relative overflow-hidden group-data-[state=expanded]:!rounded-[10px] group-data-[state=expanded]:!px-3 group-data-[state=expanded]:!py-2 group-data-[state=expanded]:gap-2.5 " +
  // Selected = filled pill. Kill any leftover ring/border that reads as a hollow orange box.
  // Selected = a raised sheet with a short accent rule; no glow.
  "data-[active=true]:!border-transparent data-[active=true]:!bg-background data-[active=true]:!text-foreground " +
  "data-[active=true]:!shadow-[0_0_0_1px_hsl(var(--sidebar-border)),0_1px_2px_hsl(var(--foreground)/0.06)] data-[active=true]:!ring-0 data-[active=true]:!outline-none " +
  "dark:data-[active=true]:!bg-white/[0.07] " +
  "data-[active=true]:before:absolute data-[active=true]:before:left-0 data-[active=true]:before:top-1/2 data-[active=true]:before:-translate-y-1/2 " +
  "data-[active=true]:before:h-4 data-[active=true]:before:w-[3px] data-[active=true]:before:rounded-full data-[active=true]:before:bg-primary " +
  "data-[active=true]:before:content-[''] " +
  "group-data-[collapsible=icon]/sidebar:data-[active=true]:before:hidden " +
  // Keyboard focus only; mouse click should not leave a hollow accent ring.
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/35 focus-visible:ring-offset-1 focus-visible:ring-offset-sidebar " +
  "transition-colors duration-150";

const footerNavButtonClassName = cn(
  navButtonClassName,
  "!h-7 min-h-0 shrink-0 group-data-[state=expanded]:!py-1.5",
);

const iconInactiveClass =
  "size-4 shrink-0 text-sidebar-foreground/80 group-hover/menu-item:text-sidebar-accent-foreground";

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
    <SidebarMenuItem className="px-2 group-data-[collapsible=icon]/sidebar:px-0">
      <div className="group-data-[collapsible=icon]/sidebar:hidden">
        <div className="rounded-[8px] border border-sidebar-border/80 bg-sidebar-accent/45 p-1 shadow-[0_10px_22px_rgba(15,23,42,0.06)] dark:bg-white/[0.04] dark:shadow-none">
          <AnimatedSegmentedControl
            items={tabs}
            value={resolvedTheme}
            onValueChange={(nextTheme) => onThemeChange(nextTheme as "light" | "dark")}
            equalWidth
            className="gap-1"
            indicatorClassName="rounded-[8px] bg-white shadow-[0_2px_10px_rgba(15,23,42,0.08)] dark:bg-white/[0.10]"
            itemClassName="h-8 gap-1.5 whitespace-nowrap rounded-[8px] px-2.5 text-[13px] font-medium [&_svg]:size-4"
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

  return (
    <Sidebar collapsible="icon" variant="inset">
      <SidebarHeader className="!p-0">
        <div className="h-12 shrink-0" data-tauri-drag-region />
        <div
          className="hidden justify-center group-data-[collapsible=icon]/sidebar:flex"
          style={{ marginTop: SIDEBAR_LOGO_TOP_OFFSET_PX }}
        >
          <button type="button" onClick={avatar.chooseImage} disabled={avatar.busy}
            aria-label={t("avatar.choose")} title={t("avatar.choose")}
            className="rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-50 md:translate-x-1">
            <AppAvatarImage className="h-[35px] w-[35px] select-none rounded-full object-cover" />
          </button>
        </div>
        <div
          className="flex w-full items-center gap-3 rounded-[10px] pl-2.5 pr-3 py-1 text-left group-data-[collapsible=icon]/sidebar:hidden"
          style={{ marginTop: SIDEBAR_LOGO_TOP_OFFSET_PX }}
        >
          <button type="button" onClick={avatar.chooseImage} disabled={avatar.busy}
            aria-label={t("avatar.choose")} title={t("avatar.choose")}
            className="relative h-[35px] w-[35px] shrink-0 rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:opacity-50">
            <AppAvatarImage className="h-full w-full select-none rounded-full object-cover" />
            <span
              aria-hidden
              className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full bg-emerald-500 ring-2 ring-sidebar"
            />
          </button>
          <button type="button" onClick={() => handleNavigate("overview")}
            className="flex min-w-0 flex-col items-start rounded-md text-left leading-tight hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
            <span className="truncate font-display text-[16px] font-bold tracking-tight text-sidebar-foreground">
              AgentHarbor
            </span>
            <span className="poster-kicker truncate text-[9.5px] text-sidebar-foreground/55">
              Codex Assistant
            </span>
          </button>
        </div>
      </SidebarHeader>
      <SidebarContent className="gap-0 pt-3">
        {navGroups.map((group) => (
          <SidebarGroup key={group.kicker} className="py-1">
            <SidebarGroupLabel className="poster-kicker h-6 px-3 text-[9px] text-sidebar-foreground/45">{group.kicker}</SidebarGroupLabel>
            <SidebarMenu>
              {group.routes.filter((route) => !hiddenNavRoutes.has(route)).map((route) => {
                const item = appNavItems.find((entry) => entry.route === route);
                if (!item) return null;
                const { icon: Icon, labelKey } = item;
                const isActive = activeRoute === route;
                return (
                  <SidebarMenuItem key={route}>
                    <SidebarMenuButton
                      isActive={isActive}
                      tooltip={t(labelKey)}
                      className={navButtonClassName}
                      onClick={() => handleNavigate(route)}
                    >
                      <Icon
                        strokeWidth={1.75}
                        className={cn(
                          "size-4 shrink-0 transition-colors duration-150",
                          isActive
                            ? "text-primary"
                            : "text-sidebar-foreground/70 group-hover/menu-item:text-sidebar-accent-foreground",
                        )}
                      />
                      <span
                        className={cn(
                          "truncate transition-colors duration-150",
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

      <SidebarFooter className="border-t border-sidebar-border px-0 pt-2 pb-2">
        <SidebarMenu className="!gap-1">
          <SidebarMenuItem className="px-2 group-data-[collapsible=icon]/sidebar:px-0">
            <SidebarMenuButton tooltip={routeLabel} className={cn(footerNavButtonClassName, "!h-8")}
              onClick={() => handleNavigate("newapi")} data-sidebar-route-status={routeState}>
              <span aria-hidden className={cn("relative flex size-4 shrink-0 items-center justify-center")}>
                <span className={cn("h-2 w-2 rounded-full",
                  routeState === "routing" ? "bg-emerald-500" : routeState === "listening" ? "bg-amber-500" : "bg-sidebar-foreground/35")} />
                {routeState === "routing" && <span className="absolute h-2 w-2 animate-ping rounded-full bg-emerald-500/60 motion-reduce:animate-none" />}
              </span>
              <span className="truncate text-[12px]">{routeLabel}</span>
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
