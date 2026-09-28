import { useState, useMemo, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { BentoCard } from "@/components/ui/bento-card";
import { AuroraCard } from "@/components/ui/aurora-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
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
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
  PaginationEllipsis,
} from "@/components/ui/pagination";
import { Switch } from "@/components/ui/switch";
import {
  Server,
  Plus,
  Pencil,
  Trash2,
  RotateCw,
  Copy,
  Search,
  Terminal,
  Globe,
  Radio,
  Key,
  Layers,
  FolderOpen,
  Check,
  X,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { toast } from "@/hooks/use-toast";
import type { McpServerSummary, McpTransport } from "@/types";
import { useBusyAction } from "@/hooks/use-busy-action";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";

const transportStyles: Record<string, { dot: string; text: string }> = {
  stdio: { dot: "bg-blue-500 shadow-[0_0_0_2px_rgba(59,130,246,0.2)]", text: "text-blue-500" },
  http: { dot: "bg-violet-500 shadow-[0_0_0_2px_rgba(139,92,246,0.2)]", text: "text-violet-500" },
  sse: { dot: "bg-amber-500 shadow-[0_0_0_2px_rgba(245,158,11,0.2)]", text: "text-amber-500" },
};

const transportTile: Record<string, string> = {
  stdio: "bg-blue-500/10 ring-blue-500/15 text-blue-500 dark:text-blue-400",
  http: "bg-violet-500/10 ring-violet-500/15 text-violet-500 dark:text-violet-400",
  sse: "bg-amber-500/10 ring-amber-500/15 text-amber-500 dark:text-amber-400",
};

function DotBadge({ dotClass, textClass, children }: { dotClass: string; textClass: string; children: React.ReactNode }) {
  return (
    <Badge variant="outline" className={cn("gap-1.5 pl-2 pr-2.5 py-0.5 text-[11px] font-medium", textClass)}>
      <span className={cn("inline-block h-1.5 w-1.5 rounded-full shrink-0", dotClass)} />
      {children}
    </Badge>
  );
}

export function McpPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [editing, setEditing] = useState<McpServerSummary | "new" | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [currentPage, setCurrentPage] = useState(1);
  const [searchQuery, setSearchQuery] = useState("");
  const [transportFilter, setTransportFilter] = useState<"all" | "enabled" | "disabled" | "stdio" | "http">("all");
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  const PAGE_SIZE = 12;

  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const { data, refetch, error, isPending } = useQuery({
    queryKey: ["mcp-servers"],
    queryFn: () => api.loadMcpServers(),
    staleTime: Infinity,
  });

  const refreshBusy = refreshAction.busy;

  const handleRefresh = async () => {
    await refreshAction.run(async () => {
      await refetch();
    });
  };

  const toggleMutation = useMutation({
    mutationFn: ({ name, enabled }: { name: string; enabled: boolean }) =>
      api.setMcpServerEnabled(name, enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["mcp-servers"] }),
  });

  const removeMutation = useMutation({
    mutationFn: (name: string) => api.removeMcpServer(name),
    onSuccess: () => {
      setRemoving(null);
      queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
    },
  });

  const servers = data?.data.items ?? [];
  const enabledCount = servers.filter((s) => s.enabled).length;
  const disabledCount = servers.length - enabledCount;
  const stdioCount = servers.filter((s) => s.transport === "stdio").length;
  const remoteCount = servers.length - stdioCount;
  const sourcePath = data?.data.sourcePath ?? "";

  const copyPath = () => {
    if (!sourcePath) return;
    navigator.clipboard.writeText(sourcePath);
    toast({
      title: t("mcp.pathCopied"),
      description: t("mcp.pathCopiedDesc"),
      variant: "default",
    });
  };

  const handleOpenConfigFolder = async () => {
    if (!sourcePath) return;
    try {
      const parts = sourcePath.split(/[/\\]/);
      parts.pop();
      const dir = parts.join(sourcePath.includes("\\") ? "\\" : "/");
      await api.openPath(dir || sourcePath);
    } catch (err) {
      toast({
        title: t("common.error"),
        description: String(err),
        variant: "destructive",
      });
    }
  };

  const handleCopyCommand = (server: McpServerSummary) => {
    let text = "";
    if (server.transport === "stdio") {
      text = [server.command, ...(server.args || [])].filter(Boolean).join(" ");
    } else {
      text = server.url ?? "";
    }
    navigator.clipboard.writeText(text);
    setCopiedCommand(server.name);
    setTimeout(() => setCopiedCommand(null), 1800);
    toast({
      title: t("mcp.commandCopied"),
      variant: "default",
    });
  };

  const filteredServers = useMemo(() => {
    return servers.filter((server) => {
      if (transportFilter === "enabled" && !server.enabled) return false;
      if (transportFilter === "disabled" && server.enabled) return false;
      if (transportFilter === "stdio" && server.transport !== "stdio") return false;
      if (transportFilter === "http" && server.transport === "stdio") return false;

      if (!searchQuery.trim()) return true;
      const q = searchQuery.toLowerCase().trim();
      const matchName = server.name.toLowerCase().includes(q);
      const matchCmd = (server.command ?? "").toLowerCase().includes(q);
      const matchArgs = (server.args ?? []).some((a) => a.toLowerCase().includes(q));
      const matchUrl = (server.url ?? "").toLowerCase().includes(q);
      return matchName || matchCmd || matchArgs || matchUrl;
    });
  }, [servers, transportFilter, searchQuery]);

  const totalPages = Math.max(1, Math.ceil(filteredServers.length / PAGE_SIZE));
  const safePage = Math.min(currentPage, totalPages);
  const pagedServers = filteredServers.slice((safePage - 1) * PAGE_SIZE, safePage * PAGE_SIZE);

  const paginationRange = useMemo(() => {
    const range: (number | "ellipsis")[] = [];
    if (totalPages <= 7) {
      for (let i = 1; i <= totalPages; i++) range.push(i);
    } else {
      range.push(1);
      if (safePage > 3) range.push("ellipsis");
      const start = Math.max(2, safePage - 1);
      const end = Math.min(totalPages - 1, safePage + 1);
      for (let i = start; i <= end; i++) range.push(i);
      if (safePage < totalPages - 2) range.push("ellipsis");
      range.push(totalPages);
    }
    return range;
  }, [totalPages, safePage]);

  if (error) return <div role="alert" className="space-y-3 p-4"><p>{t("baseline.readFailed")}</p><p className="text-sm text-muted-foreground">{String(error)}</p><Button onClick={() => void refetch()}>{t("common.refresh")}</Button></div>;
  if (isPending) return <p role="status" className="p-4 text-muted-foreground">{t("baseline.loading")}</p>;

  return (
    <TooltipProvider delayDuration={150}>
      <div className="space-y-6">
        {/* Page header */}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h1 className="text-xl font-bold tracking-tight text-foreground">{t("mcp.title")}</h1>
            <p className="mt-1 text-sm text-muted-foreground">{t("mcp.description")}</p>
          </div>
          <div className="flex items-center gap-2">
            <RainbowButton size="sm" className="h-9 rounded-lg font-medium" onClick={() => setEditing("new")}>
              <Plus className="h-4 w-4" />
              {t("mcp.addServer")}
            </RainbowButton>
            <Button
              variant="outline"
              size="icon"
              className="h-9 w-9"
              onClick={handleRefresh}
              disabled={refreshBusy}
              aria-busy={refreshBusy}
              title={refreshBusy ? t("common.refreshing") : t("common.refresh")}
            >
              <ButtonBusyContent
                busy={refreshBusy}
                idleIcon={<RotateCw className="h-4 w-4" />}
              />
            </Button>
          </div>
        </div>

        {/* Stats & Config path banner */}
        <div className="app-metric-grid grid grid-cols-1 gap-3 lg:grid-cols-3">
          <AuroraCard color="blue">
            <div className="p-4 flex items-center justify-between">
              <div>
                <span className="text-xs font-medium text-muted-foreground">{t("mcp.serverCount")}</span>
                <div className="mt-1 flex items-baseline gap-2">
                  <span className="text-2xl font-bold tracking-tight tabular-nums">{servers.length}</span>
                  <span className="text-xs text-muted-foreground font-mono">
                    ({stdioCount} stdio · {remoteCount} http)
                  </span>
                </div>
              </div>
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-blue-500/10 text-blue-500 ring-1 ring-inset ring-blue-500/20">
                <Server className="h-5 w-5" />
              </div>
            </div>
          </AuroraCard>

          <AuroraCard color="green">
            <div className="p-4 flex items-center justify-between">
              <div>
                <span className="text-xs font-medium text-muted-foreground">{t("mcp.enabledCount")}</span>
                <div className="mt-1 flex items-baseline gap-2">
                  <span className="text-2xl font-bold tracking-tight text-emerald-600 dark:text-emerald-400 tabular-nums">
                    {enabledCount}
                  </span>
                  <span className="text-xs text-muted-foreground font-mono">
                    / {servers.length} 运行中
                  </span>
                </div>
              </div>
              <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-500 ring-1 ring-inset ring-emerald-500/20">
                <Terminal className="h-5 w-5" />
              </div>
            </div>
          </AuroraCard>

          <AuroraCard color="blue">
            <div className="p-4 flex flex-col justify-between h-full">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-muted-foreground">{t("mcp.configFile")}</span>
                <div className="flex items-center gap-1">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-6 w-6 text-muted-foreground hover:text-foreground"
                    title={t("mcp.openFolder")}
                    onClick={handleOpenConfigFolder}
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="h-6 w-6 text-muted-foreground hover:text-foreground"
                    title={t("mcp.copyCommand")}
                    onClick={copyPath}
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
              <div
                className="mt-1 flex items-center gap-1.5 cursor-pointer rounded px-1.5 py-1 -mx-1.5 transition-colors hover:bg-black/[0.04] dark:hover:bg-white/[0.04]"
                onClick={copyPath}
                title={sourcePath}
              >
                <span className="truncate font-mono text-xs font-medium text-foreground/80">{sourcePath}</span>
              </div>
            </div>
          </AuroraCard>
        </div>

        {/* Search & Filter Toolbar */}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="relative min-w-[240px] max-w-sm flex-1">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground pointer-events-none" />
            <Input
              value={searchQuery}
              onChange={(e) => {
                setSearchQuery(e.target.value);
                setCurrentPage(1);
              }}
              placeholder={t("mcp.searchPlaceholder")}
              className="pl-9 pr-8 h-9 text-sm rounded-lg"
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => {
                  setSearchQuery("");
                  setCurrentPage(1);
                }}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground p-0.5 rounded"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>

          <div className="flex items-center gap-1.5 overflow-x-auto pb-0.5">
            {[
              { key: "all", label: `${t("mcp.filterAll")} (${servers.length})` },
              { key: "enabled", label: `${t("mcp.filterEnabled")} (${enabledCount})` },
              { key: "disabled", label: `${t("mcp.filterDisabled")} (${disabledCount})` },
              { key: "stdio", label: `STDIO (${stdioCount})` },
              { key: "http", label: `HTTP/SSE (${remoteCount})` },
            ].map((f) => {
              const active = transportFilter === f.key;
              return (
                <button
                  key={f.key}
                  type="button"
                  onClick={() => {
                    setTransportFilter(f.key as typeof transportFilter);
                    setCurrentPage(1);
                  }}
                  className={cn(
                    "flex shrink-0 items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-all shadow-xs",
                    active
                      ? "border-primary/50 bg-primary/10 text-primary font-semibold ring-1 ring-primary/20"
                      : "border-border/60 bg-card text-muted-foreground hover:bg-muted/70 hover:text-foreground"
                  )}
                >
                  {f.label}
                </button>
              );
            })}
          </div>
        </div>

        {/* Server Grid */}
        {servers.length === 0 ? (
          <BentoCard>
            <div className="flex h-56 flex-col items-center justify-center text-center p-6">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/20 to-primary/5 ring-1 ring-inset ring-primary/15">
                <Server className="h-6 w-6 text-primary" />
              </div>
              <p className="mt-3.5 text-base font-semibold text-foreground">{t("mcp.empty")}</p>
              <p className="mt-1 text-xs text-muted-foreground max-w-sm">
                直接添加本地进程或远程 MCP 端点，Codex 即可在对话中随时调用工具能力。
              </p>
              <RainbowButton size="sm" className="mt-4 rounded-lg" onClick={() => setEditing("new")}>
                <Plus className="h-3.5 w-3.5" />
                {t("mcp.addServer")}
              </RainbowButton>
            </div>
          </BentoCard>
        ) : filteredServers.length === 0 ? (
          <BentoCard>
            <div className="flex h-44 flex-col items-center justify-center text-center p-6">
              <Search className="h-8 w-8 text-muted-foreground/60" />
              <p className="mt-2.5 text-sm font-medium text-muted-foreground">{t("mcp.noFilterResults")}</p>
              <Button
                variant="outline"
                size="sm"
                className="mt-3"
                onClick={() => {
                  setSearchQuery("");
                  setTransportFilter("all");
                }}
              >
                {t("mcp.clearSearch")}
              </Button>
            </div>
          </BentoCard>
        ) : (
          <>
            <div className="grid gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
              {pagedServers.map((server) => {
                const isStdio = server.transport === "stdio";
                const cmdString = isStdio
                  ? [server.command, ...(server.args || [])].filter(Boolean).join(" ")
                  : server.url ?? "";
                const envKeys = Object.keys(server.environment || {});
                const headerKeys = Object.keys(server.headers || {});
                const hasEnv = envKeys.length > 0;
                const hasHeaders = headerKeys.length > 0;
                const isCopied = copiedCommand === server.name;

                const transportIcon = isStdio ? (
                  <Terminal className="h-4 w-4" />
                ) : server.transport === "http" ? (
                  <Globe className="h-4 w-4" />
                ) : (
                  <Radio className="h-4 w-4" />
                );

                return (
                  <BentoCard
                    key={server.name}
                    compact
                    className={cn(
                      "group flex flex-col justify-between transition-all duration-200 hover:border-primary/40 hover:shadow-md",
                      !server.enabled && "opacity-80 dark:opacity-75 bg-card/60"
                    )}
                  >
                    <div>
                      {/* Card Header: Icon + Name + Switch */}
                      <div className="flex items-center justify-between gap-2.5">
                        <div className="flex items-center gap-2.5 min-w-0 flex-1">
                          <div
                            className={cn(
                              "flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ring-1 ring-inset transition-transform group-hover:scale-105",
                              transportTile[server.transport] ?? transportTile.stdio
                            )}
                          >
                            {transportIcon}
                          </div>
                          <div className="min-w-0 flex-1">
                            <h2
                              className="text-[14px] font-bold tracking-tight text-foreground truncate"
                              title={server.name}
                            >
                              {server.name}
                            </h2>
                            <div className="mt-0.5 flex items-center gap-1.5">
                              <DotBadge
                                dotClass={(transportStyles[server.transport] ?? transportStyles.stdio).dot}
                                textClass={(transportStyles[server.transport] ?? transportStyles.stdio).text}
                              >
                                {server.transport.toUpperCase()}
                              </DotBadge>
                              <DotBadge
                                dotClass={server.enabled
                                  ? "bg-emerald-500 shadow-[0_0_0_2px_rgba(16,185,129,0.2)]"
                                  : "bg-zinc-400 dark:bg-zinc-600"}
                                textClass={server.enabled ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground"}
                              >
                                {server.enabled ? t("mcp.enabled") : t("mcp.disabled")}
                              </DotBadge>
                            </div>
                          </div>
                        </div>

                        {/* Always visible Switch */}
                        <Switch
                          checked={server.enabled}
                          onCheckedChange={(v) => toggleMutation.mutate({ name: server.name, enabled: v })}
                          aria-label={`${server.name} status`}
                        />
                      </div>

                      {/* Monospace Command / URL Box */}
                      <div className="relative mt-3 rounded-lg border border-border/70 bg-muted/50 dark:bg-black/40 px-2.5 py-2 font-mono text-[11px] leading-relaxed">
                        <div className="flex items-center justify-between gap-2">
                          <span
                            className="min-w-0 flex-1 truncate text-foreground/85 select-all"
                            title={cmdString}
                          >
                            {isStdio ? `> ${cmdString}` : cmdString}
                          </span>
                          <button
                            type="button"
                            onClick={() => handleCopyCommand(server)}
                            className="shrink-0 p-1 rounded hover:bg-background/80 text-muted-foreground hover:text-foreground transition-colors"
                            title={t("mcp.copyCommand")}
                          >
                            {isCopied ? (
                              <Check className="h-3 w-3 text-emerald-500" />
                            ) : (
                              <Copy className="h-3 w-3" />
                            )}
                          </button>
                        </div>
                      </div>

                      {/* Metadata Badges: Env & Headers */}
                      <div className="mt-2.5 flex flex-wrap items-center gap-1.5 min-h-[22px]">
                        {hasEnv ? (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <div className="inline-flex items-center gap-1 rounded-md border border-amber-500/25 bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-600 dark:text-amber-400 cursor-default">
                                <Key className="h-3 w-3" />
                                <span>{envKeys.length} ENV</span>
                              </div>
                            </TooltipTrigger>
                            <TooltipContent className="text-left font-mono text-[11px] max-w-xs">
                              <p className="font-sans font-semibold mb-1 text-xs">{t("mcp.envTooltip")}:</p>
                              <div className="flex flex-col gap-0.5">
                                {envKeys.map((k) => (
                                  <span key={k} className="truncate">
                                    • {k}
                                  </span>
                                ))}
                              </div>
                            </TooltipContent>
                          </Tooltip>
                        ) : (
                          <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/50 px-1">
                            <Key className="h-3 w-3 opacity-40" />
                            <span>{t("mcp.noEnv")}</span>
                          </span>
                        )}

                        {hasHeaders && (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <div className="inline-flex items-center gap-1 rounded-md border border-violet-500/25 bg-violet-500/10 px-2 py-0.5 text-[11px] font-medium text-violet-600 dark:text-violet-400 cursor-default">
                                <Layers className="h-3 w-3" />
                                <span>{headerKeys.length} Headers</span>
                              </div>
                            </TooltipTrigger>
                            <TooltipContent className="text-left font-mono text-[11px] max-w-xs">
                              <p className="font-sans font-semibold mb-1 text-xs">{t("mcp.headersTooltip")}:</p>
                              <div className="flex flex-col gap-0.5">
                                {headerKeys.map((h) => (
                                  <span key={h} className="truncate">
                                    • {h}
                                  </span>
                                ))}
                              </div>
                            </TooltipContent>
                          </Tooltip>
                        )}
                      </div>
                    </div>

                    {/* Card Footer: Protocol info + Edit/Delete actions */}
                    <div className="mt-3.5 pt-2.5 border-t border-border/50 flex items-center justify-between">
                      <span className="text-[11px] text-muted-foreground font-mono">
                        {isStdio ? "stdio process" : server.transport.toUpperCase()}
                      </span>
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => setEditing(server)}
                          className="h-7 w-7 text-muted-foreground hover:text-foreground"
                          title={t("mcp.edit")}
                        >
                          <Pencil className="h-3.5 w-3.5" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          onClick={() => setRemoving(server.name)}
                          className="h-7 w-7 text-muted-foreground hover:text-destructive hover:bg-destructive/10"
                          title={t("mcp.delete")}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </div>
                  </BentoCard>
                );
              })}
            </div>

            {/* Pagination if more than 12 servers */}
            {totalPages > 1 && (
              <Pagination className="mt-2">
                <PaginationContent>
                  <PaginationItem>
                    <PaginationPrevious
                      onClick={() => setCurrentPage((p) => Math.max(1, p - 1))}
                      className={cn(currentPage <= 1 && "pointer-events-none opacity-50")}
                    />
                  </PaginationItem>
                  {paginationRange.map((page, i) =>
                    page === "ellipsis" ? (
                      <PaginationItem key={`e${i}`}>
                        <PaginationEllipsis />
                      </PaginationItem>
                    ) : (
                      <PaginationItem key={page}>
                        <PaginationLink
                          isActive={page === currentPage}
                          onClick={() => setCurrentPage(page as number)}
                        >
                          {page}
                        </PaginationLink>
                      </PaginationItem>
                    )
                  )}
                  <PaginationItem>
                    <PaginationNext
                      onClick={() => setCurrentPage((p) => Math.min(totalPages, p + 1))}
                      className={cn(currentPage >= totalPages && "pointer-events-none opacity-50")}
                    />
                  </PaginationItem>
                </PaginationContent>
              </Pagination>
            )}
          </>
        )}

        {/* Remove confirm dialog */}
        <AlertDialog open={removing !== null} onOpenChange={(v) => !v && setRemoving(null)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("mcp.delete")}</AlertDialogTitle>
              <AlertDialogDescription>{t("mcp.confirmDelete")}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("mcp.cancel")}</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                onClick={() => removing && removeMutation.mutate(removing)}
              >
                {t("mcp.delete")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>

        <McpEditorDialog
          key={editing === "new" ? "__new__" : editing?.name ?? "__closed__"}
          open={editing !== null}
          server={editing === "new" ? undefined : editing ?? undefined}
          existingNames={servers.map((s) => s.name)}
          onClose={() => setEditing(null)}
        />
      </div>
    </TooltipProvider>
  );
}

function McpEditorDialog({
  open,
  server,
  existingNames,
  onClose,
}: {
  open: boolean;
  server?: McpServerSummary;
  existingNames: string[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<McpTransport>("stdio");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [url, setUrl] = useState("");
  const [envText, setEnvText] = useState("");
  const [headersText, setHeadersText] = useState("");

  useEffect(() => {
    if (open) {
      setName(server?.name ?? "");
      setTransport(server?.transport ?? "stdio");
      setCommand(server?.command ?? "");
      setArgs(server?.args.join("\n") ?? "");
      setUrl(server?.url ?? "");
      setEnvText(
        server?.environment ? Object.entries(server.environment).map(([k, v]) => `${k}=${v}`).join("\n") : ""
      );
      setHeadersText(
        server?.headers ? Object.entries(server.headers).map(([k, v]) => `${k}: ${v}`).join("\n") : ""
      );
    }
  }, [open, server]);

  const trimmedName = name.trim();
  // Saving a new server under an existing name would silently replace that server.
  const duplicateName = !server && existingNames.includes(trimmedName);

  const upsertMutation = useMutation({
    mutationFn: () => {
      const environment: Record<string, string> = {};
      envText.split("\n").filter(Boolean).forEach((line) => {
        const idx = line.indexOf("=");
        if (idx > 0) environment[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
      });
      const headers: Record<string, string> = {};
      headersText.split("\n").filter(Boolean).forEach((line) => {
        const idx = line.indexOf(":");
        if (idx > 0) headers[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
      });
      return api.upsertMcpServer({
        name: trimmedName,
        transport,
        enabled: server?.enabled ?? true,
        command: transport === "stdio" ? command : null,
        args: transport === "stdio" ? args.split("\n").map((s) => s.replace(/\r$/, "")).filter((s) => s.length > 0) : [],
        url: transport !== "stdio" ? url : null,
        headers,
        environment,
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["mcp-servers"] });
      onClose();
    },
  });

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader className="border-b border-border/40 pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary ring-1 ring-inset ring-primary/15">
              <Server className="h-4 w-4" />
            </div>
            <div>
              <DialogTitle className="text-base font-semibold">{server ? t("mcp.edit") : t("mcp.add")}</DialogTitle>
              <p className="mt-0.5 text-xs text-muted-foreground">{t("mcp.description")}</p>
            </div>
          </div>
        </DialogHeader>

        <div className="grid gap-4 sm:grid-cols-2 pt-1">
          {/* Left Column: Basic Service Configuration */}
          <div className="space-y-3.5">
            <InputField label={t("mcp.name")} value={name} onChange={setName} disabled={!!server} placeholder="e.g. filesystem" />
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-muted-foreground">{t("mcp.transport")}</label>
              <div className="flex gap-2">
                {(["stdio", "http", "sse"] as McpTransport[]).map((tr) => (
                  <Button
                    key={tr}
                    variant={transport === tr ? "soft" : "outline"}
                    size="sm"
                    className="flex-1"
                    onClick={() => setTransport(tr)}
                  >
                    {tr.toUpperCase()}
                  </Button>
                ))}
              </div>
            </div>
            {transport === "stdio" ? (
              <InputField label={t("mcp.command")} value={command} onChange={setCommand} mono placeholder="npx / uvx / python" />
            ) : (
              <InputField label={t("mcp.url")} value={url} onChange={setUrl} mono placeholder="https://..." />
            )}
          </div>

          {/* Right Column: Parameters, Headers & Environment */}
          <div className="space-y-3.5">
            {transport === "stdio" ? (
              <TextareaField label={t("baseline.argsPerLine")} value={args} onChange={setArgs} mono placeholder={"-y\n@modelcontextprotocol/server-filesystem"} />
            ) : (
              <TextareaField label={t("mcp.headers")} value={headersText} onChange={setHeadersText} placeholder="Authorization: Bearer sk-..." mono />
            )}
            <TextareaField label={t("mcp.env")} value={envText} onChange={setEnvText} placeholder="API_KEY=your_key" mono />
          </div>
        </div>

        <DialogFooter className="border-t border-border/40 pt-3">
          <Button variant="outline" size="sm" onClick={onClose}>
            {t("mcp.cancel")}
          </Button>
          {duplicateName && (
            <p role="alert" className="mr-auto self-center text-xs text-destructive">{t("mcp.duplicateName")}</p>
          )}
          <Button size="sm" onClick={() => upsertMutation.mutate()} disabled={!trimmedName || duplicateName || upsertMutation.isPending}>
            {t("mcp.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function InputField({
  label,
  value,
  onChange,
  disabled,
  mono,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  disabled?: boolean;
  mono?: boolean;
  placeholder?: string;
}) {
  return (
    <div className="space-y-1.5">
      <label className="text-xs font-medium text-muted-foreground">{label}</label>
      <Input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled}
        placeholder={placeholder}
        className={cn(
          "focus:ring-2 focus:ring-ring/20 focus:border-primary",
          mono && ""
        )}
      />
    </div>
  );
}

function TextareaField({
  label,
  value,
  onChange,
  placeholder,
  mono,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  mono?: boolean;
}) {
  return (
    <div className="space-y-1.5">
      <label className="text-xs font-medium text-muted-foreground">{label}</label>
      <Textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        rows={2}
        className={cn(mono && "")}
      />
    </div>
  );
}
