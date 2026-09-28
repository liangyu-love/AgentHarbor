import { useQuery } from "@tanstack/react-query";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { Separator } from "@/components/ui/separator";
import { Button } from "@/components/ui/button";
import { Search, Sun, Moon, FolderOpen } from "lucide-react";
import { isMacPlatform } from "@/lib/platform";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";

interface SiteHeaderProps {
  title: string;
  onOpenSpotlight?: () => void;
  onThemeToggle?: () => void;
  isDarkMode?: boolean;
  hasCustomBackground?: boolean;
}

export function SiteHeader({
  title,
  onOpenSpotlight,
  onThemeToggle,
  isDarkMode,
  hasCustomBackground,
}: SiteHeaderProps) {
  const newapi = useQuery({
    queryKey: queryKeys.providers,
    queryFn: api.providers.loadState,
    staleTime: 5000,
  });

  const snapshot = useQuery({
    queryKey: queryKeys.runtimeStateDisplay,
    queryFn: () => api.loadSnapshot(true),
    staleTime: 10000,
  });

  const proxyRunning = newapi.data?.proxyRunning ?? false;
  const codexHome = snapshot.data?.data.status.paths.codexHome ?? "";
  const isMac = isMacPlatform();

  const handleOpenCodexHome = async () => {
    if (!codexHome) return;
    try {
      await api.openPath(codexHome);
      toast({ title: ".codex 目录已打开", variant: "default" });
    } catch (err) {
      toast({ title: "打开失败", description: formatInvokeError(err), variant: "destructive" });
    }
  };

  return (
    <header
      className={cn(
        "flex h-12 shrink-0 items-center justify-between border-b px-4 gap-3 transition-colors",
        hasCustomBackground
          ? "border-border/50 bg-background/60 backdrop-blur-md"
          : "border-border/70 bg-background/95 backdrop-blur-md",
      )}
    >
      {/* Left: Sidebar toggle + Title */}
      <div className="flex items-center gap-2 min-w-0">
        <SidebarTrigger className="-ml-1 text-muted-foreground hover:text-foreground" />
        <Separator orientation="vertical" className="mr-1 !h-4" />
        <h1 className="text-sm font-semibold tracking-tight truncate text-foreground/90">{title}</h1>
      </div>

      {/* Center: Spotlight Search Trigger */}
      {onOpenSpotlight && (
        <button
          type="button"
          onClick={onOpenSpotlight}
          className="app-search-trigger hidden md:flex items-center gap-2 rounded-lg border border-border/80 bg-muted/40 hover:bg-muted px-3 py-1.5 text-xs text-muted-foreground transition-all hover:border-primary/40 hover:text-foreground group shadow-2xs max-w-xs w-full justify-between cursor-pointer"
          title="打开全局命令搜索 (Spotlight)"
          aria-label="打开全局命令搜索"
        >
          <div className="flex items-center gap-2 truncate">
            <Search className="h-3.5 w-3.5 text-muted-foreground group-hover:text-primary transition-colors shrink-0" />
            <span className="truncate">全局指令搜索 / 快速跳转...</span>
          </div>
          <kbd className="h-4.5 px-1.5 rounded border border-border/60 bg-card font-mono text-[10px] text-muted-foreground shrink-0">
            {isMac ? "⌘K" : "Ctrl K"}
          </kbd>
        </button>
      )}

      {/* Right: Proxy Status & Quick Tools */}
      <div className="flex items-center gap-2 shrink-0">
        {proxyRunning && (
          <div className="hidden sm:flex items-center gap-1.5 rounded-full border border-emerald-500/25 bg-emerald-500/10 px-2.5 py-0.5 text-[11px] font-medium text-emerald-600 dark:text-emerald-400">
            <span className="relative flex h-2 w-2">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
              <span className="relative inline-flex h-2 w-2 rounded-full bg-emerald-500" />
            </span>
            <span>本地代理在线</span>
          </div>
        )}

        {codexHome && (
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-8 w-8 text-muted-foreground hover:text-foreground"
            onClick={handleOpenCodexHome}
            title="打开 .codex 根目录"
          >
            <FolderOpen className="h-4 w-4" />
          </Button>
        )}

        {onThemeToggle && (
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-8 w-8 text-muted-foreground hover:text-foreground active:scale-95 transition-transform"
            onClick={onThemeToggle}
            title={isDarkMode ? "切换为浅色模式" : "切换为深色模式"}
          >
            {isDarkMode ? (
              <Sun className="h-4 w-4 transition-transform duration-300 rotate-0 hover:rotate-45" />
            ) : (
              <Moon className="h-4 w-4 transition-transform duration-300 -rotate-12 hover:rotate-0" />
            )}
          </Button>
        )}
      </div>
    </header>
  );
}
