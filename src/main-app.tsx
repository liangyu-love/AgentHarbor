import { Suspense, lazy, useCallback, useEffect, useState, type CSSProperties } from "react";
import { QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useTheme, useThemeValue, type Theme } from "@/hooks/use-theme";
import { AppAvatarProvider } from "@/hooks/use-app-avatar";
import { useAccentColor } from "@/hooks/use-accent-color";
import { useAutoRefresh } from "@/hooks/use-auto-refresh";
import { announceUpdateCheck, useUpdateCheck, type UpdateCheckResult } from "@/hooks/use-update-check";
import { toast } from "@/hooks/use-toast";
import { formatInvokeError } from "@/lib/invoke-error";
import { useDeferredReady } from "@/hooks/use-deferred-ready";
import { useRouteTransition } from "@/hooks/use-route-transition";
import { OPEN_TERMINAL_PROXY_EVENT } from "@/lib/terminal-proxy-link";
import { PageStage } from "@/components/layout/page-stage";
import {
  AppSidebar,
  appNavItems,
  SIDEBAR_COLLAPSED_WIDTH_PX,
  SIDEBAR_EXPANDED_WIDTH_PX,
} from "@/components/layout/sidebar";
import { SiteHeader } from "@/components/site-header";
import { CommandPalette } from "@/components/command-palette/command-palette";
import { SidebarInset, SidebarProvider } from "@/components/ui/sidebar";
import { Skeleton } from "@/components/ui/skeleton";
import { Toaster } from "@/components/ui/toaster";
import { UpdateOverlay } from "@/components/update/update-overlay";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { createAppQueryClient } from "@/lib/query-client";
import { api } from "@/lib/api";
import { isMacPlatform } from "@/lib/platform";
import { cn } from "@/lib/utils";
import {
  CustomBackgroundProvider,
  CustomBackgroundLayer,
  useCustomBackground,
} from "@/hooks/use-custom-background";
import { isAppRoute, type Route } from "@/types/navigation";
import "./lib/i18n";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";

const McpPage = lazy(() =>
  import("@/components/mcp/mcp-page").then((module) => ({ default: module.McpPage })),
);
const OverviewPage = lazy(() => import("@/components/overview/overview-page").then((module) => ({ default: module.OverviewPage })));
const AccountsPage = lazy(() => import("@/components/accounts/accounts-page").then((module) => ({ default: module.AccountsPage })));
const UsagePage = lazy(() => import("@/components/usage/usage-page").then((module) => ({ default: module.UsagePage })));
const NewApiPage = lazy(() => import("@/components/newapi/newapi-page").then((module) => ({ default: module.NewApiPage })));
const RelayModelsPage = lazy(() => import("@/components/newapi/relays-page").then((module) => ({ default: module.RelayModelsPage })));
const ImageWorkbenchPage = lazy(() => import("@/components/image-workbench/image-workbench-page").then((module) => ({ default: module.ImageWorkbenchPage })));
const ThreadsPage = lazy(() =>
  import("@/components/threads/threads-page").then((module) => ({ default: module.ThreadsPage })),
);
const SkillsPage = lazy(() =>
  import("@/components/skills/skills-page").then((module) => ({ default: module.SkillsPage })),
);
const CustomInstructionsPage = lazy(() =>
  import("@/components/custom-instructions/custom-instructions-page").then((module) => ({ default: module.CustomInstructionsPage })),
);
const MaintenancePage = lazy(() =>
  import("@/components/maintenance/maintenance-page").then((module) => ({ default: module.MaintenancePage })),
);
const SettingsPage = lazy(() =>
  import("@/components/settings/settings-page").then((module) => ({ default: module.SettingsPage })),
);

const DRAG_REGION_HEIGHT = isMacPlatform() ? 48 : 0;
const queryClient = createAppQueryClient();

export function MainAppRoot() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={200}>
        <AppAvatarProvider>
          <CustomBackgroundProvider>
            <MainApp />
          </CustomBackgroundProvider>
        </AppAvatarProvider>
      </TooltipProvider>
    </QueryClientProvider>
  );
}

function MainApp() {
  const [route, setRoute] = useState<Route>("overview");
  const { theme, setTheme } = useTheme();
  const resolvedTheme = useThemeValue();
  const { activeBackgroundStyle } = useCustomBackground();
  const hasCustomBackground = Boolean(activeBackgroundStyle);
  const { accent, setAccent } = useAccentColor();
  const { i18n, t } = useTranslation();
  const [sidebarOpen, setSidebarOpen] = useState(
    () => localStorage.getItem(storageKeys.sidebarCollapsed) === "false",
  );
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [proxySettingsRequested, setProxySettingsRequested] = useState(false);
  const { refreshInterval, setRefreshInterval } = useAutoRefresh();
  const update = useUpdateCheck();
  const showUpdateOverlay =
    update.status === "available" ||
    update.status === "downloading" ||
    update.status === "installing" ||
    update.status === "ready" ||
    update.status === "error";

  const installLocationPrompt = useInstallLocationPrompt();
  const routeTransition = useRouteTransition(route, { durationMs: 300 });
  const [imageWorkbenchVisited, setImageWorkbenchVisited] = useState(route === "imageWorkbench");
  useEffect(() => {
    if (route === "imageWorkbench") setImageWorkbenchVisited(true);
  }, [route]);

  const handleThemeChange = useCallback((nextTheme: Theme) => {
    setTheme(nextTheme);
  }, [setTheme]);
  const openProxySettings = useCallback(() => {
    setProxySettingsRequested(true);
    setRoute("settings");
  }, []);
  const clearProxySettingsRequest = useCallback(() => setProxySettingsRequested(false), []);
  const [terminalProxyRequested, setTerminalProxyRequested] = useState(false);
  useEffect(() => {
    const open = () => {
      setTerminalProxyRequested(true);
      setRoute("settings");
    };
    window.addEventListener(OPEN_TERMINAL_PROXY_EVENT, open);
    return () => window.removeEventListener(OPEN_TERMINAL_PROXY_EVENT, open);
  }, []);
  const clearTerminalProxyRequest = useCallback(() => setTerminalProxyRequested(false), []);

  useAccountsChangedListener();
  useTrayRequests(setRoute, update.checkForUpdate);
  useEffect(() => {
    const sync = (language: string) => { void api.setUiLanguage(language).catch(() => {}); };
    sync(i18n.language);
    i18n.on("languageChanged", sync);
    return () => { i18n.off("languageChanged", sync); };
  }, [i18n]);

  // Global Ctrl+K / Cmd+K listener
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setCommandPaletteOpen((prev) => !prev);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const prewarmRoutes = useDeferredReady(900);
  useEffect(() => {
    if (prewarmRoutes) {
      void Promise.allSettled([
        import("@/components/mcp/mcp-page"),
        import("@/components/accounts/accounts-page"),
        import("@/components/usage/usage-page"),
        import("@/components/newapi/relays-page"),
        import("@/components/image-workbench/image-workbench-page"),
        import("@/components/skills/skills-page"),
        import("@/components/custom-instructions/custom-instructions-page"),
        import("@/components/maintenance/maintenance-page"),
        import("@/components/settings/settings-page"),
      ]);
    }
  }, [prewarmRoutes]);

  const renderPage = (targetRoute: Route) => {
    switch (targetRoute) {
      case "overview":
        return <OverviewPage onNavigate={setRoute} onOpenProxySettings={openProxySettings} />;
      case "accounts":
        return <AccountsPage refreshInterval={refreshInterval} />;
      case "usage":
        return <UsagePage refreshInterval={refreshInterval} onManageProviders={() => setRoute("relays")}
          onOpenThreads={client => {
            // The threads page opens on the source the session belongs to.
            try { localStorage.setItem(storageKeys.threadsSource, client); } catch { /* storage unavailable */ }
            setRoute("threads");
          }} />;
      case "newapi":
        return <NewApiPage onOpenRelays={() => setRoute("relays")} />;
      case "relays":
        return <RelayModelsPage />;
      case "imageWorkbench":
        return <ImageWorkbenchPage />;
      case "threads":
        return <ThreadsPage />;
      case "mcp":
        return <McpPage />;
      case "skills":
        return <SkillsPage />;
      case "customInstructions":
        return <CustomInstructionsPage />;
      case "maintenance":
        return <MaintenancePage />;
      case "settings":
        return (
          <SettingsPage
            theme={theme}
            onThemeChange={handleThemeChange}
            accent={accent}
            setAccent={setAccent}
            language={i18n.language}
            setLanguage={(lang) => {
              i18n.changeLanguage(lang);
              localStorage.setItem(storageKeys.language, lang);
            }}
            refreshInterval={refreshInterval}
            setRefreshInterval={setRefreshInterval}
            onCheckUpdate={update.checkForUpdate}
            openProxySettingsRequested={proxySettingsRequested}
            onProxySettingsRequestHandled={clearProxySettingsRequest}
            terminalProxyRequested={terminalProxyRequested}
            onTerminalProxyRequestHandled={clearTerminalProxyRequest}
          />
        );
      default:
        return null;
    }
  };

  const routeLabelKey = appNavItems.find((item) => item.route === route)?.labelKey ?? "nav.overview";

  const routeOrder: Route[] = [
    "overview",
    "accounts",
    "usage",
    "newapi",
    "relays",
    "imageWorkbench",
    "threads",
    "customInstructions",
    "mcp",
    "skills",
    "maintenance",
    "settings",
  ];

  return (
    <div
      className={cn(
        "flex h-screen w-screen overflow-hidden transition-colors",
        hasCustomBackground ? "bg-transparent" : "bg-[#FFFFFF] dark:bg-background",
      )}
    >
      <CustomBackgroundLayer />
      <div
        className="fixed inset-x-0 top-0 z-[60]"
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag", height: DRAG_REGION_HEIGHT } as CSSProperties}
      />

      <SidebarProvider
        open={sidebarOpen}
        onOpenChange={(open) => {
          setSidebarOpen(open);
          localStorage.setItem(storageKeys.sidebarCollapsed, String(!open));
        }}
        style={
          {
            "--sidebar-width": `${SIDEBAR_EXPANDED_WIDTH_PX}px`,
            "--sidebar-width-icon": `${SIDEBAR_COLLAPSED_WIDTH_PX}px`,
            // The capsule stays clear of the macOS traffic lights.
            "--app-drag-height": `${DRAG_REGION_HEIGHT}px`,
          } as CSSProperties
        }
        className="flex min-h-0 flex-1 overflow-hidden"
      >
        <AppSidebar
          activeRoute={route}
          onNavigate={setRoute}
          onThemeChange={handleThemeChange}
        />
        <SidebarInset
          className={cn(
            "max-h-screen overflow-hidden transition-colors",
            hasCustomBackground && "!bg-transparent",
          )}
        >
          <SiteHeader
            title={t(routeLabelKey)}
            onOpenSpotlight={() => setCommandPaletteOpen(true)}
            onThemeToggle={() => handleThemeChange(resolvedTheme === "dark" ? "light" : "dark")}
            isDarkMode={resolvedTheme === "dark"}
            hasCustomBackground={hasCustomBackground}
          />
          <div className="relative min-h-0 flex-1 overflow-hidden">
            {routeOrder
              // Keep canvases and in-flight generation jobs alive after leaving the page.
              .filter((candidate) => routeTransition.mountedRoutes.includes(candidate)
                || (candidate === "imageWorkbench" && imageWorkbenchVisited))
              .map((candidate) => (
                <PageStage
                  key={candidate}
                  state={routeTransition.getStage(candidate)}
                >
                  <Suspense fallback={<PageShellSkeleton />}>
                    {renderPage(candidate)}
                  </Suspense>
                </PageStage>
              ))}
          </div>
        </SidebarInset>
      </SidebarProvider>

      <CommandPalette
        open={commandPaletteOpen}
        onOpenChange={setCommandPaletteOpen}
        onNavigate={setRoute}
        onThemeToggle={() => handleThemeChange(resolvedTheme === "dark" ? "light" : "dark")}
        isDarkMode={resolvedTheme === "dark"}
      />

      <Toaster />
      <InstallLocationPromptDialog prompt={installLocationPrompt} />
      {showUpdateOverlay && !installLocationPrompt.open && (
        <UpdateOverlay
          status={update.status as "checking" | "available" | "downloading" | "installing" | "ready" | "error"}
          currentVersion={update.updateInfo?.currentVersion ?? "0.0.0"}
          newVersion={update.updateInfo?.version}
          body={update.updateInfo?.body}
          progress={update.progress}
          error={update.error}
          onInstall={update.installUpdate}
          onRetry={update.checkForUpdate}
          onSkip={update.dismiss}
        />
      )}
    </div>
  );
}

/** The auto-switch loop refreshes quotas and may switch accounts in the background. */
function useAccountsChangedListener() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("codex-accounts-changed", () => {
      // Read the stored result instead of refetching, which would query every account again.
      void api.accounts.list()
        .then((data) => queryClient.setQueryData(queryKeys.codexAccounts, data))
        .catch(() => undefined);
      void queryClient.invalidateQueries({ queryKey: queryKeys.runtimeState });
    })
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [queryClient]);
}

/** What the tray asks of the window: a page, an update check, or news of a switch it flipped. */
function useTrayRequests(navigate: (route: Route) => void, checkForUpdate: () => Promise<UpdateCheckResult>) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  useEffect(() => {
    const stops = [
      listen<string>("tray-navigate", ({ payload }) => {
        if (isAppRoute(payload)) navigate(payload);
      }),
      listen("tray-check-update", () => {
        void checkForUpdate().then(result => announceUpdateCheck(t, result));
      }),
      listen<{ switch: "cursor" | "autostart"; error: string | null }>("tray-switched", ({ payload }) => {
        const cursor = payload.switch === "cursor";
        void queryClient.invalidateQueries({ queryKey: cursor ? queryKeys.cursorStatus : queryKeys.autostart });
        if (payload.error) {
          toast({
            title: t(cursor ? "cursor.failed" : "settings.autostart.failed"),
            description: formatInvokeError(payload.error),
            variant: "destructive",
          });
        }
      }),
    ];
    const removals = stops.map(stop => stop.catch(() => undefined));
    return () => {
      for (const removal of removals) void removal.then(remove => remove?.());
    };
  }, [navigate, checkForUpdate, queryClient, t]);
}

function useInstallLocationPrompt() {
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void api
      .checkUpdateInstallability()
      .then((payload) => {
        if (cancelled) return;
        if (payload.code === "app_translocation" || payload.code === "read_only_location") {
          setOpen(true);
        }
      })
      .catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, []);

  const openApplications = useCallback(() => {
    void api.openPath("/Applications");
  }, []);

  const dismiss = useCallback(() => {
    setOpen(false);
  }, []);

  return {
    open,
    openApplications,
    dismiss,
  };
}

function InstallLocationPromptDialog({
  prompt,
}: {
  prompt: ReturnType<typeof useInstallLocationPrompt>;
}) {
  const { t } = useTranslation();

  return (
    <AlertDialog open={prompt.open}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("update.installPromptTitle")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("update.installPromptDesc")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={prompt.dismiss}>
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction onClick={prompt.openApplications}>
            {t("update.openApplications")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function PageShellSkeleton() {
  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <Skeleton className="h-6 w-32" />
        <Skeleton className="h-4 w-72" />
      </div>
      <div className="rounded-2xl border border-border bg-card p-6">
        <div className="space-y-4">
          {Array.from({ length: 5 }).map((_, index) => (
            <div key={index} className="flex items-center justify-between border-b border-border/60 pb-4 last:border-b-0">
              <div className="space-y-2">
                <Skeleton className="h-4 w-36" />
                <Skeleton className="h-3 w-56" />
              </div>
              <Skeleton className="h-8 w-20 rounded-xl" />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
